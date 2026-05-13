# HANDOFF

Date: 2026-05-12
Repository: `C:\projects\projects.rust.cg\dirstat-rs`
Branch: `main`

## Current State

The repository currently contains a reusable `media-encoder` crate and the app has an `E` toggle button in the top-right toolbar that opens the encoder dialog.

Latest relevant commits before this working tree:

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

The current implementation connects the encoder dialog to the app through a request/response `FrameSource`.

Current behavior:

- opening the `E` dialog creates a `DirstatEncodeSource`
- the dialog gets `Some(&Comp)` instead of `None`
- the copied Playa encoder calls `FrameSource::get_frame(frame_idx, true)` from its encoder thread
- `DirstatEncodeSource::get_frame` sends an `EncodeFrameRequest` to the app thread and blocks
- `App::handle_image_sequence` receives the request, applies the frame time, waits for render readiness, captures the viewport, and replies with `media_encoder::Frame::rgba8`
- the encode dialog now has editable frame range controls, defaulting to `0..119`

This avoids calling the renderer/GPU directly from the encoder thread.

## Why It Was Done This Way

The copied Playa encoder expects a `Comp` / `FrameSource` object that can be read from the encoding path.

Dirstat currently renders through the app render loop and GPU/UI state. That means the real animation/image-sequence export cannot be implemented safely by just calling the renderer from the encoder thread without designing a frame production pipeline.

The snapshot adapter was added as the smallest safe bridge:

- it proves the copied encoder dialog is wired into the app
- it avoids local path dependencies
- it avoids merging copied encoder modules into one file
- it avoids forcing GPU rendering from the encoder background thread
- it gives the next implementation a concrete `FrameSource` boundary to replace

## Important Notes

The encoder now produces real per-frame requests rather than encoding a single snapshot.

For 3D mode, frame time is mapped as:

```text
seconds = (frame_idx - frame_start) / fps
animation_time = base_animation_time + seconds * animation_speed
env_time = base_env_time + seconds * animation_speed * env_speed
```

The app disables live animation toggles during encode and restores them after encode completes/stops.

If path tracing is enabled, the app waits for `pt_max_samples` before handing the frame to the encoder. If path tracing is disabled, it captures after the requested 3D render pass.

## What Still Needs To Be Done

### 1. Validate End-To-End Encoding In The UI

The code compiles, but the next practical step is to run the app and verify actual output files:

- video output through ffmpeg
- image sequence output
- Stop button while `get_frame` is blocked
- 3D path-traced sequence with low `pt_max_samples`
- non-path-traced 3D sequence

### 2. Restore/Adapt Previous Export Logic If Useful

There was previous Dirstat image-sequence/export logic before the Playa encoder replacement.

Useful source can be recovered from git history before commit:

- `5aa45eb Replace media encoder with copied Playa encoder`

That old code likely contains app-specific handling for:

- advancing `render_3d_opts.animation_time`
- waiting for render/sample completion
- capturing viewport frames
- writing frame sequences

The current implementation already adapted the main idea: UI-thread frame production and render-state restore. The old code may still be useful for output defaults or sampling UI.

### 3. CI / Cross-Platform Verification

Verify:

- image sequence output
- video output through ffmpeg
- behavior on Windows with `C:\vcpkg`
- CI behavior later on macOS/Linux/Windows

Current local checks that passed before this handoff:

```powershell
cargo fmt --check -p media-encoder -p dirstat-rs
cargo run -p xtask -- check -p media-encoder
cargo run -p xtask -- check -p dirstat-rs
cargo run -p xtask -- clippy -p media-encoder -p dirstat-rs --all-targets -- -D warnings
```

`xtask` currently falls back to global `VCPKG_ROOT`, which is expected on this machine because vcpkg exists at:

```text
C:\vcpkg
```

Use `xtask clippy`, not raw `cargo clippy`, because FFmpeg/bindgen needs the same vcpkg/MSVC environment bootstrap as `xtask check`.

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
