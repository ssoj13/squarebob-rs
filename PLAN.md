# Plan: replace `imageseq-rs` with universal `media-encoder`

## Summary

- Rename the current `imageseq-rs` crate to `media-encoder` and make it a reusable media export crate.
- Port Playa's encoder and UI as the base implementation, replacing Playa-specific `Comp` / `Project` / `Frame` with a universal frame source boundary.
- Include FFmpeg, image sequence formats, EXR via `vfx-rs`, and Playa's `xtask` / vcpkg / `vcv-rs` build setup adapted for this repository.

## Key Changes

- Workspace:
  - replace `crates/imageseq-rs` with `crates/media-encoder`;
  - update root `Cargo.toml`, `Cargo.lock`, imports, and app state;
  - remove old `imageseq_rs::*` usage from `dirstat-rs`.
- `media-encoder`:
  - copy Playa `encode.rs` / `encode_ui.rs` as the implementation base;
  - preserve codec/container settings: H.264, H.265, ProRes, AV1, MP4, MOV, MKV;
  - preserve sequence settings: EXR, PNG, JPEG, TIFF, TGA;
  - preserve progress, cancel, ETA, and thread model from Playa UI.
- Universal API:
  - introduce a `FrameSource` trait instead of `playa_engine::Comp`;
  - introduce crate-owned frame model: `MediaFrame { width, height, pixels, format }`;
  - support at least `Rgba8`, `Rgba16`, `RgbaF16`, `RgbaF32`;
  - `dirstat-rs` implements `FrameSource`, waits for `max_samples`, reads back renderer pixels, and feeds frames to the encoder.
- FFmpeg:
  - port the FFmpeg wrapper from `crates/playa-ffmpeg` into the workspace for `media-encoder`;
  - keep vcpkg static linking and encoder selection logic from Playa;
  - initialize FFmpeg from app startup or lazily inside `media-encoder`.
- EXR / image formats:
  - add `vfx-core`, `vfx-exr`, `vfx-io` from `https://github.com/ssoj13/vfx-rs.git`, branch `main`, with `exr` / `htj2k` features;
  - port Playa EXR compression enum and writer path;
  - keep PNG/JPEG/TIFF/TGA through the `image` crate as in Playa.
- Build setup:
  - copy `vcpkg.json` and `vcpkg-configuration.json`;
  - copy and adapt `crates/xtask`;
  - keep `vcv-rs` for Windows MSVC environment discovery;
  - use `cargo xtask build/test` as the recommended local/native path for media dependencies.

## Integration Into `dirstat-rs`

- Rendering settings panel uses `media_encoder::EncodeDialog`.
- Export start:
  - force 3D path tracing when needed;
  - store and restore render state;
  - freeze animation toggles while export runs;
  - advance animation/env time per frame from FPS;
  - wait until PT sample count reaches requested max samples before writing/encoding each frame.
- Controls:
  - while running, dialog controls are disabled except Stop;
  - Stop cancels source/encoder and restores render state.
- Video export remains available in UI from the first port; if local FFmpeg/vcpkg is missing, build failure is acceptable for this phase.

## Test Plan

- `cargo xtask build --debug`
- `cargo xtask test --debug`
- `cargo check --workspace --all-targets`
- `cargo clippy --workspace --all-targets -- -D warnings`
- Unit tests:
  - frame path padding `####` / `@@@@`;
  - format settings serialization;
  - `FrameSource` cancellation path;
  - EXR compression string mapping.
- Manual smoke:
  - export PNG sequence from `dirstat-rs`;
  - export EXR sequence;
  - export MP4 H.264 if vcpkg FFmpeg is installed;
  - Stop during export restores UI/render state.

## Assumptions

- `media-encoder` is the new public reusable crate name; `imageseq-rs` will be removed, not kept as an alias.
- Native FFmpeg/vfx dependencies are allowed in default native builds now.
- CI can break temporarily until vcpkg/xtask is wired into GitHub Actions later.
- Playa code should be ported first with minimal redesign; cleanup and format pruning happen after it compiles.
