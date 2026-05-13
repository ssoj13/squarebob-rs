# HANDOFF

Date: 2026-05-12
Repository: `C:\projects\projects.rust.cg\dirstat-rs`
Branch: `main`

## Current State

The repository currently contains a reusable `media-encoder` crate and the app has an `E` toggle button in the top-right toolbar that opens the encoder dialog.

Latest relevant commits:

- `a3bfdd2 Connect encoder to viewport source`
- `29c9939 Use git dependency for playa ffmpeg`
- `5aa45eb Replace media encoder with copied Playa encoder`

The working tree was clean before this handoff file was added.

## What Was Done

### `media-encoder`

Created/kept `crates/media-encoder` as a reusable crate copied/adapted from Playa encoder code, with modular structure rather than collapsing everything into one file.

Important parts:

- `crates/media-encoder/src/dialogs/encode/`
  - copied/adapted encode dialog and UI
- `crates/media-encoder/src/frame.rs`
  - frame representation
- `crates/media-encoder/src/source.rs`
  - generic `FrameSource` trait and `Comp` alias
- `crates/media-encoder/src/progress.rs`
  - encode progress state
- `crates/media-encoder/src/io/exr_layered.rs`
  - EXR/layered IO support path

The ffmpeg wrapper is not a local path dependency. It uses Git:

```toml
playa-ffmpeg = { git = "https://github.com/ssoj13/playa-ffmpeg", default-features = false, features = ["codec", "format", "software-resampling", "software-scaling", "nvenc", "static"], optional = true }
```

This was done so the project can build on another machine without depending on a local `C:\projects\...` path.

### App Integration

Files changed for app integration:

- `src/app/toolbar.rs`
  - added a separate small `E` toggle button near the 2D/3D control
  - when opened, it resets the cached encode source

- `src/app/state.rs`
  - added:
    - `encode_dialog: media_encoder::EncodeDialog`
    - `encode_source: Option<media_encoder::Comp>`
    - `encode_source_size: (u32, u32)`

- `src/app/image_sequence.rs`
  - added the app-side adapter between Dirstat and `media-encoder`
  - current adapter is `ViewportFrameSource`
  - it implements `media_encoder::FrameSource`
  - it captures the current viewport using `App::capture_viewport`
  - it passes that source into `EncodeDialog::render`

## How It Is Done Right Now

The current implementation connects the encoder dialog to the app by taking a snapshot of the current rendered viewport and exposing it as a `media_encoder::FrameSource`.

Current behavior:

- opening the `E` dialog captures the current viewport
- the dialog gets `Some(&Comp)` instead of `None`
- encode can start because there is now an active source
- the source currently has a one-frame play range: `(0, 0)`

This is a functional adapter, but it is not the final desired sequence encoder.

## Why It Was Done This Way

The copied Playa encoder expects a `Comp` / `FrameSource` object that can be read from the encoding path.

Dirstat currently renders through the app render loop and GPU/UI state. That means the real animation/image-sequence export cannot be implemented safely by just calling the renderer from the encoder thread without designing a frame production pipeline.

The snapshot adapter was added as the smallest safe bridge:

- it proves the copied encoder dialog is wired into the app
- it avoids local path dependencies
- it avoids merging copied encoder modules into one file
- it avoids forcing GPU rendering from the encoder background thread
- it gives the next implementation a concrete `FrameSource` boundary to replace

## Important Limitation

The current encoder source is only a single-frame viewport snapshot.

This is not the final implementation the app needs.

The user explicitly wants actual image sequence / animation encoding, not only a still snapshot. The next implementation must replace or extend `ViewportFrameSource` with a real frame source that can provide frame `N` as the renderer state changes over time.

## What Still Needs To Be Done

### 1. Implement Real Frame Sequence Source

Replace the temporary snapshot source with a Dirstat sequence source that can generate or request frames for a range.

Required behavior:

- expose a real play range, not `(0, 0)`
- produce distinct frames for `frame_idx`
- use Dirstat render settings and animation time
- support 2D and 3D render modes where possible
- avoid unsafe renderer access from the encoder thread

Likely shape:

- keep `media_encoder::FrameSource` as the public boundary
- add a Dirstat-specific frame producer in `src/app/image_sequence.rs` or a small submodule
- coordinate rendering on the app/UI/render thread
- let the encoder consume completed frames through a queue/cache

The key issue is thread ownership: the renderer and GPU state should stay on the app/render thread. The encoder thread should not directly mutate `App`, `Renderer3D`, `wgpu` state, or egui state.

### 2. Decide Frame Range UI

The copied Playa UI uses `active_comp.play_range(true)`.

Dirstat needs an app-level definition for:

- start frame
- end frame
- FPS
- animation duration
- whether frame range comes from render preset, encoder dialog, or app settings

Until this exists, `FrameSource::play_range` cannot honestly report a meaningful sequence range.

### 3. Restore/Adapt Previous Export Logic If Useful

There was previous Dirstat image-sequence/export logic before the Playa encoder replacement.

Useful source can be recovered from git history before commit:

- `5aa45eb Replace media encoder with copied Playa encoder`

That old code likely contains app-specific handling for:

- advancing `render_3d_opts.animation_time`
- waiting for render/sample completion
- capturing viewport frames
- writing frame sequences

This should be adapted into the new modular `media-encoder` integration instead of being pasted as a monolith.

### 4. Verify End-To-End Encoding

After the real sequence source is implemented, verify:

- still image output
- image sequence output
- video output through ffmpeg
- behavior on Windows with `C:\vcpkg`
- CI behavior later on macOS/Linux/Windows

Current local checks that passed before this handoff:

```powershell
cargo fmt --check -p media-encoder -p dirstat-rs
cargo run -p xtask -- check -p media-encoder
cargo run -p xtask -- check -p dirstat-rs
```

`xtask` currently falls back to global `VCPKG_ROOT`, which is expected on this machine because vcpkg exists at:

```text
C:\vcpkg
```

## Constraints / Decisions To Preserve

- Do not use GitNexus.
- Keep the encoder modular.
- Do not collapse copied Playa encoder code into one huge file.
- Do not use local path dependencies that will fail on another machine.
- Keep `playa-ffmpeg` as a Git dependency unless explicitly changed.
- The encoder crate should stay reusable by other apps.
- CI/CD can be handled later; current focus is encoder functionality.
- Push after completing a real fix, per user preference.

## Next Concrete Task

Implement the real Dirstat frame sequence producer.

Recommended first step:

1. Inspect the deleted/export-related code from before `5aa45eb`.
2. Identify exactly how it advanced animation and captured frames.
3. Reintroduce that behavior as a Dirstat-specific producer feeding `media_encoder::FrameSource`.
4. Keep renderer/GPU work on the app thread.
5. Leave `media-encoder` generic.

