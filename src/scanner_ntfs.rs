//! NTFS MFT scanner using FSCTL_ENUM_USN_DATA (Windows API).
//! Enumerates all MFT records via DeviceIoControl, builds tree from flat list.
//! Requires admin privileges for volume handle access.

use std::path::Path;

#[cfg(windows)]
use crate::path_key::ScanRoot;
#[cfg(windows)]
use crate::scanner::{
    ScanBuild, ScanDiagnostics, ScanFailure, ScanMsg, ScanOutcome, ScanPhase, ScanProgressUpdate,
    finish_build, send_progress_update,
};
#[cfg(windows)]
use crossbeam_channel::Sender;
#[cfg(windows)]
use log::{info, trace, warn};
#[cfg(windows)]
use squarebob_core::DirEntry;
#[cfg(windows)]
use std::path::PathBuf;
#[cfg(windows)]
use std::sync::Arc;
#[cfg(windows)]
use std::sync::atomic::{AtomicBool, Ordering};

#[cfg(windows)]
#[derive(Debug, Clone, Copy)]
struct VolumeTarget {
    letter: u8,
}

#[cfg(windows)]
impl VolumeTarget {
    fn parse(path: &Path) -> anyhow::Result<Self> {
        use std::path::{Component, Prefix};
        let Some(Component::Prefix(prefix)) = path.components().next() else {
            anyhow::bail!("NTFS raw scan requires an absolute drive path: {:?}", path);
        };
        let letter = match prefix.kind() {
            Prefix::Disk(letter) | Prefix::VerbatimDisk(letter) => letter.to_ascii_uppercase(),
            Prefix::UNC(..) | Prefix::VerbatimUNC(..) => {
                anyhow::bail!("UNC paths are not supported by raw NTFS scanning")
            }
            _ => anyhow::bail!("unsupported Windows volume prefix: {:?}", prefix.kind()),
        };
        Ok(Self { letter })
    }

    fn letter(self) -> char {
        char::from(self.letter)
    }

    fn volume_path(self) -> String {
        format!("\\\\.\\{}:", self.letter())
    }

    fn drive_root(self) -> PathBuf {
        PathBuf::from(format!("{}:\\", self.letter()))
    }

    fn relative_components(self, path: &Path) -> anyhow::Result<Vec<std::ffi::OsString>> {
        let actual = Self::parse(path)?;
        if actual.letter != self.letter {
            anyhow::bail!("path changed volume while building NTFS tree");
        }
        Ok(path
            .components()
            .skip(2)
            .map(|component| component.as_os_str().to_owned())
            .collect())
    }
}

#[cfg(windows)]
struct OwnedFileHandle(windows::Win32::Foundation::HANDLE);

#[cfg(windows)]
impl OwnedFileHandle {
    fn open_volume(volume_path: &str) -> windows::core::Result<Self> {
        use windows::Win32::Foundation::GENERIC_READ;
        use windows::Win32::Storage::FileSystem::{FILE_SHARE_READ, FILE_SHARE_WRITE};
        use windows::core::HSTRING;

        Self::open(
            &HSTRING::from(volume_path),
            GENERIC_READ.0,
            FILE_SHARE_READ | FILE_SHARE_WRITE,
            windows::Win32::Storage::FileSystem::FILE_FLAGS_AND_ATTRIBUTES(0),
        )
    }

    fn open_metadata(path: &Path) -> windows::core::Result<Self> {
        use windows::Win32::Storage::FileSystem::{
            FILE_FLAG_BACKUP_SEMANTICS, FILE_SHARE_DELETE, FILE_SHARE_READ, FILE_SHARE_WRITE,
        };
        use windows::core::HSTRING;

        Self::open(
            &HSTRING::from(path.as_os_str()),
            0,
            FILE_SHARE_READ | FILE_SHARE_WRITE | FILE_SHARE_DELETE,
            FILE_FLAG_BACKUP_SEMANTICS,
        )
    }

    fn open(
        path: &windows::core::HSTRING,
        desired_access: u32,
        share_mode: windows::Win32::Storage::FileSystem::FILE_SHARE_MODE,
        flags: windows::Win32::Storage::FileSystem::FILE_FLAGS_AND_ATTRIBUTES,
    ) -> windows::core::Result<Self> {
        use windows::Win32::Storage::FileSystem::{CreateFileW, OPEN_EXISTING};

        // SAFETY: HSTRING owns the NUL-terminated path for the call. Flags are
        // valid Win32 constants. The successful handle becomes uniquely owned
        // by this RAII wrapper and is closed exactly once in Drop.
        unsafe {
            CreateFileW(
                path,
                desired_access,
                share_mode,
                None,
                OPEN_EXISTING,
                flags,
                None,
            )
        }
        .map(Self)
    }

    fn raw(&self) -> windows::Win32::Foundation::HANDLE {
        self.0
    }
}

#[cfg(windows)]
impl Drop for OwnedFileHandle {
    fn drop(&mut self) {
        // SAFETY: self.0 is a successful CreateFileW result uniquely owned by
        // this wrapper. Drop runs once; no use occurs after close.
        unsafe {
            let _ = windows::Win32::Foundation::CloseHandle(self.0);
        }
    }
}

#[cfg(windows)]
fn query_ntfs_record_number(path: &Path) -> anyhow::Result<u64> {
    use windows::Win32::Storage::FileSystem::{
        FILE_ID_INFO, FileIdInfo, GetFileInformationByHandleEx,
    };

    let handle = OwnedFileHandle::open_metadata(path)
        .map_err(|error| anyhow::anyhow!("cannot open scan root metadata {:?}: {error}", path))?;
    let mut info = FILE_ID_INFO::default();
    let info_size = u32::try_from(std::mem::size_of::<FILE_ID_INFO>())
        .map_err(|_| anyhow::anyhow!("FILE_ID_INFO size exceeds u32"))?;
    // SAFETY: info is a live, writable FILE_ID_INFO. The exact struct size is
    // passed to Windows, and handle remains valid for the duration of the call.
    unsafe {
        GetFileInformationByHandleEx(
            handle.raw(),
            FileIdInfo,
            std::ptr::from_mut(&mut info).cast(),
            info_size,
        )
    }
    .map_err(|error| anyhow::anyhow!("cannot query NTFS file ID for {:?}: {error}", path))?;

    let identifier = info.FileId.Identifier;
    let high = u64::from_le_bytes(
        identifier[8..16]
            .try_into()
            .map_err(|_| anyhow::anyhow!("invalid FILE_ID_128 layout"))?,
    );
    if high != 0 {
        anyhow::bail!("scan root uses a 128-bit file ID unsupported by NTFS MFT enumeration");
    }
    let low = u64::from_le_bytes(
        identifier[..8]
            .try_into()
            .map_err(|_| anyhow::anyhow!("invalid FILE_ID_128 layout"))?,
    );
    Ok(mask_frn(low))
}

#[cfg(windows)]
#[repr(C)]
#[derive(Debug, Clone, Copy)]
struct MftEnumDataV0 {
    start_file_reference_number: u64,
    low_usn: i64,
    high_usn: i64,
}

#[cfg(windows)]
impl MftEnumDataV0 {
    fn all_records() -> Self {
        Self {
            start_file_reference_number: 0,
            low_usn: 0,
            high_usn: i64::MAX,
        }
    }
}

#[cfg(windows)]
const _: [(); 24] = [(); std::mem::size_of::<MftEnumDataV0>()];

#[cfg(windows)]
#[derive(Debug)]
enum EnumUsnError {
    BufferTooLarge(usize),
    ReturnedTooLarge { returned: usize, capacity: usize },
    ResponseTooShort(usize),
    NonAdvancingContinuation { current: u64, next: u64 },
    Windows(windows::core::Error),
}

#[cfg(windows)]
impl EnumUsnError {
    fn is_eof(&self) -> bool {
        use windows::Win32::Foundation::ERROR_HANDLE_EOF;
        use windows::core::HRESULT;

        matches!(
            self,
            Self::Windows(error)
                if error.code() == HRESULT::from_win32(ERROR_HANDLE_EOF.0)
        )
    }
}

#[cfg(windows)]
impl std::fmt::Display for EnumUsnError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::BufferTooLarge(size) => {
                write!(formatter, "USN output buffer exceeds u32: {size} bytes")
            }
            Self::ReturnedTooLarge { returned, capacity } => write!(
                formatter,
                "USN ioctl returned {returned} bytes for {capacity}-byte buffer"
            ),
            Self::ResponseTooShort(returned) => {
                write!(formatter, "USN ioctl returned only {returned} bytes")
            }
            Self::NonAdvancingContinuation { current, next } => write!(
                formatter,
                "USN continuation did not advance: current={current:#x}, next={next:#x}"
            ),
            Self::Windows(error) => write!(formatter, "{error}"),
        }
    }
}

#[cfg(windows)]
impl std::error::Error for EnumUsnError {}

#[cfg(windows)]
fn enumerate_usn(
    handle: &OwnedFileHandle,
    enum_data: &MftEnumDataV0,
    buffer: &mut [u8],
) -> Result<usize, EnumUsnError> {
    use windows::Win32::System::IO::DeviceIoControl;
    use windows::Win32::System::Ioctl::FSCTL_ENUM_USN_DATA;

    let input_size = u32::try_from(std::mem::size_of::<MftEnumDataV0>())
        .map_err(|_| EnumUsnError::BufferTooLarge(std::mem::size_of::<MftEnumDataV0>()))?;
    let output_size =
        u32::try_from(buffer.len()).map_err(|_| EnumUsnError::BufferTooLarge(buffer.len()))?;
    let mut returned = 0u32;
    // SAFETY: both slices remain alive and exclusively borrowed for the call.
    // Their lengths are converted exactly to u32. The kernel-reported byte count
    // is validated against buffer.len() before any caller receives it.
    unsafe {
        DeviceIoControl(
            handle.raw(),
            FSCTL_ENUM_USN_DATA,
            Some(std::ptr::from_ref(enum_data).cast()),
            input_size,
            Some(buffer.as_mut_ptr().cast()),
            output_size,
            Some(&mut returned),
            None,
        )
    }
    .map_err(EnumUsnError::Windows)?;

    let returned = usize::try_from(returned).map_err(|_| EnumUsnError::ReturnedTooLarge {
        returned: usize::MAX,
        capacity: buffer.len(),
    })?;
    if returned > buffer.len() {
        return Err(EnumUsnError::ReturnedTooLarge {
            returned,
            capacity: buffer.len(),
        });
    }
    Ok(returned)
}

#[cfg(windows)]
fn advance_enumeration(
    enum_data: &mut MftEnumDataV0,
    buffer: &[u8],
    returned: usize,
) -> Result<(), EnumUsnError> {
    if returned > buffer.len() {
        return Err(EnumUsnError::ReturnedTooLarge {
            returned,
            capacity: buffer.len(),
        });
    }
    let continuation = buffer
        .get(..returned)
        .and_then(|bytes| bytes.get(..8))
        .ok_or(EnumUsnError::ResponseTooShort(returned))?;
    let next = u64::from_le_bytes(
        continuation
            .try_into()
            .map_err(|_| EnumUsnError::ResponseTooShort(returned))?,
    );
    if next <= enum_data.start_file_reference_number {
        return Err(EnumUsnError::NonAdvancingContinuation {
            current: enum_data.start_file_reference_number,
            next,
        });
    }
    enum_data.start_file_reference_number = next;
    Ok(())
}

/// Try opening a raw volume the same way as MFT enumeration (often requires elevation).
#[cfg(windows)]
pub fn probe_raw_volume_access(path: &Path) -> anyhow::Result<()> {
    let volume_path = VolumeTarget::parse(path)?.volume_path();
    OwnedFileHandle::open_volume(&volume_path).map_err(|error| {
        anyhow::anyhow!(
            "CreateFile {:?} (elevated admin may be required): {}",
            volume_path,
            error
        )
    })?;
    Ok(())
}

#[cfg(not(windows))]
#[allow(dead_code)] // API-parity stub; real impl lives in cfg(windows) above.
pub fn probe_raw_volume_access(_path: &Path) -> anyhow::Result<()> {
    anyhow::bail!("raw volume probe is only supported on Windows");
}

/// Check if NTFS scan is available for the given path
#[cfg(windows)]
pub fn is_ntfs_available(path: &Path) -> bool {
    let Ok(target) = VolumeTarget::parse(path) else {
        return false;
    };

    use windows::Win32::Storage::FileSystem::GetVolumeInformationW;
    use windows::core::HSTRING;

    let root = target.drive_root();
    let root_w = HSTRING::from(root.as_os_str());
    let mut fs_name = [0u16; 64];

    // SAFETY: HSTRING owns the wide-string root path; `fs_name` is a stack-
    // allocated [u16; 64] borrowed mutably for the duration of the FFI call.
    // Other out-params are None — Windows tolerates that for unwanted fields.
    unsafe {
        let ok = GetVolumeInformationW(&root_w, None, None, None, None, Some(&mut fs_name));
        if ok.is_ok() {
            let fs = String::from_utf16_lossy(&fs_name);
            return fs.trim_end_matches('\0') == "NTFS";
        }
    }
    false
}

/// NTFS backend body. Thread/session ownership lives in scanner::spawn.
#[cfg(windows)]
pub(crate) fn run_ntfs(
    root: ScanRoot,
    tx: Sender<ScanMsg>,
    terminal_tx: Sender<ScanMsg>,
    cancel: Arc<AtomicBool>,
) {
    info!("NTFS MFT scan started: {:?}", root.path());
    let outcome = match scan_mft_usn(root.path(), &tx, &cancel) {
        Ok(build) => finish_build(&root, build),
        Err(ScanFailure::Cancelled) => ScanOutcome::Cancelled,
        Err(ScanFailure::BackendUnavailable(error)) => {
            warn!("NTFS backend unavailable: {error:#}, falling back to standard");
            match tx.try_send(ScanMsg::NtfsFallback(format!("{error:#}"))) {
                Ok(()) | Err(crossbeam_channel::TrySendError::Full(_)) => {}
                Err(crossbeam_channel::TrySendError::Disconnected(_)) => return,
            }
            match crate::scanner::scan_dir_public(root.path(), &tx, &cancel) {
                Ok(build) => finish_build(&root, build),
                Err(ScanFailure::Cancelled) => ScanOutcome::Cancelled,
                Err(ScanFailure::BackendUnavailable(fallback) | ScanFailure::Failed(fallback)) => {
                    ScanOutcome::Failed(format!("standard fallback failed: {fallback:#}"))
                }
            }
        }
        Err(ScanFailure::Failed(error)) => {
            ScanOutcome::Failed(format!("NTFS scan failed: {error:#}"))
        }
    };
    let _ = terminal_tx.send(ScanMsg::Terminal(outcome));
}

/// MFT record from USN enumeration.
#[cfg(windows)]
#[derive(Debug, PartialEq, Eq)]
struct MftRecord {
    file_ref: u64,
    parent_ref: u64,
    name: String,
    is_dir: bool,
}

#[cfg(windows)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum UsnRecordParseError {
    TruncatedHeader,
    InvalidRecordLength { declared: usize, available: usize },
    UnsupportedVersion(u16),
    ExtendedNtfsFileId,
    InvalidNameRange,
    OddNameLength,
    NameTooLarge,
    NameAllocation,
}

#[cfg(windows)]
impl std::fmt::Display for UsnRecordParseError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::TruncatedHeader => write!(formatter, "truncated USN record header"),
            Self::InvalidRecordLength {
                declared,
                available,
            } => write!(
                formatter,
                "invalid USN record length {declared} for {available}-byte slice"
            ),
            Self::UnsupportedVersion(version) => {
                write!(formatter, "unsupported USN record major version {version}")
            }
            Self::ExtendedNtfsFileId => {
                write!(
                    formatter,
                    "NTFS USN record contains a non-zero extended file ID"
                )
            }
            Self::InvalidNameRange => write!(formatter, "invalid USN filename range"),
            Self::OddNameLength => write!(formatter, "odd USN UTF-16 filename length"),
            Self::NameTooLarge => write!(formatter, "USN filename size overflow"),
            Self::NameAllocation => write!(formatter, "USN filename allocation failed"),
        }
    }
}

/// NTFS MFT record number. Upper 16 bits in a 64-bit file reference are the
/// reuse sequence number; hierarchy identity in this snapshot uses the low 48.
#[cfg(windows)]
#[inline]
fn mask_frn(reference: u64) -> u64 {
    reference & 0x0000_FFFF_FFFF_FFFF
}

#[cfg(windows)]
fn read_u16(bytes: &[u8], offset: usize) -> Result<u16, UsnRecordParseError> {
    let value = bytes
        .get(offset..offset + 2)
        .ok_or(UsnRecordParseError::TruncatedHeader)?;
    Ok(u16::from_le_bytes(
        value
            .try_into()
            .map_err(|_| UsnRecordParseError::TruncatedHeader)?,
    ))
}

#[cfg(windows)]
fn read_u32(bytes: &[u8], offset: usize) -> Result<u32, UsnRecordParseError> {
    let value = bytes
        .get(offset..offset + 4)
        .ok_or(UsnRecordParseError::TruncatedHeader)?;
    Ok(u32::from_le_bytes(
        value
            .try_into()
            .map_err(|_| UsnRecordParseError::TruncatedHeader)?,
    ))
}

#[cfg(windows)]
fn read_u64(bytes: &[u8], offset: usize) -> Result<u64, UsnRecordParseError> {
    let value = bytes
        .get(offset..offset + 8)
        .ok_or(UsnRecordParseError::TruncatedHeader)?;
    Ok(u64::from_le_bytes(
        value
            .try_into()
            .map_err(|_| UsnRecordParseError::TruncatedHeader)?,
    ))
}

#[cfg(windows)]
fn ntfs_record_number_from_file_id_128(
    bytes: &[u8],
    offset: usize,
) -> Result<u64, UsnRecordParseError> {
    let low = read_u64(bytes, offset)?;
    let high = read_u64(bytes, offset + 8)?;
    if high != 0 {
        return Err(UsnRecordParseError::ExtendedNtfsFileId);
    }
    Ok(mask_frn(low))
}

#[cfg(windows)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct UsnRecordHeader {
    file_ref: u64,
    parent_ref: u64,
    is_dir: bool,
    name_start: usize,
    name_end: usize,
}

/// Interpret only layouts documented as filename-bearing enumeration records.
/// V4 has a different extent layout and no filename; unknown majors must never
/// be guessed as V2.
#[cfg(windows)]
fn parse_usn_record_header(rec: &[u8]) -> Result<UsnRecordHeader, UsnRecordParseError> {
    let record_len = usize::try_from(read_u32(rec, 0)?).map_err(|_| {
        UsnRecordParseError::InvalidRecordLength {
            declared: usize::MAX,
            available: rec.len(),
        }
    })?;
    let major = read_u16(rec, 4)?;

    let (minimum_len, file_ref, parent_ref, attributes_offset, name_len_offset, name_offset) =
        match major {
            2 => (
                60usize,
                mask_frn(read_u64(rec, 8)?),
                mask_frn(read_u64(rec, 16)?),
                52usize,
                56usize,
                58usize,
            ),
            3 => (
                76usize,
                ntfs_record_number_from_file_id_128(rec, 8)?,
                ntfs_record_number_from_file_id_128(rec, 24)?,
                68usize,
                72usize,
                74usize,
            ),
            version => return Err(UsnRecordParseError::UnsupportedVersion(version)),
        };

    if record_len < minimum_len || record_len > rec.len() {
        return Err(UsnRecordParseError::InvalidRecordLength {
            declared: record_len,
            available: rec.len(),
        });
    }

    let attributes = read_u32(rec, attributes_offset)?;
    let file_name_len = usize::from(read_u16(rec, name_len_offset)?);
    let file_name_offset = usize::from(read_u16(rec, name_offset)?);
    if file_name_len % 2 != 0 {
        return Err(UsnRecordParseError::OddNameLength);
    }
    let file_name_end = file_name_offset
        .checked_add(file_name_len)
        .ok_or(UsnRecordParseError::InvalidNameRange)?;
    if file_name_offset < minimum_len || file_name_end > record_len {
        return Err(UsnRecordParseError::InvalidNameRange);
    }

    Ok(UsnRecordHeader {
        file_ref,
        parent_ref,
        is_dir: (attributes & 0x10) != 0,
        name_start: file_name_offset,
        name_end: file_name_end,
    })
}

#[cfg(windows)]
fn decode_usn_name(rec: &[u8], header: UsnRecordHeader) -> Result<String, UsnRecordParseError> {
    let name_bytes = rec
        .get(header.name_start..header.name_end)
        .ok_or(UsnRecordParseError::InvalidNameRange)?;
    let name_units = name_bytes
        .chunks_exact(2)
        .map(|pair| u16::from_le_bytes([pair[0], pair[1]]));
    let maximum_utf8_len = name_bytes
        .len()
        .checked_mul(2)
        .ok_or(UsnRecordParseError::NameTooLarge)?;
    let mut name = String::new();
    name.try_reserve(maximum_utf8_len)
        .map_err(|_| UsnRecordParseError::NameAllocation)?;
    for unit in char::decode_utf16(name_units) {
        name.push(unit.unwrap_or(char::REPLACEMENT_CHARACTER));
    }
    Ok(name)
}

#[cfg(windows)]
fn decode_mft_record(
    rec: &[u8],
    header: UsnRecordHeader,
) -> Result<MftRecord, UsnRecordParseError> {
    Ok(MftRecord {
        file_ref: header.file_ref,
        parent_ref: header.parent_ref,
        name: decode_usn_name(rec, header)?,
        is_dir: header.is_dir,
    })
}

#[cfg(windows)]
fn parse_single_usn_record(rec: &[u8]) -> Result<MftRecord, UsnRecordParseError> {
    let header = parse_usn_record_header(rec)?;
    decode_mft_record(rec, header)
}

#[cfg(windows)]
#[derive(Debug)]
enum DecodeUsnError {
    Incompatible(UsnRecordParseError),
    Fatal(anyhow::Error),
}

#[cfg(windows)]
impl std::fmt::Display for DecodeUsnError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Incompatible(error) => write!(formatter, "{error}"),
            Self::Fatal(error) => write!(formatter, "{error:#}"),
        }
    }
}

#[cfg(windows)]
impl std::error::Error for DecodeUsnError {}

#[cfg(windows)]
fn parse_error(error: UsnRecordParseError) -> DecodeUsnError {
    match error {
        error @ UsnRecordParseError::UnsupportedVersion(_)
        | error @ UsnRecordParseError::ExtendedNtfsFileId => DecodeUsnError::Incompatible(error),
        error @ UsnRecordParseError::NameTooLarge | error @ UsnRecordParseError::NameAllocation => {
            DecodeUsnError::Fatal(anyhow::anyhow!(error))
        }
        error => DecodeUsnError::Fatal(anyhow::anyhow!(error)),
    }
}

/// Visit validated filename-bearing records from one `FSCTL_ENUM_USN_DATA`
/// output buffer. The first DWORDLONG is the continuation value.
#[cfg(windows)]
fn visit_usn_buffer(
    buffer: &[u8],
    returned: usize,
    diagnostics: &mut ScanDiagnostics,
    visitor: &mut impl FnMut(&[u8], UsnRecordHeader) -> Result<(), DecodeUsnError>,
) -> Result<usize, DecodeUsnError> {
    if returned > buffer.len() {
        return Err(DecodeUsnError::Fatal(anyhow::anyhow!(
            "USN decoder received {returned} bytes for {}-byte buffer",
            buffer.len()
        )));
    }
    let buf = &buffer[..returned];
    if buf.len() <= 8 {
        return Ok(0);
    }

    let mut visited = 0usize;
    let mut offset = 8usize;
    while offset < buf.len() {
        let Some(length_bytes) = buf.get(offset..offset.saturating_add(4)) else {
            diagnostics.malformed_records = diagnostics.malformed_records.saturating_add(1);
            break;
        };
        let record_len = usize::try_from(u32::from_le_bytes(
            length_bytes
                .try_into()
                .map_err(|_| DecodeUsnError::Fatal(anyhow::anyhow!("invalid USN length field")))?,
        ))
        .map_err(|_| DecodeUsnError::Fatal(anyhow::anyhow!("USN record length exceeds usize")))?;
        let Some(end) = offset.checked_add(record_len) else {
            diagnostics.malformed_records = diagnostics.malformed_records.saturating_add(1);
            break;
        };
        if record_len < 60 || end > buf.len() {
            warn!("USN truncated at offset {offset}; record_len {record_len}");
            diagnostics.malformed_records = diagnostics.malformed_records.saturating_add(1);
            break;
        }
        let rec_slice = &buf[offset..end];
        let major = read_u16(rec_slice, 4).unwrap_or(0);

        match parse_usn_record_header(rec_slice) {
            Ok(header) => {
                visitor(rec_slice, header)?;
                visited = visited.checked_add(1).ok_or_else(|| {
                    DecodeUsnError::Fatal(anyhow::anyhow!("USN buffer record count overflow"))
                })?;
            }
            Err(error @ UsnRecordParseError::UnsupportedVersion(_))
            | Err(error @ UsnRecordParseError::ExtendedNtfsFileId) => {
                return Err(parse_error(error));
            }
            Err(error) => {
                diagnostics.malformed_records = diagnostics.malformed_records.saturating_add(1);
                trace!("skip USN at offset {offset}; len={record_len} major={major}: {error}");
            }
        }
        offset = end;
    }
    Ok(visited)
}

/// Append every decoded record from one output buffer.
#[cfg(windows)]
fn accumulate_usn_buffer(
    buffer: &[u8],
    returned: usize,
    records: &mut Vec<MftRecord>,
    file_count: &mut u64,
    dir_count: &mut u64,
    diagnostics: &mut ScanDiagnostics,
) -> Result<usize, DecodeUsnError> {
    let at_start = records.len();
    visit_usn_buffer(buffer, returned, diagnostics, &mut |rec_slice, header| {
        let record = decode_mft_record(rec_slice, header).map_err(parse_error)?;
        records.try_reserve(1).map_err(|error| {
            DecodeUsnError::Fatal(anyhow::anyhow!(
                "MFT record allocation failed while decoding: {error}"
            ))
        })?;
        if record.is_dir {
            *dir_count = (*dir_count).checked_add(1).ok_or_else(|| {
                DecodeUsnError::Fatal(anyhow::anyhow!("MFT directory count overflow"))
            })?;
        } else {
            *file_count = (*file_count)
                .checked_add(1)
                .ok_or_else(|| DecodeUsnError::Fatal(anyhow::anyhow!("MFT file count overflow")))?;
        }
        records.push(record);
        Ok(())
    })?;
    Ok(records.len() - at_start)
}

#[cfg(windows)]
#[derive(Debug, Clone, Copy)]
struct DirectoryEdge {
    file_ref: u64,
    parent_ref: u64,
}

#[cfg(windows)]
#[derive(Debug, Default)]
struct MftPassStats {
    visited: u64,
    files: u64,
    dirs: u64,
}

#[cfg(windows)]
fn decode_error_to_scan(error: DecodeUsnError) -> ScanFailure {
    match error {
        DecodeUsnError::Incompatible(error) => {
            ScanFailure::BackendUnavailable(anyhow::anyhow!(error))
        }
        DecodeUsnError::Fatal(error) => ScanFailure::Failed(error),
    }
}

#[cfg(windows)]
fn run_mft_pass(
    handle: &OwnedFileHandle,
    buffer: &mut [u8],
    tx: &Sender<ScanMsg>,
    cancel: &AtomicBool,
    diagnostics: &mut ScanDiagnostics,
    phase: ScanPhase,
    visitor: &mut impl FnMut(&[u8], UsnRecordHeader) -> Result<bool, DecodeUsnError>,
) -> Result<MftPassStats, ScanFailure> {
    let mut enum_data = MftEnumDataV0::all_records();
    let mut stats = MftPassStats::default();

    loop {
        if cancel.load(Ordering::Acquire) {
            return Err(ScanFailure::Cancelled);
        }
        let returned = match enumerate_usn(handle, &enum_data, buffer) {
            Ok(returned) => returned,
            Err(error) if error.is_eof() => break,
            Err(error) => {
                return Err(ScanFailure::BackendUnavailable(anyhow::anyhow!(
                    "FSCTL_ENUM_USN_DATA ioctl failed: {error}"
                )));
            }
        };
        let before = stats.visited;
        visit_usn_buffer(
            buffer,
            returned,
            diagnostics,
            &mut |record_bytes, header| {
                stats.visited = stats.visited.checked_add(1).ok_or_else(|| {
                    DecodeUsnError::Fatal(anyhow::anyhow!("MFT visited count overflow"))
                })?;
                if visitor(record_bytes, header)? {
                    if header.is_dir {
                        stats.dirs = stats.dirs.checked_add(1).ok_or_else(|| {
                            DecodeUsnError::Fatal(anyhow::anyhow!("MFT directory count overflow"))
                        })?;
                    } else {
                        stats.files = stats.files.checked_add(1).ok_or_else(|| {
                            DecodeUsnError::Fatal(anyhow::anyhow!("MFT file count overflow"))
                        })?;
                    }
                }
                Ok(())
            },
        )
        .map_err(decode_error_to_scan)?;
        advance_enumeration(&mut enum_data, buffer, returned).map_err(|error| {
            ScanFailure::BackendUnavailable(anyhow::anyhow!(
                "invalid FSCTL_ENUM_USN_DATA continuation: {error}"
            ))
        })?;

        if stats.visited / 10_000 > before / 10_000 {
            send_progress_update(
                tx,
                cancel,
                ScanProgressUpdate {
                    phase,
                    items: stats.visited,
                    files: stats.files,
                    dirs: stats.dirs,
                    bytes: 0,
                    errors: diagnostics.total_errors(),
                },
            )?;
        }
    }

    if stats.visited == 0 {
        return Err(ScanFailure::BackendUnavailable(anyhow::anyhow!(
            "FSCTL_ENUM_USN_DATA completed without filename-bearing records"
        )));
    }
    Ok(stats)
}

#[cfg(windows)]
fn selected_directory_refs(
    mut edges: Vec<DirectoryEdge>,
    target_ref: u64,
) -> Result<Vec<u64>, ScanFailure> {
    use std::collections::HashSet;

    if !edges.iter().any(|edge| edge.file_ref == target_ref) {
        return Err(ScanFailure::BackendUnavailable(anyhow::anyhow!(
            "selected directory MFT record {target_ref:#x} was not enumerated"
        )));
    }
    edges.sort_unstable_by_key(|edge| (edge.parent_ref, edge.file_ref));

    let mut discovered = HashSet::new();
    discovered.try_reserve(1).map_err(|error| {
        ScanFailure::Failed(anyhow::anyhow!("MFT selection allocation failed: {error}"))
    })?;
    discovered.insert(target_ref);

    let mut selected = Vec::new();
    selected.try_reserve(1).map_err(|error| {
        ScanFailure::Failed(anyhow::anyhow!(
            "MFT selection queue allocation failed: {error}"
        ))
    })?;
    selected.push(target_ref);

    let mut cursor = 0usize;
    while cursor < selected.len() {
        let parent_ref = selected[cursor];
        cursor = cursor
            .checked_add(1)
            .ok_or_else(|| ScanFailure::Failed(anyhow::anyhow!("MFT selection cursor overflow")))?;
        let start = edges.partition_point(|edge| edge.parent_ref < parent_ref);
        let end = edges.partition_point(|edge| edge.parent_ref <= parent_ref);
        for edge in &edges[start..end] {
            if discovered.contains(&edge.file_ref) {
                continue;
            }
            discovered.try_reserve(1).map_err(|error| {
                ScanFailure::Failed(anyhow::anyhow!("MFT selection allocation failed: {error}"))
            })?;
            selected.try_reserve(1).map_err(|error| {
                ScanFailure::Failed(anyhow::anyhow!(
                    "MFT selection queue allocation failed: {error}"
                ))
            })?;
            discovered.insert(edge.file_ref);
            selected.push(edge.file_ref);
        }
    }

    selected.sort_unstable();
    Ok(selected)
}

/// Enumerate MFT via FSCTL_ENUM_USN_DATA, build tree, fill sizes via std::fs.
#[cfg(windows)]
fn scan_mft_usn(
    root: &Path,
    tx: &Sender<ScanMsg>,
    cancel: &AtomicBool,
) -> Result<ScanBuild, ScanFailure> {
    let target = VolumeTarget::parse(root).map_err(ScanFailure::BackendUnavailable)?;
    let volume_path = target.volume_path();
    let target_ref = query_ntfs_record_number(root).map_err(ScanFailure::BackendUnavailable)?;
    let is_volume_root = target
        .relative_components(root)
        .map_err(ScanFailure::BackendUnavailable)?
        .is_empty();
    info!("Opening volume: {volume_path}; target MFT ref: {target_ref:#x}");

    let handle = OwnedFileHandle::open_volume(&volume_path).map_err(|error| {
        ScanFailure::BackendUnavailable(anyhow::anyhow!(
            "cannot open volume {volume_path} for raw NTFS access: {error}"
        ))
    })?;
    let buffer_size = 64 * 1024usize;
    let mut buffer = Vec::new();
    buffer.try_reserve_exact(buffer_size).map_err(|error| {
        ScanFailure::Failed(anyhow::anyhow!("MFT I/O buffer allocation failed: {error}"))
    })?;
    buffer.resize(buffer_size, 0);

    let selected_dirs = if is_volume_root {
        None
    } else {
        info!("Indexing MFT directory graph...");
        let mut edges = Vec::new();
        let mut index_diagnostics = ScanDiagnostics::default();
        let stats = run_mft_pass(
            &handle,
            &mut buffer,
            tx,
            cancel,
            &mut index_diagnostics,
            ScanPhase::IndexingVolume,
            &mut |_, header| {
                if !header.is_dir {
                    return Ok(false);
                }
                edges.try_reserve(1).map_err(|error| {
                    DecodeUsnError::Fatal(anyhow::anyhow!(
                        "MFT directory edge allocation failed: {error}"
                    ))
                })?;
                edges.push(DirectoryEdge {
                    file_ref: header.file_ref,
                    parent_ref: header.parent_ref,
                });
                Ok(true)
            },
        )?;
        info!(
            "MFT directory graph: {} records visited, {} directories",
            stats.visited, stats.dirs
        );
        Some(selected_directory_refs(edges, target_ref)?)
    };

    let mut records = Vec::new();
    records.try_reserve(16_384).map_err(|error| {
        ScanFailure::Failed(anyhow::anyhow!("MFT subtree allocation failed: {error}"))
    })?;
    let mut diagnostics = ScanDiagnostics::default();
    info!("Reading selected MFT tree...");
    let stats = run_mft_pass(
        &handle,
        &mut buffer,
        tx,
        cancel,
        &mut diagnostics,
        ScanPhase::SelectingTree,
        &mut |record_bytes, header| {
            let selected = selected_dirs
                .as_ref()
                .is_none_or(|dirs| dirs.binary_search(&header.parent_ref).is_ok());
            if !selected {
                return Ok(false);
            }
            let record = decode_mft_record(record_bytes, header).map_err(parse_error)?;
            records.try_reserve(1).map_err(|error| {
                DecodeUsnError::Fatal(anyhow::anyhow!(
                    "MFT subtree record allocation failed: {error}"
                ))
            })?;
            records.push(record);
            Ok(true)
        },
    )?;
    info!(
        "Selected MFT tree: {} volume records visited, {} files, {} directories",
        stats.visited, stats.files, stats.dirs
    );

    build_tree_from_mft(root, target_ref, &records, tx, cancel, diagnostics)
}

/// Debugging: histogram of `USN_RECORD::MajorVersion` and IOCTL buffer sizes (`FSCTL_ENUM_USN_DATA`).
/// Does not build a tree — `squarebob-rs test enum-diagnose [PATH] [MAX_IOCTL_LOOPS]` (often needs elevation).
#[cfg(windows)]
pub fn diagnose_fsctl_enum_usn(path: &Path, max_ioctl_loops: usize) -> anyhow::Result<String> {
    use std::collections::HashMap;
    use std::fmt::Write;

    let volume_path = VolumeTarget::parse(path)?.volume_path();

    let handle = OwnedFileHandle::open_volume(&volume_path)
        .map_err(|error| anyhow::anyhow!("CreateFile {volume_path:?}: {error}"))?;

    let mut enum_data = MftEnumDataV0::all_records();

    let buf_size: usize = 64 * 1024;
    let mut buffer = vec![0u8; buf_size];
    let mut hist_major: HashMap<u16, u64> = HashMap::new();
    let mut parsed_ok: u64 = 0;
    let mut parse_fail: u64 = 0;
    let mut ioctl_round: usize = 0;

    let mut out = String::new();
    writeln!(
        &mut out,
        "volume: {:?}, max_ioctl_rounds={}",
        volume_path,
        max_ioctl_loops.max(1)
    )?;

    loop {
        if ioctl_round >= max_ioctl_loops.max(1) {
            writeln!(
                &mut out,
                "stopped after {} ioctl rounds (limit)",
                ioctl_round
            )?;
            break;
        }
        ioctl_round += 1;

        let ret_sz = match enumerate_usn(&handle, &enum_data, &mut buffer) {
            Ok(returned) => returned,
            Err(error) if error.is_eof() => {
                writeln!(&mut out, "ioctl round {ioctl_round}: EOF {error}")?;
                break;
            }
            Err(error) => return Err(anyhow::anyhow!("ioctl failed: {error}")),
        };
        writeln!(
            &mut out,
            "ioctl round {}: {} bytes out",
            ioctl_round, ret_sz,
        )?;
        advance_enumeration(&mut enum_data, &buffer, ret_sz)
            .map_err(|error| anyhow::anyhow!("invalid continuation: {error}"))?;
        let buf = &buffer[..ret_sz];
        let next_ref = enum_data.start_file_reference_number;
        writeln!(
            &mut out,
            "  next StartFileReferenceNumber: 0x{:016x}",
            next_ref
        )?;

        let mut offset = 8usize;
        while offset < buf.len() {
            let Some(rb) = buf
                .get(offset..offset + 4)
                .and_then(|b| TryInto::<[u8; 4]>::try_into(b).ok())
            else {
                break;
            };
            let record_len = usize::try_from(u32::from_le_bytes(rb))
                .map_err(|_| anyhow::anyhow!("USN diagnostic record length exceeds usize"))?;
            let Some(end) = offset.checked_add(record_len) else {
                writeln!(&mut out, "  offset {offset}: record length overflow")?;
                break;
            };
            if record_len < 60 || end > buf.len() {
                writeln!(
                    &mut out,
                    "  offset {}: record_len={}, buffer len {}",
                    offset,
                    record_len,
                    buf.len()
                )?;
                break;
            }
            let rec_slice = &buf[offset..end];
            let maj = rec_slice
                .get(4..6)
                .map(|x| u16::from_le_bytes([x[0], x[1]]))
                .unwrap_or(0);
            let count = hist_major.entry(maj).or_insert(0);
            *count = count
                .checked_add(1)
                .ok_or_else(|| anyhow::anyhow!("USN major-version count overflow"))?;
            match parse_single_usn_record(rec_slice) {
                Ok(_) => {
                    parsed_ok = parsed_ok
                        .checked_add(1)
                        .ok_or_else(|| anyhow::anyhow!("USN parsed count overflow"))?;
                }
                Err(_) => {
                    parse_fail = parse_fail
                        .checked_add(1)
                        .ok_or_else(|| anyhow::anyhow!("USN failed count overflow"))?;
                }
            }
            offset = end;
        }
    }

    writeln!(&mut out, "---")?;
    writeln!(&mut out, "histogram MajorVersion → count: {:?}", hist_major)?;
    writeln!(
        &mut out,
        "parse_single_usn_record: ok={}, failed={}",
        parsed_ok, parse_fail,
    )?;
    writeln!(
        &mut out,
        "Interpretation: major 2 = USN_RECORD_V2; major 3 = V3(FILE_ID_128 FRN); both are handled in parse_single_usn_record.",
    )?;

    Ok(out)
}

#[cfg(not(windows))]
#[allow(dead_code)] // API-parity stub; real impl lives in cfg(windows) above.
pub fn diagnose_fsctl_enum_usn(_path: &Path, _max_ioctl_loops: usize) -> anyhow::Result<String> {
    anyhow::bail!("diagnose_fsctl_enum_usn is Windows-only")
}

/// Перечислить записи MFT через `FSCTL_ENUM_USN_DATA`, вернуть первые `max_names` имён (как есть в журнале).
#[cfg(windows)]
pub fn mft_dump_names(path: &Path, max_names: usize) -> anyhow::Result<String> {
    use std::fmt::Write;

    let cap = max_names.clamp(1, 250_000);
    let volume_path = VolumeTarget::parse(path)?.volume_path();

    let handle = OwnedFileHandle::open_volume(&volume_path)
        .map_err(|error| anyhow::anyhow!("CreateFile {volume_path:?}: {error}"))?;

    let mut enum_data = MftEnumDataV0::all_records();

    let buf_size = 64 * 1024usize;
    let mut buffer = vec![0u8; buf_size];
    let mut records: Vec<MftRecord> = Vec::with_capacity(cap.min(10_000));
    let mut file_count: u64 = 0;
    let mut dir_count: u64 = 0;
    let mut diagnostics = ScanDiagnostics::default();

    let mut out = String::new();
    writeln!(
        &mut out,
        "MFT (FSCTL_ENUM_USN_DATA) first {} names on {:?}",
        cap, volume_path
    )?;

    loop {
        if records.len() >= cap {
            break;
        }
        let ret_sz = match enumerate_usn(&handle, &enum_data, &mut buffer) {
            Ok(returned) => returned,
            Err(error) if error.is_eof() => break,
            Err(error) => return Err(anyhow::anyhow!("FSCTL_ENUM_USN_DATA: {error}")),
        };
        accumulate_usn_buffer(
            &buffer,
            ret_sz,
            &mut records,
            &mut file_count,
            &mut dir_count,
            &mut diagnostics,
        )?;
        if records.len() > cap {
            records.truncate(cap);
            break;
        }
        advance_enumeration(&mut enum_data, &buffer, ret_sz)
            .map_err(|error| anyhow::anyhow!("invalid continuation: {error}"))?;
    }

    for rec in &records {
        let tag = if rec.is_dir { "DIR " } else { "FILE" };
        writeln!(&mut out, "{tag} {}", rec.name)?;
    }
    writeln!(
        &mut out,
        "--- shown {} names (files+dirs in enum order; not full paths)",
        records.len()
    )?;
    Ok(out)
}

#[cfg(not(windows))]
#[allow(dead_code)] // API-parity stub; real impl lives in cfg(windows) above.
pub fn mft_dump_names(_path: &Path, _max_names: usize) -> anyhow::Result<String> {
    anyhow::bail!("mft_dump_names is Windows-only")
}

/// Build DirEntry tree from flat MFT records, scoped to `root` path.
#[cfg(windows)]
fn build_tree_from_mft(
    root: &Path,
    target_ref: u64,
    records: &[MftRecord],
    tx: &Sender<ScanMsg>,
    cancel: &AtomicBool,
    mut diagnostics: ScanDiagnostics,
) -> Result<ScanBuild, ScanFailure> {
    use std::collections::HashMap;

    if cancel.load(Ordering::Acquire) {
        return Err(ScanFailure::Cancelled);
    }

    let mut child_counts: HashMap<u64, usize> = HashMap::new();
    child_counts.try_reserve(records.len()).map_err(|error| {
        ScanFailure::Failed(anyhow::anyhow!(
            "MFT parent index allocation failed: {error}"
        ))
    })?;
    for record in records {
        let count = child_counts.entry(record.parent_ref).or_default();
        *count = count
            .checked_add(1)
            .ok_or_else(|| ScanFailure::Failed(anyhow::anyhow!("MFT child count overflow")))?;
    }

    let mut children_map: HashMap<u64, Vec<usize>> = HashMap::new();
    children_map
        .try_reserve(child_counts.len())
        .map_err(|error| {
            ScanFailure::Failed(anyhow::anyhow!(
                "MFT child index allocation failed: {error}"
            ))
        })?;
    for (parent, count) in child_counts {
        let mut children = Vec::new();
        children.try_reserve_exact(count).map_err(|error| {
            ScanFailure::Failed(anyhow::anyhow!("MFT child list allocation failed: {error}"))
        })?;
        children_map.insert(parent, children);
    }
    for (index, record) in records.iter().enumerate() {
        children_map
            .get_mut(&record.parent_ref)
            .ok_or_else(|| {
                ScanFailure::Failed(anyhow::anyhow!("MFT parent index invariant failed"))
            })?
            .push(index);
    }

    info!("Target dir MFT ref: {target_ref}");
    let root_name = root
        .file_name()
        .map(|s| s.to_string_lossy().to_string())
        .unwrap_or_else(|| root.to_string_lossy().to_string());
    let mut tree = build_subtree(
        target_ref,
        &root_name,
        root,
        records,
        &children_map,
        cancel,
        &mut diagnostics,
    )?;

    info!("Fetching file sizes...");
    let mut progress = NtfsProgress::default();
    let _ = fill_sizes(&mut tree, true, &mut progress, tx, cancel, &mut diagnostics)?;
    send_ntfs_tree_progress(tx, cancel, &progress, &diagnostics)?;

    Ok(ScanBuild { tree, diagnostics })
}

#[cfg(windows)]
#[derive(Default)]
struct NtfsProgress {
    files: u64,
    dirs: u64,
    bytes: u64,
}

#[cfg(windows)]
fn send_ntfs_tree_progress(
    tx: &Sender<ScanMsg>,
    cancel: &AtomicBool,
    progress: &NtfsProgress,
    diagnostics: &ScanDiagnostics,
) -> Result<(), ScanFailure> {
    send_progress_update(
        tx,
        cancel,
        ScanProgressUpdate {
            phase: ScanPhase::MeasuringTree,
            items: progress.files.saturating_add(progress.dirs),
            files: progress.files,
            dirs: progress.dirs,
            bytes: progress.bytes,
            errors: diagnostics.total_errors(),
        },
    )
}

#[cfg(windows)]
fn build_subtree(
    file_ref: u64,
    name: &str,
    path: &Path,
    records: &[MftRecord],
    children_map: &std::collections::HashMap<u64, Vec<usize>>,
    cancel: &AtomicBool,
    diagnostics: &mut ScanDiagnostics,
) -> Result<DirEntry, ScanFailure> {
    struct BuildFrame<'a> {
        file_ref: u64,
        entry: DirEntry,
        children: &'a [usize],
        next_child: usize,
    }

    if cancel.load(Ordering::Acquire) {
        return Err(ScanFailure::Cancelled);
    }

    let root_children = children_map
        .get(&file_ref)
        .map(Vec::as_slice)
        .unwrap_or_default();
    let mut root_entry = DirEntry::new_dir(name.to_string(), path.to_path_buf());
    root_entry
        .children
        .try_reserve_exact(root_children.len())
        .map_err(|error| {
            ScanFailure::Failed(anyhow::anyhow!(
                "MFT subtree allocation failed at {:?}: {error}",
                path
            ))
        })?;

    let mut ancestry = std::collections::HashSet::new();
    ancestry.try_reserve(1).map_err(|error| {
        ScanFailure::Failed(anyhow::anyhow!("MFT ancestry allocation failed: {error}"))
    })?;
    ancestry.insert(file_ref);

    let mut frames = Vec::new();
    frames.try_reserve(1).map_err(|error| {
        ScanFailure::Failed(anyhow::anyhow!("MFT traversal allocation failed: {error}"))
    })?;
    frames.push(BuildFrame {
        file_ref,
        entry: root_entry,
        children: root_children,
        next_child: 0,
    });

    loop {
        if cancel.load(Ordering::Acquire) {
            return Err(ScanFailure::Cancelled);
        }

        let next_record = {
            let frame = frames.last_mut().expect("MFT traversal always has a root");
            if frame.next_child < frame.children.len() {
                let index = frame.children[frame.next_child];
                frame.next_child += 1;
                Some(&records[index])
            } else {
                None
            }
        };

        if let Some(record) = next_record {
            if record.name == "." || record.name == ".." || record.name.is_empty() {
                continue;
            }
            if record.file_ref < 24 && record.parent_ref == 5 {
                continue;
            }
            if record.is_dir && record.parent_ref == 5 && is_system_dir(&record.name) {
                continue;
            }

            let parent_path = &frames
                .last()
                .expect("MFT child always has a parent")
                .entry
                .path;
            let child_path = parent_path.join(&record.name);
            if !record.is_dir {
                let extension = child_path
                    .extension()
                    .map(|extension| extension.to_string_lossy().to_lowercase())
                    .unwrap_or_default();
                frames
                    .last_mut()
                    .expect("MFT child always has a parent")
                    .entry
                    .children
                    .push(DirEntry::new_file(
                        record.name.clone(),
                        child_path,
                        0,
                        extension,
                        None,
                    ));
                continue;
            }

            if ancestry.contains(&record.file_ref) {
                diagnostics.depth_errors = diagnostics.depth_errors.saturating_add(1);
                frames
                    .last_mut()
                    .expect("MFT cycle placeholder always has a parent")
                    .entry
                    .children
                    .push(DirEntry::new_dir(record.name.clone(), child_path));
                continue;
            }

            let child_indices = children_map
                .get(&record.file_ref)
                .map(Vec::as_slice)
                .unwrap_or_default();
            let mut child_entry = DirEntry::new_dir(record.name.clone(), child_path);
            child_entry
                .children
                .try_reserve_exact(child_indices.len())
                .map_err(|error| {
                    ScanFailure::Failed(anyhow::anyhow!(
                        "MFT subtree allocation failed at {:?}: {error}",
                        child_entry.path
                    ))
                })?;
            frames.try_reserve(1).map_err(|error| {
                ScanFailure::Failed(anyhow::anyhow!("MFT traversal allocation failed: {error}"))
            })?;
            ancestry.try_reserve(1).map_err(|error| {
                ScanFailure::Failed(anyhow::anyhow!("MFT ancestry allocation failed: {error}"))
            })?;
            ancestry.insert(record.file_ref);
            frames.push(BuildFrame {
                file_ref: record.file_ref,
                entry: child_entry,
                children: child_indices,
                next_child: 0,
            });
            continue;
        }

        let completed = frames
            .pop()
            .expect("MFT traversal always completes an existing frame");
        ancestry.remove(&completed.file_ref);
        if let Some(parent) = frames.last_mut() {
            parent.entry.children.push(completed.entry);
        } else {
            return Ok(completed.entry);
        }
    }
}

#[cfg(windows)]
fn measure_ntfs_file(
    entry: &mut DirEntry,
    progress: &mut NtfsProgress,
    tx: &Sender<ScanMsg>,
    cancel: &AtomicBool,
    diagnostics: &mut ScanDiagnostics,
) -> Result<bool, ScanFailure> {
    if cancel.load(Ordering::Acquire) {
        return Err(ScanFailure::Cancelled);
    }

    let metadata = match std::fs::metadata(&entry.path) {
        Ok(metadata) => metadata,
        Err(error) => {
            diagnostics.metadata_errors = diagnostics.metadata_errors.saturating_add(1);
            trace!("metadata failed for {:?}: {error}", entry.path);
            return Ok(false);
        }
    };
    entry.size = metadata.len();
    entry.own_size = metadata.len();
    entry.modified_time = metadata
        .modified()
        .ok()
        .and_then(|time| time.duration_since(std::time::UNIX_EPOCH).ok())
        .map(|duration| duration.as_secs());
    progress.files = progress
        .files
        .checked_add(1)
        .ok_or_else(|| ScanFailure::Failed(anyhow::anyhow!("file count overflow")))?;
    progress.bytes = progress
        .bytes
        .checked_add(entry.size)
        .ok_or_else(|| ScanFailure::Failed(anyhow::anyhow!("byte count overflow")))?;
    if progress.files.is_multiple_of(5000) {
        send_ntfs_tree_progress(tx, cancel, progress, diagnostics)?;
    }
    Ok(true)
}

#[cfg(windows)]
fn fill_sizes(
    entry: &mut DirEntry,
    is_root: bool,
    progress: &mut NtfsProgress,
    tx: &Sender<ScanMsg>,
    cancel: &AtomicBool,
    diagnostics: &mut ScanDiagnostics,
) -> Result<bool, ScanFailure> {
    struct SizeFrame {
        entry: DirEntry,
        remaining: std::vec::IntoIter<DirEntry>,
        retained: Vec<DirEntry>,
    }

    fn frame_for(mut entry: DirEntry) -> Result<SizeFrame, ScanFailure> {
        let children = std::mem::take(&mut entry.children);
        let mut retained = Vec::new();
        retained
            .try_reserve_exact(children.len())
            .map_err(|error| {
                ScanFailure::Failed(anyhow::anyhow!(
                    "MFT size-pass allocation failed at {:?}: {error}",
                    entry.path
                ))
            })?;
        Ok(SizeFrame {
            entry,
            remaining: children.into_iter(),
            retained,
        })
    }

    if cancel.load(Ordering::Acquire) {
        return Err(ScanFailure::Cancelled);
    }

    let placeholder = DirEntry::new_dir(entry.name.clone(), entry.path.clone());
    let mut root = std::mem::replace(entry, placeholder);
    if !root.is_dir {
        let keep = measure_ntfs_file(&mut root, progress, tx, cancel, diagnostics)?;
        *entry = root;
        return Ok(keep);
    }
    if !is_root {
        progress.dirs = progress
            .dirs
            .checked_add(1)
            .ok_or_else(|| ScanFailure::Failed(anyhow::anyhow!("directory count overflow")))?;
    }

    let mut frames = Vec::new();
    frames.try_reserve(1).map_err(|error| {
        ScanFailure::Failed(anyhow::anyhow!(
            "MFT size traversal allocation failed: {error}"
        ))
    })?;
    frames.push(frame_for(root)?);

    loop {
        if cancel.load(Ordering::Acquire) {
            return Err(ScanFailure::Cancelled);
        }

        let next_child = frames
            .last_mut()
            .expect("MFT size traversal always has a root")
            .remaining
            .next();
        if let Some(mut child) = next_child {
            if child.is_dir {
                progress.dirs = progress.dirs.checked_add(1).ok_or_else(|| {
                    ScanFailure::Failed(anyhow::anyhow!("directory count overflow"))
                })?;
                frames.try_reserve(1).map_err(|error| {
                    ScanFailure::Failed(anyhow::anyhow!(
                        "MFT size traversal allocation failed: {error}"
                    ))
                })?;
                frames.push(frame_for(child)?);
            } else if measure_ntfs_file(&mut child, progress, tx, cancel, diagnostics)? {
                frames
                    .last_mut()
                    .expect("measured file always has a parent")
                    .retained
                    .push(child);
            }
            continue;
        }

        let frame = frames
            .pop()
            .expect("MFT size traversal always completes an existing frame");
        let mut completed = frame.entry;
        completed.children = frame.retained;
        completed.size = completed.own_size;
        completed.file_count = 0;
        completed.dir_count = 0;
        for child in &completed.children {
            completed.size = completed.size.checked_add(child.size).ok_or_else(|| {
                ScanFailure::Failed(anyhow::anyhow!("size overflow at {:?}", completed.path))
            })?;
            completed.file_count = completed
                .file_count
                .checked_add(if child.is_dir { child.file_count } else { 1 })
                .ok_or_else(|| {
                    ScanFailure::Failed(anyhow::anyhow!(
                        "file count overflow at {:?}",
                        completed.path
                    ))
                })?;
            let child_dirs = if child.is_dir {
                child.dir_count.checked_add(1).ok_or_else(|| {
                    ScanFailure::Failed(anyhow::anyhow!(
                        "directory count overflow at {:?}",
                        completed.path
                    ))
                })?
            } else {
                0
            };
            completed.dir_count = completed.dir_count.checked_add(child_dirs).ok_or_else(|| {
                ScanFailure::Failed(anyhow::anyhow!(
                    "directory count overflow at {:?}",
                    completed.path
                ))
            })?;
        }

        if let Some(parent) = frames.last_mut() {
            parent.retained.push(completed);
        } else {
            *entry = completed;
            return Ok(true);
        }
    }
}

/// System/protected directories to skip at volume root
#[cfg(windows)]
fn is_system_dir(name: &str) -> bool {
    matches!(
        name.to_lowercase().as_str(),
        "system volume information"
            | "$recycle.bin"
            | "$windows.~bt"
            | "$windows.~ws"
            | "recovery"
            | "$sysreset"
            | "$winreagent"
    )
}

// Non-Windows stubs (unused but needed for cross-platform compilation)
#[cfg(not(windows))]
#[allow(dead_code)]
pub fn is_ntfs_available(_path: &std::path::Path) -> bool {
    false
}

#[cfg(all(test, windows))]
mod tests {
    use super::*;

    fn encoded_name(name: &str) -> Vec<u8> {
        name.encode_utf16()
            .flat_map(u16::to_le_bytes)
            .collect::<Vec<_>>()
    }

    fn v2_record(file_ref: u64, parent_ref: u64, name: &str, is_dir: bool) -> Vec<u8> {
        let name = encoded_name(name);
        let record_len = 60usize.checked_add(name.len()).expect("test record length");
        let mut record = vec![0u8; record_len];
        record[0..4].copy_from_slice(
            &u32::try_from(record_len)
                .expect("test record fits u32")
                .to_le_bytes(),
        );
        record[4..6].copy_from_slice(&2u16.to_le_bytes());
        record[8..16].copy_from_slice(&file_ref.to_le_bytes());
        record[16..24].copy_from_slice(&parent_ref.to_le_bytes());
        record[52..56].copy_from_slice(&(if is_dir { 0x10u32 } else { 0 }).to_le_bytes());
        record[56..58].copy_from_slice(
            &u16::try_from(name.len())
                .expect("test name fits u16")
                .to_le_bytes(),
        );
        record[58..60].copy_from_slice(&60u16.to_le_bytes());
        record[60..].copy_from_slice(&name);
        record
    }

    fn v3_record(file_ref: u64, parent_ref: u64, name: &str, is_dir: bool) -> Vec<u8> {
        let name = encoded_name(name);
        let record_len = 76usize.checked_add(name.len()).expect("test record length");
        let mut record = vec![0u8; record_len];
        record[0..4].copy_from_slice(
            &u32::try_from(record_len)
                .expect("test record fits u32")
                .to_le_bytes(),
        );
        record[4..6].copy_from_slice(&3u16.to_le_bytes());
        record[8..16].copy_from_slice(&file_ref.to_le_bytes());
        record[24..32].copy_from_slice(&parent_ref.to_le_bytes());
        record[68..72].copy_from_slice(&(if is_dir { 0x10u32 } else { 0 }).to_le_bytes());
        record[72..74].copy_from_slice(
            &u16::try_from(name.len())
                .expect("test name fits u16")
                .to_le_bytes(),
        );
        record[74..76].copy_from_slice(&76u16.to_le_bytes());
        record[76..].copy_from_slice(&name);
        record
    }

    #[test]
    fn mft_enum_data_v0_matches_windows_abi() {
        assert_eq!(std::mem::size_of::<MftEnumDataV0>(), 24);
        assert_eq!(std::mem::align_of::<MftEnumDataV0>(), 8);
        let data = MftEnumDataV0::all_records();
        assert_eq!(data.start_file_reference_number, 0);
        assert_eq!(data.low_usn, 0);
        assert_eq!(data.high_usn, i64::MAX);
    }

    #[test]
    fn parses_v2_and_masks_reuse_sequence() {
        let record = v2_record(
            0x1234_0000_0000_0042,
            0x5678_0000_0000_0005,
            "alpha.txt",
            false,
        );
        let parsed = parse_single_usn_record(&record).expect("valid V2 record");
        assert_eq!(parsed.file_ref, 0x42);
        assert_eq!(parsed.parent_ref, 5);
        assert_eq!(parsed.name, "alpha.txt");
        assert!(!parsed.is_dir);
    }

    #[test]
    fn parses_v3_ntfs_ids_from_low_qword() {
        let record = v3_record(
            0x1234_0000_0000_0042,
            0x5678_0000_0000_0005,
            "directory",
            true,
        );
        let parsed = parse_single_usn_record(&record).expect("valid V3 record");
        assert_eq!(parsed.file_ref, 0x42);
        assert_eq!(parsed.parent_ref, 5);
        assert_eq!(parsed.name, "directory");
        assert!(parsed.is_dir);
    }

    #[test]
    fn rejects_extended_v3_id_in_ntfs_backend() {
        let mut record = v3_record(0x42, 5, "entry", false);
        record[16..24].copy_from_slice(&1u64.to_le_bytes());
        assert_eq!(
            parse_single_usn_record(&record),
            Err(UsnRecordParseError::ExtendedNtfsFileId)
        );
    }

    #[test]
    fn rejects_v4_instead_of_guessing_v2_layout() {
        let mut record = v2_record(0x42, 5, "entry", false);
        record[4..6].copy_from_slice(&4u16.to_le_bytes());
        assert_eq!(
            parse_single_usn_record(&record),
            Err(UsnRecordParseError::UnsupportedVersion(4))
        );
    }

    #[test]
    fn accepts_empty_filename_without_misclassifying_buffer() {
        let record = v2_record(0x42, 5, "", false);
        let parsed = parse_single_usn_record(&record).expect("zero-length name is representable");
        assert!(parsed.name.is_empty());
    }

    #[test]
    fn continuation_must_advance() {
        let mut data = MftEnumDataV0 {
            start_file_reference_number: 10,
            ..MftEnumDataV0::all_records()
        };
        let next = 11u64.to_le_bytes();
        advance_enumeration(&mut data, &next, next.len()).expect("advancing continuation");
        assert_eq!(data.start_file_reference_number, 11);

        let same = 11u64.to_le_bytes();
        assert!(matches!(
            advance_enumeration(&mut data, &same, same.len()),
            Err(EnumUsnError::NonAdvancingContinuation {
                current: 11,
                next: 11
            })
        ));
        assert!(matches!(
            advance_enumeration(&mut data, &same[..7], 7),
            Err(EnumUsnError::ResponseTooShort(7))
        ));
    }

    #[test]
    fn buffer_decoder_counts_and_appends_v2_records() {
        let record = v2_record(0x42, 5, "entry", false);
        let mut buffer = Vec::with_capacity(8 + record.len());
        buffer.extend_from_slice(&43u64.to_le_bytes());
        buffer.extend_from_slice(&record);
        let mut records = Vec::new();
        let mut files = 0;
        let mut dirs = 0;
        let mut diagnostics = ScanDiagnostics::default();

        let appended = accumulate_usn_buffer(
            &buffer,
            buffer.len(),
            &mut records,
            &mut files,
            &mut dirs,
            &mut diagnostics,
        )
        .expect("valid buffer");

        assert_eq!(appended, 1);
        assert_eq!(records.len(), 1);
        assert_eq!(files, 1);
        assert_eq!(dirs, 0);
        assert_eq!(diagnostics.malformed_records, 0);
    }

    #[test]
    fn continuation_rejects_kernel_count_larger_than_buffer() {
        let mut data = MftEnumDataV0::all_records();
        let buffer = 11u64.to_le_bytes();
        assert!(matches!(
            advance_enumeration(&mut data, &buffer, buffer.len() + 1),
            Err(EnumUsnError::ReturnedTooLarge {
                returned: 9,
                capacity: 8
            })
        ));
    }

    #[test]
    fn directory_selection_follows_only_target_descendants() {
        let edges = vec![
            DirectoryEdge {
                file_ref: 10,
                parent_ref: 5,
            },
            DirectoryEdge {
                file_ref: 11,
                parent_ref: 10,
            },
            DirectoryEdge {
                file_ref: 12,
                parent_ref: 11,
            },
            DirectoryEdge {
                file_ref: 20,
                parent_ref: 5,
            },
            DirectoryEdge {
                file_ref: 21,
                parent_ref: 20,
            },
        ];
        let selected = match selected_directory_refs(edges, 10) {
            Ok(selected) => selected,
            Err(_) => panic!("valid directory graph"),
        };
        assert_eq!(selected, vec![10, 11, 12]);
    }

    #[test]
    fn directory_selection_rejects_missing_target() {
        let error = selected_directory_refs(
            vec![DirectoryEdge {
                file_ref: 11,
                parent_ref: 5,
            }],
            10,
        );
        assert!(matches!(error, Err(ScanFailure::BackendUnavailable(_))));
    }

    #[test]
    fn subtree_builder_preserves_deep_directory_chains() {
        const DEPTH: usize = 4_096;
        const ROOT_REF: u64 = 100;

        let mut records = Vec::with_capacity(DEPTH);
        let mut children_map = std::collections::HashMap::with_capacity(DEPTH);
        for offset in 0..DEPTH {
            let parent_ref = ROOT_REF + u64::try_from(offset).expect("depth fits u64");
            let file_ref = parent_ref + 1;
            records.push(MftRecord {
                file_ref,
                parent_ref,
                name: format!("d{offset}"),
                is_dir: true,
            });
            children_map.insert(parent_ref, vec![offset]);
        }

        let cancel = AtomicBool::new(false);
        let mut diagnostics = ScanDiagnostics::default();
        let tree = build_subtree(
            ROOT_REF,
            "root",
            Path::new(r"C:\root"),
            &records,
            &children_map,
            &cancel,
            &mut diagnostics,
        )
        .expect("deep subtree must build");

        assert_eq!(tree.iter().count(), DEPTH + 1);
        assert_eq!(diagnostics.depth_errors, 0);
    }

    #[test]
    fn subtree_builder_localizes_cycles() {
        const ROOT_REF: u64 = 100;
        let records = vec![
            MftRecord {
                file_ref: 101,
                parent_ref: ROOT_REF,
                name: "child".to_string(),
                is_dir: true,
            },
            MftRecord {
                file_ref: ROOT_REF,
                parent_ref: 101,
                name: "cycle".to_string(),
                is_dir: true,
            },
        ];
        let children_map = std::collections::HashMap::from([(ROOT_REF, vec![0]), (101, vec![1])]);

        let cancel = AtomicBool::new(false);
        let mut diagnostics = ScanDiagnostics::default();
        let tree = build_subtree(
            ROOT_REF,
            "root",
            Path::new(r"C:\root"),
            &records,
            &children_map,
            &cancel,
            &mut diagnostics,
        )
        .expect("cycle must be isolated");

        assert_eq!(tree.iter().count(), 3);
        assert_eq!(diagnostics.depth_errors, 1);
        assert!(tree.children[0].children[0].children.is_empty());
    }

    #[test]
    fn size_pass_handles_deep_directory_chains() {
        const DEPTH: usize = 4_096;

        let mut tree = DirEntry::new_dir(
            format!("d{DEPTH}"),
            PathBuf::from(format!(r"C:\root\d{DEPTH}")),
        );
        for offset in (0..DEPTH).rev() {
            let (name, path) = if offset == 0 {
                ("root".to_string(), PathBuf::from(r"C:\root"))
            } else {
                (
                    format!("d{offset}"),
                    PathBuf::from(format!(r"C:\root\d{offset}")),
                )
            };
            let mut parent = DirEntry::new_dir(name, path);
            parent.children.push(tree);
            tree = parent;
        }

        let (tx, _rx) = crossbeam_channel::bounded(1);
        let cancel = AtomicBool::new(false);
        let mut progress = NtfsProgress::default();
        let mut diagnostics = ScanDiagnostics::default();
        let keep = fill_sizes(
            &mut tree,
            true,
            &mut progress,
            &tx,
            &cancel,
            &mut diagnostics,
        )
        .expect("deep size pass must complete");

        assert!(keep);
        assert_eq!(tree.iter().count(), DEPTH + 1);
        assert_eq!(
            tree.dir_count,
            u64::try_from(DEPTH).expect("depth fits u64")
        );
        assert_eq!(progress.dirs, u64::try_from(DEPTH).expect("depth fits u64"));
        assert_eq!(diagnostics.total_errors(), 0);
    }
}
