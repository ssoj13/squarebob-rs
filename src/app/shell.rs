//! Cross-platform shell operations module
//!
//! Provides platform-specific integrations with native file managers, terminals,
//! and system operations like revealing files, opening properties dialogs, and
//! moving items to trash.

use std::path::Path;

// ── Cross-platform context menu labels ──

/// Get platform-specific label for "reveal in file manager" action
pub(super) fn reveal_label() -> &'static str {
    #[cfg(target_os = "macos")]
    { "Reveal in Finder" }
    #[cfg(target_os = "windows")]
    { "Show in Explorer" }
    #[cfg(target_os = "linux")]
    { "Show in File Manager" }
    #[cfg(not(any(target_os = "macos", target_os = "windows", target_os = "linux")))]
    { "Reveal in File Manager" }
}

/// Get platform-specific label for "properties/info" action
#[cfg(any(target_os = "windows", target_os = "macos"))]
pub(super) fn properties_label() -> &'static str {
    #[cfg(target_os = "macos")]
    { "Get Info" }
    #[cfg(target_os = "windows")]
    { "Properties" }
}

/// Get platform-specific label for "move to trash" action
pub(super) fn trash_label() -> &'static str {
    #[cfg(target_os = "macos")]
    { "Move to Trash" }
    #[cfg(target_os = "windows")]
    { "Move to Recycle Bin" }
    #[cfg(not(any(target_os = "macos", target_os = "windows")))]
    { "Move to Trash" }
}

// ── Cross-platform shell operations ──

/// Reveal file/folder in native file manager
pub(super) fn shell_reveal(path: &Path) {
    #[cfg(target_os = "macos")]
    {
        let _ = std::process::Command::new("open")
            .arg("-R")
            .arg(path)
            .spawn();
    }
    #[cfg(target_os = "windows")]
    {
        let _ = std::process::Command::new("explorer")
            .arg(format!("/select,{}", path.display()))
            .spawn();
    }
    #[cfg(target_os = "linux")]
    {
        // Try common file managers with reveal/select support
        let dir = path_to_dir(path);
        // dbus method for nautilus/nemo (GNOME/Cinnamon)
        let dbus_result = std::process::Command::new("dbus-send")
            .args([
                "--print-reply",
                "--dest=org.freedesktop.FileManager1",
                "/org/freedesktop/FileManager1",
                "org.freedesktop.FileManager1.ShowItems",
                &format!("array:string:file://{}", path.display()),
                "string:",
            ])
            .spawn();
        if dbus_result.is_err() {
            // Fallback: just open the containing folder
            let _ = open::that(dir);
        }
    }
    #[cfg(not(any(target_os = "macos", target_os = "windows", target_os = "linux")))]
    {
        let dir = path_to_dir(path);
        let _ = open::that(dir);
    }
}

/// Open terminal at file/folder location
pub(super) fn shell_open_terminal(path: &Path) {
    let dir = path_to_dir(path);
    #[cfg(target_os = "macos")]
    {
        let script = format!(
            "tell application \"Terminal\" to do script \"cd '{}'\"",
            dir.display().to_string().replace('\'', "'\\''")
        );
        let _ = std::process::Command::new("osascript")
            .arg("-e")
            .arg(&script)
            .spawn();
    }
    #[cfg(target_os = "windows")]
    {
        // Try Windows Terminal first, fall back to cmd
        let wt_result = std::process::Command::new("wt")
            .arg("-d")
            .arg(dir)
            .spawn();
        if wt_result.is_err() {
            let _ = std::process::Command::new("cmd")
                .arg("/k")
                .current_dir(dir)
                .spawn();
        }
    }
    #[cfg(target_os = "linux")]
    {
        // Try common terminal emulators
        let terminals = ["gnome-terminal", "konsole", "xfce4-terminal", "xterm"];
        for term in terminals {
            let result = match term {
                "gnome-terminal" => std::process::Command::new(term)
                    .arg("--working-directory")
                    .arg(dir)
                    .spawn(),
                "konsole" => std::process::Command::new(term)
                    .arg("--workdir")
                    .arg(dir)
                    .spawn(),
                _ => std::process::Command::new(term)
                    .current_dir(dir)
                    .spawn(),
            };
            if result.is_ok() {
                break;
            }
        }
    }
    #[cfg(not(any(target_os = "macos", target_os = "windows", target_os = "linux")))]
    {
        let _ = dir; // suppress unused warning
    }
}

/// Show file/folder properties/info dialog
#[cfg(any(target_os = "windows", target_os = "macos"))]
pub(super) fn shell_properties(path: &Path) {
    #[cfg(target_os = "macos")]
    {
        let script = format!(
            "tell application \"Finder\" to open information window of (POSIX file \"{}\" as alias)",
            path.display().to_string().replace('"', "\\\"")
        );
        let _ = std::process::Command::new("osascript")
            .arg("-e")
            .arg(&script)
            .spawn();
    }
    #[cfg(target_os = "windows")]
    {
        use std::os::windows::process::CommandExt;
        let path_str = path.to_string_lossy().to_string().replace('\'', "''");
        let script = format!(
            "$shell = New-Object -ComObject Shell.Application; $shell.NameSpace((Split-Path '{0}')).ParseName((Split-Path '{0}' -Leaf)).InvokeVerb('properties')",
            path_str
        );
        let _ = std::process::Command::new("powershell")
            .args(["-NoProfile", "-Command", &script])
            .creation_flags(0x08000000)
            .spawn();
    }
}

/// Move file/folder to trash (cross-platform)
pub(super) fn shell_trash(path: &Path) {
    if let Err(e) = trash::delete(path) {
        log::error!("Failed to move to trash: {}", e);
    }
}

// ── Helper functions ──

/// Convert file path to its directory path
fn path_to_dir(path: &Path) -> &Path {
    if path.is_file() {
        path.parent().unwrap_or(path)
    } else {
        path
    }
}
