# Bug Hunt Plan 2 - residual readback and invariant cleanup

Date: 2026-05-16
Branch: main
Inputs: `BUG_HUNT_REPORT.md`, `AGENTS.md`, `DIAGRAMS.md`, existing `.bughunt/plan1.md`

## Goal

Remove the remaining shared GPU readback panic path and reduce hot-render invariant panics without reintroducing divergent code paths.

## Status

Waiting for approval before code changes.

## Tasks

### 1. Shared readback Result API

Files:
- `crates/render-core/src/lib.rs`
- `crates/treemap/src/wgpu.rs`
- `crates/render-3d/src/lib.rs`
- `crates/render-3d/src/pt/megakernel/render.rs`
- `src/app/screenshot.rs` if caller signatures need propagation

Steps:
1. Add a compact `ReadbackError` enum in `render_core::gpu`.
2. Change `map_readback` to return `Result<Vec<u8>, ReadbackError>`.
3. Replace `tx.send(result).unwrap()` with ignored/logged send failure or structured error handling.
4. Replace `rx.recv().unwrap().unwrap()` with explicit `RecvError` and map-error handling.
5. Update every caller to handle `Result` once at the mode boundary.

Success criteria:
- No `rx.recv().unwrap().unwrap()` remains in render readback code.
- Map failures produce logs and a recoverable result, not a panic.
- `cargo check --workspace --exclude squarebob-rs --exclude media-encoder` passes.

### 2. Checked readback layout arithmetic

Files:
- `crates/render-core/src/lib.rs`

Steps:
1. Compute `bytes_per_row`, padded row bytes, total buffer size, and pixel capacity with checked `u64`/`usize` arithmetic.
2. Return `ReadbackError::SizeOverflow` for impossible dimensions.
3. Keep row slicing derived from the same checked layout object so allocation and extraction cannot diverge.

Success criteria:
- No `width * height * 4` or `padded_bytes_per_row * height` unchecked arithmetic remains in readback helpers.

### 3. Unsafe comment cleanup

Files:
- `src/app/helpers.rs`
- `crates/xtask/src/env_setup.rs`

Steps:
1. Add `// SAFETY:` for `GetDiskFreeSpaceExW` pointer usage.
2. Add `// SAFETY:` for `libc::statvfs` and `assume_init`.
3. Add `// SAFETY:` for `std::env::remove_var`, matching the existing `env_set` invariant.

Success criteria:
- `grep unsafe` shows every unsafe block in these files has a nearby `// SAFETY:` comment.

### 4. Deduplicate megakernel path-tracer initialization

Files:
- `crates/render-3d/src/pt/megakernel/render.rs`
- `crates/render-3d/src/pt/megakernel/render_no_readback.rs`
- optionally `crates/render-3d/src/pt/megakernel/mod.rs`

Steps:
1. Extract shared lazy initialization/env-forwarding into one helper.
2. Use the helper in both readback and no-readback render paths.
3. Preserve dirty-flag behavior exactly.

Success criteria:
- The env texture/CDF forwarding block exists in only one place.
- Both render paths still compile and preserve behavior.

### 5. Centralize render resource invariants

Files:
- `crates/render-3d/src/lib.rs`
- `crates/render-3d/src/renderer3d/render.rs`
- `crates/render-3d/src/pt/megakernel/render.rs`
- `crates/render-3d/src/pt/megakernel/render_no_readback.rs`
- `crates/treemap/src/wgpu.rs`

Steps:
1. Replace post-assignment cache expects with direct local bindings or `Option::insert` patterns.
2. Add one `require_render_state` or `render_state_after_ensure` helper if it materially reduces repeated expects.
3. For 2D GPU, consider a small render-target bundle if it avoids repeated `render_texture`/`render_view` unwraps.

Success criteria:
- Panic surface is smaller and centralized.
- No new abstraction is introduced unless it removes repeated checks.

### 6. Dead-code classification

Files:
- `crates/pt-mats/src/lib.rs`
- `crates/render-shared/src/lib.rs`
- `crates/treemap/src/lib.rs`
- optional PT config modules

Steps:
1. Keep platform/API parity stubs documented in `scanner_ntfs.rs`.
2. For old material classifiers and non-inertia camera helpers, trace references and decide keep-with-test vs remove.
3. Do not mix removals into the readback fix PR unless they are trivial and proven unrelated.

Success criteria:
- Each remaining `#[allow(dead_code)]` is either documented as intentional or removed.

## Verification Plan

- `cargo check --workspace --exclude squarebob-rs --exclude media-encoder`
- `cargo check -p render-core -p treemap -p render-3d -p pt-megakernel`
- Full `cargo check --workspace` only after native ffmpeg dependencies are available (`VCPKG_ROOT` or `PKG_CONFIG_PATH` for `libavutil.pc`).
- Grep checks:
  - `rx.recv().unwrap().unwrap`
  - `map_readback(`
  - `unsafe {`
  - `#[allow(dead_code)]`

## Open Blockers

- Full workspace verification is blocked in this environment by missing ffmpeg discovery for `ffmpeg-sys-next v8.1.0`.
- GitNexus graph query failed with `Transport closed`, so caller tracing for this pass used filesystem searches and direct reads.
