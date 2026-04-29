//! NTFS MFT scanner using FSCTL_ENUM_USN_DATA (Windows API).
//! Enumerates all MFT records via DeviceIoControl, builds tree from flat list.
//! Requires admin privileges for volume handle access.

use std::path::Path;

#[cfg(windows)]
use std::path::PathBuf;
#[cfg(windows)]
use std::sync::Arc;
#[cfg(windows)]
use std::sync::atomic::{AtomicBool, Ordering};
#[cfg(windows)]
use crossbeam_channel::Sender;
#[cfg(windows)]
use log::{info, trace, warn};
#[cfg(windows)]
use crate::scanner::ScanMsg;
use dirstat_core::DirEntry;

/// Try opening `\\.\X:` the same way as MFT enumeration (often requires elevation).
#[cfg(windows)]
pub fn probe_raw_volume_access(path: &Path) -> anyhow::Result<()> {
    use windows::Win32::Storage::FileSystem::*;
    use windows::Win32::Foundation::{GENERIC_READ, CloseHandle};
    use windows::core::HSTRING;

    let drive_letter = path
        .to_string_lossy()
        .chars()
        .find(|c| c.is_ascii_alphabetic())
        .ok_or_else(|| {
            anyhow::anyhow!(
                "path must contain a drive letter (extended/UNC paths are not supported for raw volume)"
            )
        })?;

    let volume_path = format!("\\\\.\\{}:", drive_letter);
    let handle = unsafe {
        CreateFileW(
            &HSTRING::from(&volume_path),
            GENERIC_READ.0,
            FILE_SHARE_READ | FILE_SHARE_WRITE,
            None,
            OPEN_EXISTING,
            FILE_FLAG_BACKUP_SEMANTICS,
            None,
        )
    }
    .map_err(|e| anyhow::anyhow!("CreateFile {:?} (elevated admin may be required): {}", volume_path, e))?;
    unsafe {
        let _ = CloseHandle(handle);
    }
    Ok(())
}

#[cfg(not(windows))]
pub fn probe_raw_volume_access(_path: &Path) -> anyhow::Result<()> {
    anyhow::bail!("raw volume probe is only supported on Windows");
}

/// Check if NTFS scan is available for the given path
#[cfg(windows)]
pub fn is_ntfs_available(path: &Path) -> bool {
    let s = path.to_string_lossy();
    let drive_letter = match s.chars().find(|c| c.is_ascii_alphabetic()) {
        Some(c) => c,
        _ => return false,
    };

    use windows::Win32::Storage::FileSystem::GetVolumeInformationW;
    use windows::core::HSTRING;

    let root = format!("{}:\\", drive_letter);
    let root_w = HSTRING::from(&root);
    let mut fs_name = [0u16; 64];

    unsafe {
        let ok = GetVolumeInformationW(
            &root_w, None, None, None, None, Some(&mut fs_name),
        );
        if ok.is_ok() {
            let fs = String::from_utf16_lossy(&fs_name);
            return fs.trim_end_matches('\0') == "NTFS";
        }
    }
    false
}

/// Launch NTFS MFT scan in background thread
#[cfg(windows)]
pub fn scan_ntfs_bg(root: PathBuf, tx: Sender<ScanMsg>) -> Arc<AtomicBool> {
    let cancel = Arc::new(AtomicBool::new(false));
    let cancel_clone = cancel.clone();

    std::thread::Builder::new()
        .name("ntfs-scanner".into())
        .spawn(move || {
            info!("NTFS MFT scan started: {:?}", root);
            let _ = tx.send(ScanMsg::Progress { files: 0, dirs: 0, bytes: 0, errors: 0 });

            match scan_mft_usn(&root, &tx, &cancel_clone) {
                Ok(mut tree) => {
                    tree.sort_by_size();
                    info!("NTFS MFT scan done: {} files, {} dirs, {} bytes",
                        tree.file_count, tree.dir_count, tree.size);
                    let _ = tx.send(ScanMsg::Done(tree));
                }
                Err(e) => {
                    warn!("NTFS MFT scan failed: {e:#}, falling back to standard");
                    let _ = tx.send(ScanMsg::NtfsFallback(format!("{e:#}")));
                    let cancel2 = cancel_clone.clone();
                    match crate::scanner::scan_dir_public(&root, &tx, &cancel2) {
                        Ok(mut tree) => {
                            tree.sort_by_size();
                            let _ = tx.send(ScanMsg::Done(tree));
                        }
                        Err(e2) => {
                            let _ = tx.send(ScanMsg::Error(format!("Scan failed: {e2:#}")));
                        }
                    }
                }
            }
        })
        .expect("failed to spawn NTFS scanner thread");

    cancel
}

/// MFT record from USN enumeration
#[cfg(windows)]
struct MftRecord {
    file_ref: u64,
    parent_ref: u64,
    name: String,
    is_dir: bool,
}

/// NTFS-compatible 48-bit logical ID from `DWORDLONG`/first qword of `FILE_ID`.
#[cfg(windows)]
#[inline]
fn mask_frn(lo: u64) -> u64 {
    lo & 0x0000_FFFF_FFFF_FFFF
}

/// Leading qwords of NTFS FILE_ID-ish structs (compat with enumeration trees).
#[cfg(windows)]
#[inline]
fn id128_hi_qword(rec: &[u8], base: usize) -> Option<u64> {
    Some(mask_frn(u64::from_le_bytes(
        rec.get(base..base + 8)?.try_into().ok()?,
    )))
}

/// Interpret `MajorVersion`-specific field positions (see MSDN USN_RECORD_V2 / USN_RECORD_V3).
#[cfg(windows)]
fn parse_single_usn_record(rec: &[u8]) -> Option<MftRecord> {
    let record_len = u32::from_le_bytes(rec.get(0..4)?.try_into().ok()?) as usize;
    if record_len == 0 || rec.len() < record_len || record_len < 20 {
        return None;
    }
    let maj = u16::from_le_bytes(rec.get(4..6)?.try_into().ok()?);

    // V2: DWORDLONG FRN @8 — V3: FILE_ID_128 @8, @24 (MSDN USN_RECORD_V3).
    let (file_ref, parent_ref, attrs_off, name_len_off, name_off_member) = match maj {
        3 => (
            id128_hi_qword(rec, 8)?,
            id128_hi_qword(rec, 24)?,
            68usize,
            72usize,
            74usize,
        ),
        2 | 4 => (
            mask_frn(u64::from_le_bytes(rec.get(8..16)?.try_into().ok()?)),
            mask_frn(u64::from_le_bytes(rec.get(16..24)?.try_into().ok()?)),
            52usize,
            56usize,
            58usize,
        ),
        _ => (
            mask_frn(u64::from_le_bytes(rec.get(8..16)?.try_into().ok()?)),
            mask_frn(u64::from_le_bytes(rec.get(16..24)?.try_into().ok()?)),
            52usize,
            56usize,
            58usize,
        ),
    };

    let attributes = u32::from_le_bytes(rec.get(attrs_off..attrs_off + 4)?.try_into().ok()?);
    let name_len = u16::from_le_bytes(rec.get(name_len_off..name_len_off + 2)?.try_into().ok()?) as usize;
    let name_off = u16::from_le_bytes(rec.get(name_off_member..name_off_member + 2)?.try_into().ok()?) as usize;

    if name_off + name_len > record_len {
        return None;
    }

    let name_bytes = rec.get(name_off..name_off + name_len)?;
    if name_bytes.len() < 2 {
        return None;
    }

    let wchars: Vec<u16> = name_bytes
        .chunks_exact(2)
        .map(|c| u16::from_le_bytes([c[0], c[1]]))
        .collect();
    let name = String::from_utf16_lossy(&wchars);
    let is_dir = (attributes & 0x10) != 0;

    Some(MftRecord {
        file_ref,
        parent_ref,
        name,
        is_dir,
    })
}

/// Append decoded USN records from one `FSCTL_ENUM_USN_DATA` output buffer.
///
/// Writes into `records`. On success IOCTL, `buffer[.. returned]` begins with DWORDLONG continuation.
#[cfg(windows)]
fn accumulate_usn_buffer(
    buffer: &[u8],
    returned: usize,
    records: &mut Vec<MftRecord>,
    file_count: &mut u64,
    dir_count: &mut u64,
) -> usize {
    let take = returned.min(buffer.len());
    let buf = &buffer[..take];
    if buf.len() <= 8 {
        return 0;
    }

    let at_start = records.len();

    let mut offset = 8usize;
    while offset < buf.len() {
        let rl = buf.get(offset..offset + 4);
        let Some(rb) = rl.and_then(|b| TryInto::<[u8; 4]>::try_into(b).ok()) else {
            break;
        };
        let record_len_u = u32::from_le_bytes(rb);
        let record_len = record_len_u as usize;
        // MS sample uses >60; keep margin for variable names.
        if record_len < 60 || offset.checked_add(record_len).is_none() {
            break;
        }
        if offset + record_len > buf.len() {
            warn!("USN truncated at offset {}; record_len {}", offset, record_len);
            break;
        }
        let rec_slice = &buf[offset..offset + record_len];
        let maj = rec_slice
            .get(4..6)
            .map(|x| u16::from_le_bytes([x[0], x[1]]))
            .unwrap_or(0);

        match parse_single_usn_record(rec_slice) {
            Some(rec) => {
                if rec.is_dir {
                    *dir_count += 1;
                } else {
                    *file_count += 1;
                }
                records.push(rec);
            }
            None => trace!("skip USN at offset {}; len={} major={}", offset, record_len, maj),
        }

        offset += record_len;
    }

    records.len() - at_start
}

/// Enumerate MFT via FSCTL_ENUM_USN_DATA, build tree, fill sizes via std::fs
#[cfg(windows)]
fn scan_mft_usn(root: &Path, tx: &Sender<ScanMsg>, cancel: &AtomicBool) -> anyhow::Result<DirEntry> {
    use windows::Win32::Storage::FileSystem::*;
    use windows::Win32::System::IO::DeviceIoControl;
    use windows::Win32::System::Ioctl::FSCTL_ENUM_USN_DATA;
    use windows::Win32::Foundation::{GENERIC_READ, CloseHandle};
    use windows::core::HSTRING;

    let drive_str = root.to_string_lossy();
    let drive_letter = drive_str
        .chars()
        .find(|c| c.is_ascii_alphabetic())
        .ok_or_else(|| anyhow::anyhow!("path must start with a drive letter (e.g. C:\\)"))?;

    let volume_path = format!("\\\\.\\{}:", drive_letter);
    info!("Opening volume: {}", volume_path);

    let handle = unsafe {
        CreateFileW(
            &HSTRING::from(&volume_path),
            GENERIC_READ.0,
            FILE_SHARE_READ | FILE_SHARE_WRITE,
            None,
            OPEN_EXISTING,
            FILE_FLAG_BACKUP_SEMANTICS,
            None,
        )
    }.map_err(|e| anyhow::anyhow!("Cannot open volume {} (run as admin): {}", volume_path, e))?;
    info!("Volume opened OK");

    // HighUsn must be i64::MAX to enumerate ALL MFT records regardless of USN journal state
    // Using MaxUsn from journal fails when journal is empty/absent (MaxUsn=0 returns nothing)
    let high_usn: i64 = i64::MAX;

    // MFT_ENUM_DATA_V0: StartFileReferenceNumber(u64) + LowUsn(i64) + HighUsn(i64) = 24 bytes
    let mut enum_data = [0u8; 24];
    // LowUsn at [8..16] stays 0
    enum_data[16..24].copy_from_slice(&high_usn.to_le_bytes());

    let buf_size: usize = 64 * 1024;
    let mut buffer = vec![0u8; buf_size];
    let mut records: Vec<MftRecord> = Vec::with_capacity(500_000);
    let mut file_count: u64 = 0;
    let mut dir_count: u64 = 0;

    info!("Enumerating MFT records...");

    loop {
        if cancel.load(Ordering::Relaxed) {
            unsafe { let _ = CloseHandle(handle); }
            return Err(anyhow::anyhow!("Scan cancelled"));
        }

        let mut returned: u32 = 0;
        let ioctl_res = unsafe {
            DeviceIoControl(
                handle,
                FSCTL_ENUM_USN_DATA,
                Some(enum_data.as_ptr() as *const _),
                enum_data.len() as u32,
                Some(buffer.as_mut_ptr() as *mut _),
                buf_size as u32,
                Some(&mut returned),
                None,
            )
        };

        match ioctl_res {
            Ok(()) => {}
            Err(e) => {
                use windows::Win32::Foundation::ERROR_HANDLE_EOF;
                use windows::core::HRESULT;
                if e.code() == HRESULT::from_win32(ERROR_HANDLE_EOF.0) {
                    break;
                }
                unsafe {
                    let _ = CloseHandle(handle);
                }
                return Err(anyhow::anyhow!("FSCTL_ENUM_USN_DATA ioctl failed: {:?}", e));
            }
        }

        let ret_sz = returned as usize;
        if ret_sz < 8 {
            break;
        }

        let n_before = records.len();
        accumulate_usn_buffer(&buffer[..], ret_sz, &mut records, &mut file_count, &mut dir_count);

        let next_ref = u64::from_le_bytes(buffer[0..8].try_into().unwrap_or([0; 8]));
        enum_data[0..8].copy_from_slice(&next_ref.to_le_bytes());

        if records.len() / 10_000 > n_before / 10_000 {
            let _ = tx.send(ScanMsg::Progress { files: file_count, dirs: dir_count, bytes: 0, errors: 0 });
        }
    }

    unsafe { let _ = CloseHandle(handle); }
    info!("MFT enumeration done: {} records ({} files, {} dirs)", records.len(), file_count, dir_count);

    // Build tree scoped to target path
    build_tree_from_mft(root, &records, tx, cancel)
}

/// Debugging: histogram of `USN_RECORD::MajorVersion` and IOCTL buffer sizes (`FSCTL_ENUM_USN_DATA`).
/// Does not build a tree — `dirstat-rs test enum-diagnose [PATH] [MAX_IOCTL_LOOPS]` (often needs elevation).
#[cfg(windows)]
pub fn diagnose_fsctl_enum_usn(path: &Path, max_ioctl_loops: usize) -> anyhow::Result<String> {
    use std::collections::HashMap;
    use std::fmt::Write;

    use windows::Win32::Foundation::{CloseHandle, ERROR_HANDLE_EOF, GENERIC_READ};
    use windows::Win32::Storage::FileSystem::*;
    use windows::Win32::System::IO::DeviceIoControl;
    use windows::Win32::System::Ioctl::FSCTL_ENUM_USN_DATA;
    use windows::core::{HSTRING, HRESULT};

    let drive_str = path.to_string_lossy();
    let drive_letter = drive_str
        .chars()
        .find(|c| c.is_ascii_alphabetic())
        .ok_or_else(|| anyhow::anyhow!("path must contain a drive letter"))?;
    let volume_path = format!("\\\\.\\{}:", drive_letter);

    let handle = unsafe {
        CreateFileW(
            &HSTRING::from(&volume_path),
            GENERIC_READ.0,
            FILE_SHARE_READ | FILE_SHARE_WRITE,
            None,
            OPEN_EXISTING,
            FILE_FLAG_BACKUP_SEMANTICS,
            None,
        )
    }
    .map_err(|e| anyhow::anyhow!("CreateFile {:?}: {:?}", volume_path, e))?;

    let high_usn: i64 = i64::MAX;
    let mut enum_data = [0u8; 24];
    enum_data[16..24].copy_from_slice(&high_usn.to_le_bytes());

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
        volume_path, max_ioctl_loops.max(1)
    )?;

    loop {
        if ioctl_round >= max_ioctl_loops.max(1) {
            writeln!(&mut out, "stopped after {} ioctl rounds (limit)", ioctl_round)?;
            break;
        }
        ioctl_round += 1;

        let mut returned: u32 = 0;
        let ioctl_res = unsafe {
            DeviceIoControl(
                handle,
                FSCTL_ENUM_USN_DATA,
                Some(enum_data.as_ptr() as *const _),
                enum_data.len() as u32,
                Some(buffer.as_mut_ptr() as *mut _),
                buf_size as u32,
                Some(&mut returned),
                None,
            )
        };

        match ioctl_res {
            Ok(()) => {}
            Err(e) => {
                if e.code() == HRESULT::from_win32(ERROR_HANDLE_EOF.0) {
                    writeln!(&mut out, "ioctl round {}: EOF {:?}", ioctl_round, e)?;
                    break;
                }
                unsafe {
                    let _ = CloseHandle(handle);
                }
                return Err(anyhow::anyhow!("ioctl failed: {:?}", e));
            }
        }

        let ret_sz = returned as usize;
        writeln!(
            &mut out,
            "ioctl round {}: {} bytes out",
            ioctl_round,
            ret_sz,
        )?;
        if ret_sz < 8 {
            writeln!(&mut out, "(returned < 8, done)")?;
            break;
        }

        let buf = &buffer[..ret_sz];
        let next_ref = u64::from_le_bytes(buf[0..8].try_into().unwrap_or([0; 8]));
        writeln!(&mut out, "  next StartFileReferenceNumber: 0x{:016x}", next_ref)?;

        let mut offset = 8usize;
        while offset < buf.len() {
            let Some(rb) =
                buf.get(offset..offset + 4).and_then(|b| TryInto::<[u8; 4]>::try_into(b).ok())
            else {
                break;
            };
            let record_len = u32::from_le_bytes(rb) as usize;
            if record_len < 60 || offset + record_len > buf.len() {
                writeln!(
                    &mut out,
                    "  offset {}: record_len={}, buffer len {}",
                    offset,
                    record_len,
                    buf.len()
                )?;
                break;
            }
            let rec_slice = &buf[offset..offset + record_len];
            let maj = rec_slice
                .get(4..6)
                .map(|x| u16::from_le_bytes([x[0], x[1]]))
                .unwrap_or(0);
            *hist_major.entry(maj).or_insert(0) += 1;
            match parse_single_usn_record(rec_slice) {
                Some(_) => parsed_ok += 1,
                None => parse_fail += 1,
            }
            offset += record_len;
        }

        enum_data[0..8].copy_from_slice(&next_ref.to_le_bytes());
    }

    unsafe {
        let _ = CloseHandle(handle);
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
pub fn diagnose_fsctl_enum_usn(_path: &Path, _max_ioctl_loops: usize) -> anyhow::Result<String> {
    anyhow::bail!("diagnose_fsctl_enum_usn is Windows-only")
}

/// Перечислить записи MFT через `FSCTL_ENUM_USN_DATA`, вернуть первые `max_names` имён (как есть в журнале).
#[cfg(windows)]
pub fn mft_dump_names(path: &Path, max_names: usize) -> anyhow::Result<String> {
    use std::fmt::Write;

    use windows::Win32::Foundation::{CloseHandle, ERROR_HANDLE_EOF, GENERIC_READ};
    use windows::Win32::Storage::FileSystem::*;
    use windows::Win32::System::IO::DeviceIoControl;
    use windows::Win32::System::Ioctl::FSCTL_ENUM_USN_DATA;
    use windows::core::{HSTRING, HRESULT};

    let cap = max_names.clamp(1, 250_000);
    let drive_str = path.to_string_lossy();
    let drive_letter = drive_str
        .chars()
        .find(|c| c.is_ascii_alphabetic())
        .ok_or_else(|| anyhow::anyhow!("path must contain a drive letter"))?;
    let volume_path = format!("\\\\.\\{}:", drive_letter);

    let handle = unsafe {
        CreateFileW(
            &HSTRING::from(&volume_path),
            GENERIC_READ.0,
            FILE_SHARE_READ | FILE_SHARE_WRITE,
            None,
            OPEN_EXISTING,
            FILE_FLAG_BACKUP_SEMANTICS,
            None,
        )
    }
    .map_err(|e| anyhow::anyhow!("CreateFile {:?}: {:?}", volume_path, e))?;

    let high_usn: i64 = i64::MAX;
    let mut enum_data = [0u8; 24];
    enum_data[16..24].copy_from_slice(&high_usn.to_le_bytes());

    let buf_size = 64 * 1024usize;
    let mut buffer = vec![0u8; buf_size];
    let mut records: Vec<MftRecord> = Vec::with_capacity(cap.min(10_000));
    let mut file_count: u64 = 0;
    let mut dir_count: u64 = 0;

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
        let mut returned: u32 = 0;
        let ioctl_res = unsafe {
            DeviceIoControl(
                handle,
                FSCTL_ENUM_USN_DATA,
                Some(enum_data.as_ptr() as *const _),
                enum_data.len() as u32,
                Some(buffer.as_mut_ptr() as *mut _),
                buf_size as u32,
                Some(&mut returned),
                None,
            )
        };
        match ioctl_res {
            Ok(()) => {}
            Err(e) => {
                if e.code() == HRESULT::from_win32(ERROR_HANDLE_EOF.0) {
                    break;
                }
                unsafe {
                    let _ = CloseHandle(handle);
                }
                return Err(anyhow::anyhow!("FSCTL_ENUM_USN_DATA: {:?}", e));
            }
        }
        let ret_sz = returned as usize;
        if ret_sz < 8 {
            break;
        }
        accumulate_usn_buffer(&buffer[..], ret_sz, &mut records, &mut file_count, &mut dir_count);
        if records.len() > cap {
            records.truncate(cap);
            break;
        }
        let next_ref = u64::from_le_bytes(buffer[0..8].try_into().unwrap_or([0; 8]));
        enum_data[0..8].copy_from_slice(&next_ref.to_le_bytes());
    }

    unsafe {
        let _ = CloseHandle(handle);
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
pub fn mft_dump_names(_path: &Path, _max_names: usize) -> anyhow::Result<String> {
    anyhow::bail!("mft_dump_names is Windows-only")
}

/// Build DirEntry tree from flat MFT records, scoped to `root` path.
#[cfg(windows)]
fn build_tree_from_mft(
    root: &Path,
    records: &[MftRecord],
    tx: &Sender<ScanMsg>,
    cancel: &AtomicBool,
) -> anyhow::Result<DirEntry> {
    use std::collections::HashMap;

    let mut children_map: HashMap<u64, Vec<usize>> = HashMap::new();
    for (i, rec) in records.iter().enumerate() {
        children_map.entry(rec.parent_ref).or_default().push(i);
    }

    // NTFS root dir is always MFT entry 5
    let drive_str = root.to_string_lossy();
    let drive_letter = drive_str
        .chars()
        .find(|c| c.is_ascii_alphabetic())
        .unwrap_or('C');
    let drive_root = PathBuf::from(format!("{}:\\", drive_letter));

    // Navigate to target subdirectory
    let target_ref = if root == drive_root || root == PathBuf::from(format!("{}:", drive_letter)) {
        5u64
    } else {
        let rel = root.strip_prefix(&drive_root)
            .or_else(|_| root.strip_prefix(format!("{}:", drive_letter)))
            .unwrap_or(root.as_ref());
        let mut cur = 5u64;
        for component in rel.components() {
            let comp = component.as_os_str().to_string_lossy();
            let children = children_map.get(&cur).ok_or_else(|| {
                anyhow::anyhow!("Dir not found in MFT: {}", comp)
            })?;
            let mut found = false;
            for &ci in children {
                let r = &records[ci];
                if r.is_dir && r.name.eq_ignore_ascii_case(&comp) {
                    cur = r.file_ref;
                    found = true;
                    break;
                }
            }
            if !found { return Err(anyhow::anyhow!("Dir not found in MFT: {}", comp)); }
        }
        cur
    };

    info!("Target dir MFT ref: {}", target_ref);

    let root_name = root.file_name()
        .map(|s| s.to_string_lossy().to_string())
        .unwrap_or_else(|| root.to_string_lossy().to_string());

    let mut tree = build_subtree(target_ref, &root_name, root, true, records, &children_map, 0, cancel);

    // Fill sizes via std::fs::metadata (fast on NTFS, reads from MFT cache)
    info!("Fetching file sizes...");
    let mut fc: u64 = 0;
    let mut dc: u64 = 0;
    fill_sizes(&mut tree, &mut fc, &mut dc, tx, cancel);
    tree.file_count = fc;
    tree.dir_count = dc;
    Ok(tree)
}

#[cfg(windows)]
#[allow(clippy::too_many_arguments)]
fn build_subtree(
    file_ref: u64, name: &str, path: &std::path::Path, is_dir: bool,
    records: &[MftRecord],
    children_map: &std::collections::HashMap<u64, Vec<usize>>,
    depth: u32, cancel: &AtomicBool,
) -> DirEntry {
    if !is_dir {
        let ext = path.extension().map(|e| e.to_string_lossy().to_lowercase()).unwrap_or_default();
        return DirEntry::new_file(name.to_string(), path.to_path_buf(), 0, ext, None);
    }
    if depth > 256 { return DirEntry::new_dir(name.to_string(), path.to_path_buf()); }

    let mut entry = DirEntry::new_dir(name.to_string(), path.to_path_buf());
    if let Some(kids) = children_map.get(&file_ref) {
        for &ci in kids {
            if cancel.load(Ordering::Relaxed) { break; }
            let rec = &records[ci];
            if rec.name == "." || rec.name == ".." || rec.name.is_empty() { continue; }
            if rec.file_ref < 24 && rec.parent_ref == 5 { continue; }
            // Skip system directories
            if rec.is_dir && rec.parent_ref == 5 && is_system_dir(&rec.name) { continue; }

            let cp = path.join(&rec.name);
            entry.children.push(build_subtree(rec.file_ref, &rec.name, &cp, rec.is_dir, records, children_map, depth+1, cancel));
        }
    }
    entry
}

#[cfg(windows)]
fn fill_sizes(entry: &mut DirEntry, fc: &mut u64, dc: &mut u64, tx: &Sender<ScanMsg>, cancel: &AtomicBool) {
    if !entry.is_dir {
        if let Ok(m) = std::fs::metadata(&entry.path) {
            entry.size = m.len();
            entry.own_size = m.len();
            entry.modified_time = m.modified().ok()
                .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
                .map(|d| d.as_secs());
        }
        *fc += 1;
        if (*fc).is_multiple_of(5000) {
            let _ = tx.send(ScanMsg::Progress { files: *fc, dirs: *dc, bytes: 0, errors: 0 });
            if cancel.load(Ordering::Relaxed) { return; }
        }
        return;
    }
    *dc += 1;
    for child in &mut entry.children {
        fill_sizes(child, fc, dc, tx, cancel);
    }
    entry.size = entry.children.iter().map(|c| c.size).sum();
    entry.file_count = entry.children.iter().map(|c| if c.is_dir { c.file_count } else { 1 }).sum();
    entry.dir_count = entry.children.iter().filter(|c| c.is_dir).map(|c| c.dir_count + 1).sum();
}

/// System/protected directories to skip at volume root
#[cfg(windows)]
fn is_system_dir(name: &str) -> bool {
    matches!(name.to_lowercase().as_str(),
        "system volume information" | "$recycle.bin" | "$windows.~bt" |
        "$windows.~ws" | "recovery" | "$sysreset" | "$winreagent"
    )
}

// Non-Windows stubs (unused but needed for cross-platform compilation)
#[cfg(not(windows))]
#[allow(dead_code)]
pub fn is_ntfs_available(_path: &std::path::Path) -> bool { false }

#[cfg(not(windows))]
#[allow(dead_code)]
pub fn scan_ntfs_bg(_root: std::path::PathBuf, tx: crossbeam_channel::Sender<crate::scanner::ScanMsg>) -> std::sync::Arc<std::sync::atomic::AtomicBool> {
    let _ = tx.send(crate::scanner::ScanMsg::Error("NTFS not available".into()));
    std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false))
}
