//! Manual / diagnostic CLI tests: `dirstat-rs test <name> [...]`
//!
//! Not `cargo test`; avoids GUI and persists for on-machine checks.

use std::path::PathBuf;

pub fn run(args: &[String]) -> anyhow::Result<()> {
    let sub = args.first().map(|s| s.as_str()).unwrap_or("help");

    match sub {
        "help" | "-h" | "--help" => {
            print_help();
            Ok(())
        }
        "ping" => {
            println!("cli_test pong");
            Ok(())
        }
        "ntfs-available" => {
            let path = ntfs_sample_path(args.get(1));
            println!("checking: {:?}", path.display());
            #[cfg(windows)]
            {
                use crate::scanner_ntfs;
                let ok = scanner_ntfs::is_ntfs_available(&path);
                println!("is_ntfs_available: {}", ok);
                Ok(())
            }
            #[cfg(not(windows))]
            {
                println!("is_ntfs_available: n/a (not Windows)");
                Ok(())
            }
        }
        "volume-open" => {
            let path = ntfs_sample_path(args.get(1));
            println!("opening raw volume for: {:?}", path.display());
            #[cfg(windows)]
            {
                crate::scanner_ntfs::probe_raw_volume_access(&path)?;
                println!("volume-open: OK (handle opened and closed)");
                Ok(())
            }
            #[cfg(not(windows))]
            {
                anyhow::bail!("volume-open is Windows-only")
            }
        }
        "mft-ready" => {
            #[cfg(windows)]
            {
                use crate::scanner_ntfs;
                let path = ntfs_sample_path(args.get(1));
                let max_diag = args
                    .get(2)
                    .and_then(|s| s.parse::<usize>().ok())
                    .unwrap_or(3);
                println!("path: {:?}", path.display());
                let fs_ok = scanner_ntfs::is_ntfs_available(&path);
                println!("is_ntfs_available: {}", fs_ok);
                if !fs_ok {
                    println!("MFT fast path: no (not NTFS or path has no drive letter). App will use jwalk here.");
                    return Ok(());
                }
                scanner_ntfs::probe_raw_volume_access(&path)?;
                println!("volume device open: OK");
                let report = scanner_ntfs::diagnose_fsctl_enum_usn(&path, max_diag)?;
                println!("---\n{}\n---", report);
                println!(
                    "MFT_ioctl: works on this volume — you may enable Settings → Scanner → NTFS MFT."
                );
                println!(
                    "Default GUI scanner is jwalk (Standard), NOT MFT unless you change saved settings.",
                );
                Ok(())
            }
            #[cfg(not(windows))]
            {
                let _ = args;
                anyhow::bail!("mft-ready is Windows-only")
            }
        }
        "enum-diagnose" => {
            #[cfg(windows)]
            {
                let path = ntfs_sample_path(args.get(1));
                let max_lp = args
                    .get(2)
                    .and_then(|s| s.parse::<usize>().ok())
                    .unwrap_or(8);
                let report = crate::scanner_ntfs::diagnose_fsctl_enum_usn(&path, max_lp)?;
                println!("{}", report);
                Ok(())
            }
            #[cfg(not(windows))]
            {
                let _ = args;
                anyhow::bail!("enum-diagnose is Windows-only")
            }
        }
        "mft-list" => {
            #[cfg(windows)]
            {
                let path = ntfs_sample_path(args.get(1));
                let n = args
                    .get(2)
                    .and_then(|s| s.parse::<usize>().ok())
                    .unwrap_or(40);
                print!("{}", crate::scanner_ntfs::mft_dump_names(&path, n)?);
                Ok(())
            }
            #[cfg(not(windows))]
            {
                let _ = args;
                anyhow::bail!("mft-list is Windows-only")
            }
        }
        _ => anyhow::bail!(
            "unknown test {:?}; run `dirstat-rs test help` for commands",
            sub
        ),
    }
}

fn ntfs_sample_path(extra: Option<&String>) -> PathBuf {
    if let Some(p) = extra {
        return PathBuf::from(p);
    }
    #[cfg(windows)]
    {
        PathBuf::from("C:\\")
    }
    #[cfg(not(windows))]
    {
        PathBuf::from("/")
    }
}

fn print_help() {
    eprintln!(
        r#"dirstat-rs test — diagnostic CLI harness (not `cargo test`)

USAGE:
    dirstat-rs test [NAME] [ARGS...]

TESTS:
    help               This list
    ping               Sanity check (prints pong)
    ntfs-available [PATH]   Print whether `is_ntfs_available` is true (default: C:\ on Windows)
    volume-open    [PATH]   Try opening \\.\X: like MFT scanner (often needs admin)
    mft-ready [PATH] [N] IOCTL smoke + hint; N = enum-diagnose rounds (default 3)
    mft-list [PATH] [N]  First N FILE/DIR names from MFT enumeration (default 40); not full paths
    enum-diagnose [PATH] [N] Peek USN enumeration (histogram); N=max IOCTL rounds (default 8)
"#
    );
}
