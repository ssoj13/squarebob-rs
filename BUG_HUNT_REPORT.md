# Bug Hunt Report - squarebob-rs

Date: 2026-05-16
Branch: main
Scope: full repository scan excluding `target/**` and `.git/**`.
Method: filesystem inventory, marker grep, targeted source reads, Context7 wgpu API check, Cargo verification, GitNexus attempt.

## Executive Summary

This pass verified the state after the existing `.bughunt/plan1.md` fixes. Most of the previous high-risk GPU resource-slot panic class was reduced, but a shared readback helper still contains the same fundamental failure mode: `render_core::gpu::map_readback` unwraps both the callback channel receive and the `wgpu` map result. That helper is used by 2D legacy rendering, 3D raster legacy rendering, screenshot paths, and PT megakernel readback rendering.

No `todo!()` or `unimplemented!()` sites were found. Source TODO markers are limited to documented media-encoder compression placeholders and historical comments. A workspace-wide `cargo check` is blocked by native ffmpeg discovery, but the workspace excluding the binary and `media-encoder` checks successfully.

## Findings

### [CRITICAL] Shared GPU readback helper panics on sender drop or `BufferAsyncError`

- Evidence: `crates/render-core/src/lib.rs:227` calls `buffer_slice.map_async` and receives a callback result.
- Evidence: `crates/render-core/src/lib.rs:228` calls `tx.send(result).unwrap()`.
- Evidence: `crates/render-core/src/lib.rs:232` calls `rx.recv().unwrap().unwrap()`.
- Callers: `crates/treemap/src/wgpu.rs:688`, `crates/render-3d/src/lib.rs:1041`, `crates/render-3d/src/lib.rs:1348`, `crates/render-3d/src/pt/megakernel/render.rs:481`.
- External API check: Context7 for `/gfx-rs/wgpu/wgpu-v29.0.1` shows `map_async` callback receives a `result` and examples check `result.is_ok()` before mapped data access; `device.poll(PollType::Wait)` only drives completion, it does not make map failure impossible.

Impact: GPU device loss, driver reset, mapping failure, or callback/channel failure can terminate the app during ordinary rendering, screenshot capture, or PT readback. Because the panic is in the shared helper, every caller inherits the same crash behavior.

Recommended fix: introduce `ReadbackError` in `render-core` and change `map_readback` to return `Result<Vec<u8>, ReadbackError>`. Handle both `rx.recv()` failure and callback `Err`. Update callers to log and return an empty/stale/fallback image instead of panicking. Keep one helper as the single source of truth.

### [HIGH] Readback buffer size arithmetic is unchecked before allocation and indexing

- Evidence: `crates/render-core/src/lib.rs:180` computes `let bytes_per_row = 4 * width` in `u32`.
- Evidence: `crates/render-core/src/lib.rs:181` computes padded row bytes in `u32`.
- Evidence: `crates/render-core/src/lib.rs:185` allocates `(padded_bytes_per_row * height) as u64` after multiplication.
- Evidence: `crates/render-core/src/lib.rs:235` reserves `(width * height * 4) as usize` after multiplication.
- Evidence: `crates/render-core/src/lib.rs:237-239` indexes rows using `row * padded_bytes_per_row` and `width * 4`.

Impact: very large offscreen/screenshot dimensions can overflow in debug builds or wrap in release builds before the cast. A wrapped buffer size can lead to an undersized readback buffer and later slice-index panic or incorrect output.

Recommended fix: do readback dimensions in `u64`/`usize` with `checked_mul` and return `ReadbackError::SizeOverflow` or `ReadbackError::ImageTooLarge`. Reuse the same checked layout in both `readback_texture` and `map_readback`.

### [MEDIUM] Unsafe blocks without `// SAFETY:` comments remain

- Evidence: `src/app/helpers.rs:238` wraps `GetDiskFreeSpaceExW` without a `// SAFETY:` comment.
- Evidence: `src/app/helpers.rs:271` wraps `libc::statvfs` without a `// SAFETY:` comment.
- Evidence: `src/app/helpers.rs:273` wraps `MaybeUninit::assume_init` without a `// SAFETY:` comment.
- Evidence: `crates/xtask/src/env_setup.rs:64` wraps `std::env::remove_var` without a `// SAFETY:` comment, while `env_set` has one at `crates/xtask/src/env_setup.rs:57`.

Impact: the code has real FFI and process-environment unsafety. Missing safety contracts make future reviews weaker and will fail if `undocumented_unsafe_blocks` is enabled consistently.

Recommended fix: add precise `// SAFETY:` comments immediately before each unsafe block. For `env_remove_var`, mirror the `env_set` invariant: xtask mutates environment on the main thread before spawning Cargo.

### [MEDIUM] Megakernel path-tracer initialization is duplicated across readback and no-readback paths

- Evidence: `crates/render-3d/src/pt/megakernel/render.rs:20-52` lazily constructs `PathTraceCompute`, forwards env texture/CDFs, mutates `pt_scene_dirty`/`pt_env_dirty`, then unwraps `path_tracer`.
- Evidence: `crates/render-3d/src/pt/megakernel/render_no_readback.rs:19-49` duplicates the same initialization and unwrap pattern.

Impact: any future change to path-tracer initialization, environment setup, CDF upload, or dirty-flag handling must be made twice. Divergence would affect only one of the readback/no-readback modes and would be hard to notice because both paths are valid user-facing modes.

Recommended fix: extract a single helper in the megakernel module, for example `ensure_path_tracer(renderer, opts, width, height) -> &mut PathTraceCompute`, and let both render paths call it. Be careful with borrow scope around `renderer.pt` and `renderer.ctx`.

### [MEDIUM] Remaining `expect` invariant checks are still in hot render paths

- Evidence: `crates/render-3d/src/lib.rs:1190-1192`, `1230-1232`, and `1236-1238` use `expect` after cache checks or after setting `cached_instances = Some(...)`.
- Evidence: `crates/render-3d/src/lib.rs:1318-1321` uses `expect` after `ensure_targets(width, height)`.
- Evidence: `crates/render-3d/src/renderer3d/render.rs:225` and `crates/render-3d/src/pt/megakernel/render.rs:455` retain the render-state expect pattern.
- Evidence: `crates/treemap/src/wgpu.rs:507`, `552`, `615`, `642`, and `683` retain resource-slot unwraps after `ensure_render_target` or allocation branches.

Impact: most of these are mechanically safe today, but they keep a panic-style invariant enforcement model in code that runs while rendering. It is also inconsistent with the previous plan's move toward bundled resources and graceful early returns.

Recommended fix: centralize each invariant in one helper or use resource bundles. Low-effort examples: use `Option::insert`/local bindings after assignment, `let Some(state) = ... else { log + return }` for UI render paths, and a small `RenderTargets2D` bundle in the 2D renderer.

### [LOW] Dead-code allowances should be classified as intentional API/stub vs obsolete logic

- Evidence: explicit dead-code allowances exist in `crates/pt-mats/src/lib.rs:854`, `861`, `916`, `crates/treemap/src/lib.rs:444`, `crates/render-shared/src/lib.rs:1194`, `1205`, `1353`, and several optional PT feature/config modules.
- Evidence: platform/API-parity stubs in `src/scanner_ntfs.rs:70`, `626`, `752`, `962`, `968` are documented as stubs and should not be removed casually.

Impact: no confirmed runtime bug. The risk is maintenance noise: obsolete classifiers and legacy non-inertia camera methods can hide actual unused code warnings.

Recommended fix: keep documented platform stubs. For material classifiers and camera helpers, either wire them into settings/tests or remove them in a dedicated cleanup PR after caller tracing.

## Verification

- Inventory: `search_files **/*` excluding `target/**` and `.git/**` returned 223 files.
- Markers: `TODO|FIXME|HACK|XXX|unimplemented!|todo!|panic!|unwrap(|expect(` scan found no `todo!`/`unimplemented!`; remaining TODOs are comments/placeholders.
- `cargo check --workspace`: failed before checking the whole workspace because `ffmpeg-sys-next v8.1.0` could not find vcpkg or `libavutil.pc`. Error says `VCPKG_ROOT` is not set and `PKG_CONFIG_PATH` is not set.
- `cargo check --workspace --exclude squarebob-rs --exclude media-encoder`: passed in 1m 16s.
- GitNexus query attempt: repository discovery returned only `GitNexus`; querying that repo failed with `Transport closed`, so graph evidence was not available in this environment.

## Clean / Lower-Risk Areas

- NTFS scanner unsafe blocks still have good `// SAFETY:` coverage in the Windows-specific scanner path.
- `crates/bvh-gpu/src/bvh_gpu/mod.rs` already handles readback map errors with logging after the previous plan; the remaining shared readback panic is in `render-core`.
- `crates/pt-megakernel/src/compute.rs:4421` handles PT pick map failure with `rx.recv().ok().and_then(|r| r.ok())?` instead of panicking.
- `src/app/treemap_view.rs:1293` is gated by `ui_treemap` checking `self.wgpu_render_state.is_some()` at `src/app/treemap_view.rs:31-35`; not classified as a current bug.

## Recommended Fix Order

1. Fix `render_core::gpu::map_readback` and all callers to return/handle `Result`.
2. Add checked readback size/layout arithmetic in the same helper.
3. Add missing `// SAFETY:` comments.
4. Deduplicate megakernel path-tracer initialization.
5. Centralize remaining render-state/resource-slot invariants.
6. Triage dead-code allowances in a cleanup pass.
