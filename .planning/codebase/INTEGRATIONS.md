# External Integrations

**Analysis Date:** 2026-04-20

## Overview

**dirstat-rs** is a **local-first desktop disk visualizer**. There are **no cloud APIs, remote telemetry endpoints, or OAuth** built into the core stack. Integrations are **OS services**, **local disk metadata**, and **user-action hooks** (open file, trash, folder dialog).

## Local & OS

| Integration | Purpose | Code touchpoints |
|---------------|---------|------------------|
| **Standard filesystem walk** | Cross-platform recursive scan | `src/scanner.rs` (`jwalk`, `rayon`) |
| **Windows NTFS MFT** | Fast enumeration when available | `src/scanner_ntfs.rs`, `src/scanner.rs` (fallback messages) |
| **Shell reveal / terminal** | “Show in Explorer”, open terminal | `src/app/shell.rs` |
| **System trash** | Delete to recycle bin / trash | `trash` crate, `src/app/shell.rs`, UI from `src/app/treemap_view.rs` |
| **Open file or folder** | Default app / explorer | `open` crate |
| **Folder picker** | Browse for scan root | `rfd` — `src/app/helpers.rs` (`rfd_pick_folder`) |
| **Cross-platform paths / app dirs** | Cache & exclusion file locations | `directories`, `src/cache.rs`, `src/exclusions.rs`, `src/path_key.rs` |

## Data Persistence (Local Only)

| Store | Format | Notes |
|-------|--------|--------|
| **Scan cache** | **bincode** (v1 crate) | Written under user project dirs; keyed by stable path hash (`path_key`) |
| **Exclusions list** | File under app dirs | Same hashing pattern as cache |
| **egui / eframe persistence** | JSON via `eframe::Storage` | Window/settings state — `src/app/mod.rs` (`PersistState`, `save`) |
| **Named presets** | Files via `src/app/presets.rs` | User-named layout/render presets |

## Hardware / System Info

| Source | Use |
|--------|-----|
| **sysinfo** | RAM usage in status bar (`src/app/status_bar.rs`, `src/app/mod.rs` / `App::sys`) |
| **wgpu adapters** | GPU device selection for 2D GPU + 3D (`src/main.rs` Wgpu setup, `render-3d` / `treemap`) |

## Not Integrated (By Design)

- No HTTP client stack in root `Cargo.toml` for product features.
- No database beyond flat files.
- No analytics SDKs.

If future features add network or accounts, document new crates and threat model here.

---

*Integrations analysis: 2026-04-20*
