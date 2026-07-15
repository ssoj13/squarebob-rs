# Bug Hunt Plan 3 — SSH VFX Dependency Migration

Date: 2026-07-15
Repository: `squarebob-rs`
Status: implementation complete; targeted verification passed; full-feature verification externally blocked

## Outcome

The obsolete `vfx-rs` monorepo paths are removed from active configuration. Split VFX crates are consumed from their current SSH Git repository:

- `vfx-core`, `vfx-io`, `vfx-ocio`, `vfx-lut` → `ssh://git@github.com/ssoj13/oiio-rs.git`, branch `main`.
- OpenEXR implementation → transitive `exr-core` from `ssh://git@github.com/ssoj13/exr-rs.git`.
- Direct `vfx-exr` dependency removed. The media encoder already writes EXR through `vfx-io`; the direct dependency was stale.
- Local `[patch]` overrides for deleted `../vfx-rs/crates/**` paths removed.

Primary references:

- `Cargo.toml:112`
- `Cargo.toml:113`
- `Cargo.toml:120`
- `Cargo.toml:124`
- `crates/media-encoder/Cargo.toml:11`
- `crates/media-encoder/Cargo.toml:26`
- `crates/media-encoder/Cargo.toml:27`

## Locked Revisions

`Cargo.lock` resolves:

- `oiio-rs`: `21664a2ffe757cef7bc01658ba842e852768ec71`
- `exr-rs`: `c6fdc69dea277c1744ce724827d3e340fdd510c9`

Representative references:

- `Cargo.lock:2197`
- `Cargo.lock:7836`

## Systemic Feature-Boundary Repair

The migration exposed pre-existing feature leakage: optional FFmpeg and EXR implementations were referenced from unconditional code.

Repairs:

- FFmpeg imports, encoder implementation, and swscale utilities compile only with `feature = "ffmpeg"`.
- Public `encode_comp` API remains stable and returns `EncodeError::BackendUnavailable("FFmpeg")` when disabled.
- EXR writers compile only with `feature = "exr"`.
- Persisted `SequenceFormat::Exr` remains deserializable; execution returns `EncodeError::BackendUnavailable("OpenEXR")` when disabled.
- `SequenceFormat::all()` exposes only compiled formats.
- Default sequence format is EXR with `exr`, PNG without `exr`.
- UI imports both worker entry points explicitly.

References:

- `crates/media-encoder/src/dialogs/encode/encode.rs:402`
- `crates/media-encoder/src/dialogs/encode/encode.rs:523`
- `crates/media-encoder/src/dialogs/encode/encode.rs:537`
- `crates/media-encoder/src/dialogs/encode/encode.rs:1209`
- `crates/media-encoder/src/dialogs/encode/encode.rs:2084`
- `crates/media-encoder/src/dialogs/encode/encode.rs:2135`
- `crates/media-encoder/src/dialogs/encode/encode.rs:2246`
- `crates/media-encoder/src/dialogs/encode/encode.rs:2581`
- `crates/media-encoder/src/dialogs/encode/encode.rs:2689`
- `crates/media-encoder/src/dialogs/encode/encode_ui.rs:18`

## Toolchain Alignment

Current `oiio-rs` crates require Rust 1.96. Workspace MSRV and pinned toolchain now agree:

- `Cargo.toml:44`
- `rust-toolchain.toml:2`
- `README.md:309`

## Verification

| Command | Result |
|---|---|
| `cargo check -p media-encoder --no-default-features` | PASS, no warnings |
| `cargo check -p media-encoder --no-default-features --features exr` | PASS, no warnings |
| `cargo test -p media-encoder --no-default-features --features exr --lib` | PASS, 9/9 |
| Active-tree search for `vfx-exr`, `vfx_exr`, `ssoj13/vfx-rs`, `../vfx-rs` | PASS; only historical `.bughunt/plan2.md:25` remains |
| SSH dependency fetch and lock resolution | PASS |
| `cargo check -p media-encoder --all-features` | BLOCKED before crate compilation by missing system FFmpeg/vcpkg metadata |

The full-feature blocker is external environment setup, not VFX dependency resolution: `ffmpeg-sys-next` cannot find `libavutil` because `VCPKG_ROOT` and a system FFmpeg installation are absent.

## Impact and Commit Safety

Pre-edit GitNexus upstream impact:

- `encode_comp`: LOW
- `encode_image_sequence`: LOW
- `write_exr_frame`: LOW
- `EncodeDialog::start_encoding`: LOW
- `EncodeError`: LOW

Final whole-worktree `gitnexus_detect_changes(scope: "all")` reports CRITICAL: 90 changed files, 963 changed symbols, 260 affected symbols. This result includes the existing multi-agent bug-hunt worktree, not only this migration.

Do not commit the entire worktree as one dependency migration. Review or stage the intended scope explicitly.

## Intended Migration Scope

- `Cargo.toml`
- `Cargo.lock`
- `rust-toolchain.toml`
- `crates/media-encoder/Cargo.toml`
- `crates/media-encoder/src/dialogs/encode/encode.rs`
- `crates/media-encoder/src/dialogs/encode/encode_ui.rs`
- `README.md`
- `docs/oidn-integration-plan.md`
- `crates/xtask/src/main.rs`
- `.bughunt/plan3.md`

## Remaining External Action

Install/configure the project-supported FFmpeg development environment, then rerun:

```powershell
cargo check -p media-encoder --all-features
```

No source workaround is appropriate for the missing native FFmpeg libraries.
