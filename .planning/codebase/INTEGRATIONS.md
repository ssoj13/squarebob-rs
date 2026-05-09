# External Integrations

**Analysis Date:** 2026-05-09

## APIs & External Services

**Web / SaaS APIs:**
- None. `dirstat-rs` is a fully offline desktop application. No HTTP client (`reqwest`, `ureq`, `hyper`), no gRPC, no GraphQL, and no third-party SaaS SDK is declared in any workspace `Cargo.toml`.

**Telemetry / Analytics:**
- None. No `sentry`, `tracing-opentelemetry`, `metrics`, or analytics SDK is present.

## Operating System Integrations

**Windows (`cfg(windows)`):**
- `windows` 0.62 (root `Cargo.toml:64-71`) with feature set:
  - `Win32_Storage_FileSystem` - file metadata, volume info, USN journal records
  - `Win32_System_Ioctl` - `FSCTL_ENUM_USN_DATA` ioctl for NTFS Master File Table enumeration
  - `Win32_System_IO` - `DeviceIoControl` for issuing the USN ioctls
  - `Win32_Foundation` - `HANDLE`, `CloseHandle`, `GENERIC_READ`, `ERROR_HANDLE_EOF`, `HRESULT` types
  - `Win32_Security` - security descriptor / access flags used when opening volume handles
- Consumed by `src/scanner_ntfs.rs` for the fast NTFS-aware scanner path. Imports include `GetVolumeInformationW`, `DeviceIoControl`, `FSCTL_ENUM_USN_DATA`, and `HSTRING` (verified at `src/scanner_ntfs.rs:24-26, 77-78, 307-311, 378-379, 438-442, 605-609`).

**Unix (`cfg(unix)`):**
- `libc` 0.2 (root `Cargo.toml:61-62`) - POSIX `stat`/`statx`-style metadata access for the generic scanner fallback. Limited usage; no Unix-specific scanner module on par with `scanner_ntfs.rs` exists.

## Filesystem Integrations

**Directory traversal:**
- `jwalk` 0.8 - Parallel walker driving the cross-platform scanner in `src/scanner.rs`. Configured with worker count from `num_cpus`.

**File operations from the UI:**
- `rfd` 0.17 - Native open/save folder dialogs (file picker on "Open folder")
- `trash` 5 - Sends deleted entries to the OS recycle bin / trash rather than hard-deleting
- `open` 5 - Launches the OS default handler ("Open with default app", "Reveal in Explorer/Finder")

**Process / system info:**
- `sysinfo` 0.38 - Reports current process memory and host CPU count for the status bar

## Data Storage

**Databases:**
- None. No SQLite (`rusqlite`/`sqlx`), no embedded KV (`sled`/`redb`), no client driver of any kind.

**Local persistence:**
- `directories` 6 - Resolves OS-appropriate config/cache/data directories via `ProjectDirs` (`src/app/presets.rs:5`)
- `bincode` 1 - Binary serialization for the on-disk scan cache (`src/cache.rs`)
- `serde_json` 1 - JSON serialization for human-editable settings/presets
- `sha2` 0.11 - SHA-256 path hashing to derive cache filenames (`src/path_key.rs:3`)
- `eframe` `persistence` feature - egui state (window size, dock layout) persisted to platform-appropriate location

**File Storage:**
- Local filesystem only. No object storage, no cloud SDK (`aws-sdk-*`, `google-cloud-*`, `azure_*`).

**Caching:**
- Custom on-disk cache under the OS cache directory (managed by `src/cache.rs` using `bincode` + `sha2` keys). No Redis / Memcached.

## GPU Integrations

**Graphics API:**
- `wgpu` 29 - Cross-platform GPU abstraction. Backend selected at runtime by wgpu (DX12/Vulkan on Windows, Vulkan on Linux, Metal on macOS).
  - Adapter/device setup centralized in `crates/render-core` (`render-core/Cargo.toml:7-8` pulls `wgpu` + `pollster`).
  - Used by: root binary (via `egui-wgpu` for UI), `render-3d` (scene renderer), `treemap` (optional `wgpu` feature), `bvh-gpu` (BVH compute), `pt-megakernel` (path tracer), `pt-wavefront` (path tracer).

**Compute:**
- All GPU compute is expressed as wgpu compute pipelines + WGSL. No CUDA crate dependency despite `treemap`'s `cuda` feature flag (the flag is declared at `crates/treemap/Cargo.toml:28` but pulls in no CUDA dependency; it gates CPU code paths only).

## Image / Media I/O

- `image` 0.25 - PNG/JPEG/HDR/EXR encoding & decoding for screenshot export and texture loading (root binary, `render-3d`)
- `half` 2.7.1 - `f16` interop for HDR pixel buffers and GPU storage textures

## Serialization Formats

| Format | Crate | Usage |
|--------|-------|-------|
| JSON | `serde_json` 1 | User-editable settings, presets, dev-only round-trip tests in `render-shared` |
| Binary | `bincode` 1 | On-disk scan cache (compact, fast) |
| Derive | `serde` 1 (`derive`) | All persisted types in `dirstat-core`, `render-shared`, `pt-mats`, root binary |
| GPU POD | `bytemuck` 1 (`derive`) | Zero-copy struct uploads to GPU buffers |

## Authentication & Identity

- None. The application is single-user, runs entirely client-side, and has no notion of accounts, sessions, OAuth, JWT, or credential storage.

## Monitoring & Observability

**Logging:**
- `log` 0.4 facade used throughout the workspace
- `env_logger` 0.11 implementation initialized in `src/main.rs`; verbosity controlled by `RUST_LOG`

**Error Tracking:**
- None. No Sentry, Bugsnag, or crash-reporting SDK.

**Metrics / Tracing:**
- None. No `tracing`, `metrics`, `prometheus`, or OpenTelemetry crate.

## CI/CD & Deployment

**Hosting:**
- Not applicable. Distributed as a native desktop binary; there is no server component.

**CI Pipeline:**
- No `.github/workflows/`, `.gitlab-ci.yml`, or `azure-pipelines.yml` referenced from manifests. (Workflow files were not enumerated as part of this audit; verify under `.github/` if needed.)

**Distribution:**
- `cargo build --release` produces a single binary. No installer, packaging, or notarization configuration is declared in workspace manifests.

## Environment Configuration

**Required env vars:**
- None are *required*. The application starts with sensible defaults.

**Optional env vars:**
- `RUST_LOG` - controls `env_logger` verbosity (e.g. `RUST_LOG=dirstat_rs=debug`)
- Standard `wgpu` env vars (`WGPU_BACKEND`, `WGPU_POWER_PREF`) are honored by the `wgpu` crate but not set or read explicitly by application code.

**Secrets location:**
- None. The application stores no credentials, API keys, or tokens. No `.env`, `.env.example`, or secrets directory is part of the project layout.

## Webhooks & Callbacks

**Incoming:**
- None. Application exposes no network listener.

**Outgoing:**
- None. Application makes no outbound HTTP/RPC calls.

## Inter-process / IPC

- None. The app is a single-process binary. Worker concurrency is in-process via `rayon` thread pools and `crossbeam-channel` for scanner-to-UI message passing.

---

*Integration audit: 2026-05-09*
