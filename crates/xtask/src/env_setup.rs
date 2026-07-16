//! Build-time environment bootstrap for MSVC and libclang.

#[cfg(windows)]
use anyhow::Context;
use anyhow::Result;
use std::ffi::OsStr;
use std::path::PathBuf;

#[cfg(windows)]
use vcv_rs::Arch;
#[cfg(windows)]
use vcv_rs::detect::{detect_sdk, detect_ucrt, detect_vs};
#[cfg(windows)]
use vcv_rs::env::build_env;

fn env_set<K: AsRef<OsStr>, V: AsRef<OsStr>>(key: K, val: V) {
    // SAFETY: `xtask` mutates env only once on the main thread before spawning Cargo.
    unsafe {
        std::env::set_var(key, val);
    }
}

fn env_remove_var(key: &str) {
    unsafe {
        std::env::remove_var(key);
    }
}

pub fn prepare_build_environment() -> Result<()> {
    #[cfg(windows)]
    windows_msvc_paths()?;
    fix_libclang();
    Ok(())
}

#[cfg(windows)]
fn windows_msvc_paths() -> Result<()> {
    let vs = detect_vs(None).with_context(|| {
        format!(
            "Visual Studio MSVC not detected (need vswhere + VC tools). Versions found: {:?}",
            vcv_rs::detect::list_vs_versions()
        )
    })?;
    let sdk = detect_sdk().context("Windows SDK (10.x) not found in registry")?;
    let ucrt = detect_ucrt().context("Universal CRT (Windows Kits 10.x) not found")?;

    let assembled = build_env(&vs, Some(&sdk), Some(&ucrt), Arch::X64, Arch::X64);

    for (k, v) in &assembled.vars {
        env_set(k, v);
    }

    prepend_sem_paths("PATH", &assembled.path);
    prepend_sem_paths("INCLUDE", &assembled.include);
    prepend_sem_paths("LIB", &assembled.lib);
    prepend_sem_paths("LIBPATH", &assembled.libpath);

    eprintln!("xtask: MSVC toolchain environment applied (via vcv-rs)");
    Ok(())
}

#[cfg(windows)]
fn prepend_sem_paths(var: &'static str, extra: &[PathBuf]) {
    if extra.is_empty() {
        return;
    }
    let prefix = paths_to_string(extra);
    merge_with_sep(var, &prefix, ';');
}

#[cfg(windows)]
fn paths_to_string(extra: &[PathBuf]) -> String {
    extra
        .iter()
        .map(|p| p.display().to_string())
        .collect::<Vec<_>>()
        .join(";")
}

#[cfg(windows)]
fn merge_with_sep(key: &'static str, prefix: &str, sep: char) {
    if prefix.is_empty() {
        return;
    }
    let merged = match std::env::var(key) {
        Ok(rest) if !rest.is_empty() => format!("{prefix}{sep}{rest}"),
        _ => prefix.to_owned(),
    };
    env_set(key, merged.as_str());
}

/// Clear `LIBCLANG_PATH` if it points at ESP-IDF / Xtensa clang (same as bootstrap.py).
fn fix_libclang() {
    let Ok(lcp) = std::env::var("LIBCLANG_PATH") else {
        return;
    };
    let lower = lcp.to_lowercase();
    if lower.contains("esp") || lower.contains("xtensa") {
        eprintln!("xtask: clearing LIBCLANG_PATH (ESP/Xtensa clang breaks bindgen/msvc-sys)");
        env_remove_var("LIBCLANG_PATH");
    }
}
