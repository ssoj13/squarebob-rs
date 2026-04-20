//! NTFS MFT scanner using FSCTL_ENUM_USN_DATA (Windows API).
//! Enumerates all MFT records via DeviceIoControl, builds tree from flat list.
//! Requires admin privileges for volume handle access.

#[cfg(windows)]
use std::path::{Path, PathBuf};
#[cfg(windows)]
use std::sync::Arc;
#[cfg(windows)]
use std::sync::atomic::{AtomicBool, Ordering};
#[cfg(windows)]
use crossbeam_channel::Sender;
#[cfg(windows)]
use log::{info, warn};
#[cfg(windows)]
use crate::scanner::ScanMsg;
use dirstat_core::DirEntry;

/// Check if NTFS scan is available for the given path
#[cfg(windows)]
pub fn is_ntfs_available(path: &Path) -> bool {
    let s = path.to_string_lossy();
    let drive_letter = match s.chars().next() {
        Some(c) if c.is_ascii_alphabetic() => c,
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

/// Enumerate MFT via FSCTL_ENUM_USN_DATA, build tree, fill sizes via std::fs
#[cfg(windows)]
fn scan_mft_usn(root: &Path, tx: &Sender<ScanMsg>, cancel: &AtomicBool) -> anyhow::Result<DirEntry> {
    use windows::Win32::Storage::FileSystem::*;
    use windows::Win32::System::IO::DeviceIoControl;
    use windows::Win32::System::Ioctl::FSCTL_ENUM_USN_DATA;
    use windows::Win32::Foundation::{GENERIC_READ, CloseHandle};
    use windows::core::HSTRING;

    let drive_str = root.to_string_lossy();
    let drive_letter = drive_str.chars().next()
        .ok_or_else(|| anyhow::anyhow!("Invalid path"))?;

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
    let mut counter: u64 = 0;

    info!("Enumerating MFT records...");

    loop {
        if cancel.load(Ordering::Relaxed) {
            unsafe { let _ = CloseHandle(handle); }
            return Err(anyhow::anyhow!("Scan cancelled"));
        }

        let mut returned: u32 = 0;
        let ok = unsafe {
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

        if ok.is_err() || returned <= 8 {
            break;
        }

        // First 8 bytes = next StartFileReferenceNumber
        let next_ref = u64::from_le_bytes(buffer[0..8].try_into().unwrap_or([0; 8]));

        // Parse USN_RECORD_V2 entries at offset 8
        let mut offset: usize = 8;
        while offset + 64 <= returned as usize {
            let record_len = u32::from_le_bytes(
                buffer[offset..offset+4].try_into().unwrap_or([0; 4])
            ) as usize;
            if record_len < 64 || offset + record_len > returned as usize { break; }

            let base = offset;
            let file_ref = u64::from_le_bytes(buffer[base+8..base+16].try_into().unwrap_or([0;8]));
            let parent_ref = u64::from_le_bytes(buffer[base+16..base+24].try_into().unwrap_or([0;8]));
            let attributes = u32::from_le_bytes(buffer[base+52..base+56].try_into().unwrap_or([0;4]));
            let name_len = u16::from_le_bytes(buffer[base+56..base+58].try_into().unwrap_or([0;2])) as usize;
            let name_off = u16::from_le_bytes(buffer[base+58..base+60].try_into().unwrap_or([0;2])) as usize;

            let ns = base + name_off;
            let ne = ns + name_len;
            if ne <= returned as usize && name_len > 0 {
                let wchars: Vec<u16> = buffer[ns..ne].chunks_exact(2)
                    .map(|c| u16::from_le_bytes([c[0], c[1]]))
                    .collect();
                let name = String::from_utf16_lossy(&wchars);
                let is_dir = (attributes & 0x10) != 0;
                if is_dir { dir_count += 1; } else { file_count += 1; }

                records.push(MftRecord {
                    file_ref: file_ref & 0x0000_FFFF_FFFF_FFFF,
                    parent_ref: parent_ref & 0x0000_FFFF_FFFF_FFFF,
                    name,
                    is_dir,
                });
            }

            offset += record_len;
            counter += 1;
            if counter.is_multiple_of(10000) {
                let _ = tx.send(ScanMsg::Progress { files: file_count, dirs: dir_count, bytes: 0, errors: 0 });
            }
        }

        enum_data[0..8].copy_from_slice(&next_ref.to_le_bytes());
    }

    unsafe { let _ = CloseHandle(handle); }
    info!("MFT enumeration done: {} records ({} files, {} dirs)", records.len(), file_count, dir_count);

    // Build tree scoped to target path
    build_tree_from_mft(root, &records, tx, cancel)
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
    let drive_letter = drive_str.chars().next().unwrap_or('C');
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
