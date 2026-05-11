# Changelog

All notable behaviour-affecting changes to this project. Refactors that
preserve behaviour are summarised at the end of each sprint section.

Format inspired by [Keep a Changelog](https://keepachangelog.com/) but
adapted for a single-developer workflow that batches by sprint.

---

## Unreleased — sprint-5 (2026-05-11) — palettes + viz abstraction + light perf

Major day. Three orthogonal threads landed: perceptual color palettes
for materials and per-cube tint, a unified `viz` abstraction
(`CurveParams` / `RampParams` / `Mapping<P, N>`) reused across height /
color / folder-tint / effects, and an O(1) emissive light sampler that
unblocked thousands-of-lights scenes.

### Material palette system (commit `2151d04`)

Replaced the 14-bin `MaterialClass` discretisation with continuous
palette ramps so ordered sources (Size, Age, Depth) produce smooth
gradients instead of a hash-binned mosaic.

- New `crates/pt-mats/src/palette.rs`: `Palette` enum (`Viridis`,
  `Magma`, `Plasma`, `Turbo`, `Sunset`, `Cubehelix`) backed by
  Inigo-Quilez polynomial approximations / Green-2011 cubehelix /
  hand-picked diverging stops. `auto_palette_for_source` routes each
  `MaterialSource` to a sensible default.
- `MaterialLibrary` now bakes `34 + 6×256 = 1570` materials (legacy
  slots + 256-bin palette samples) and exposes `palette_material_id`.
- `classify_to_id` / `classify_path_filtered_id` return the final
  library index — light/glass overrides still resolve into the legacy
  slot range so emissive / IOR semantics survive.
- `hierarchical_path_value` accumulates per-segment hashes with 0.4
  decay so sibling files cluster into nearby palette colors.
- Scene-aware normalisation for `Depth` / `Size`: `MaterialCache`
  holds `scene_max_depth` + `scene_max_size`, set once per frame from
  a single `scan_scene_bounds` pre-walk in `collect_cubes`. Without
  this, both sources collapsed to a single bin regardless of distribute
  mode.
- New `set_scene_meta()` clears the per-path cache when bounds change
  so renders stay consistent.

### viz abstraction (commit `a51906d`)

`render-shared::viz` lifts the repeated "per-mode persistent params"
pattern into reusable primitives.

- `CurveParams { scale, exponent }` — scalar curve for Height.
- `RampParams { palette, distribution, quant_levels, band_count,
  spatial_scale, curve }` — color ramp for Color / Folder / Materials.
- `Mapping<P, const N: usize>` — indexed-by-mode persistent storage
  with custom serde impl that pads short configs and truncates long
  ones (config drift across refactors stays non-fatal).
- `EffectsState { hash_per_variant: Mapping<HashEffectParams, N> }` —
  per-`HashTransformEffect` strength + speed survives mode switches.
- Const sizes (`N_HEIGHT_MODES`, `N_COLOR_MODES`, …) compile-checked.

Egui widgets in `src/app/settings/ramp_widget.rs`:

- `curve_rows(ui, &mut CurveParams)` emits Scale + Scale Exponent rows.
- `ramp_rows(ui, &mut RampParams, RampUiCtx)` emits Palette +
  Distribute + conditional sub-params + optional curve rows.
- `ramp_section(ui, title, params, ctx)` wraps the rows in a
  collapsible `egui::CollapsingHeader` so palettes can be folded
  away once tuned.

### Height per-mode (commits `a51906d`, `4966713`, …)

- `Render3DOptions.height_scale` / `height_power` / `height_power_enabled`
  removed — switching modes used to bleed an inappropriate scale
  across (e.g. "Const" length carrying into "Size").
- New `height_curves: Mapping<CurveParams, N_HEIGHT_MODES>` indexed by
  the active `CubeHeightMode`. Compute formula:
  `(base ^ exponent) * scale * mode_const`.
- UI: mode multibutton followed by `curve_rows`. Old `^` checkbox
  replaced with explicit "Scale" / "Scale Exponent" labels aligned
  inside the standard grid.

### Color + Folder palette (commit `5ca7d45`)

- `color_ramps: Mapping<RampParams, N_COLOR_MODES>` +
  `folder_ramps: Mapping<RampParams, N_FOLDER_COLOR_MODES>`. Each
  variant stores its own palette / distribution / curve.
- `renderer3d::instance_collect` emits a scalar `t∈[0,1]` per mode
  (FileType: ext hash, FileSize: log size / log scene_max, FileAge:
  age-normalised, Treemap: `hierarchical_path_value`, Depth:
  depth / scene_max_depth) and feeds it through a shared
  `sample_color_ramp` helper: curve → distribute (Direct / Quantized
  / Gradient / Bands; Spatial falls back to path-hash wobble) →
  palette sample. Auto-routed palettes: Size→Viridis, Age→Sunset,
  Depth→Cubehelix, FileType→Plasma, Treemap→Turbo.
- Folder modes follow the same path; the legacy folder-color hash
  map is replaced.

### UI grouping (commit `4966713`, partial revert `d2e3216`)

- Geometry section split into collapsible subsections: "Height: <mode>"
  (default open), "Color: <mode>" (collapsed), "Folder tint: <mode>"
  (collapsed), "LOD" (collapsed). Each header shows the active mode.
- Color / Folder ramps nest a "Ramp" collapsing header inside their
  parent section.
- Cube placement: centered on the treemap plane (`pos.z = 0`) so the
  user can see height instead of a flat front-facing wall.
  Earlier-pass: extruding **toward** the camera put it inside tall
  cubes for big files and produced ~100× slowdown — fixed by
  centring instead of full-forward extrude.
- Animation panel restructured: dedicated `Animation` section
  (between Effects and mode-specific panels) holds the master
  `Animate` checkbox + `Speed` slider, plus an `Env` (Animate + Speed)
  row. Removed duplicates from Effects and Environment.
- Per-effect `Speed` added to `HashEffectParams` next to `Strength`,
  acts as a multiplier on `animation_speed`.

### Animation timeline correctness

- `fix(3d) cube click no longer resets PT accumulation` (`35d6773`) —
  selection only flips `selected_ids_buf`; `needs_layout = true` on
  click destroyed in-flight samples for no reason. Switched to
  `needs_render_3d = true`.
- `fix(anim) wall-clock dt anchor` (`1b9070f`) — `stable_dt` from egui
  ballooned during idle and produced visible jumps on resume.
  Replaced with a wall-clock anchor `last_anim_tick: Option<Instant>`
  on `App`; first frame after `None` produces `dt = 0`, the rest
  clamp `(now - last).min(33ms)`.
- `fix(anim) env timeline gated by master Animate` (`99135a3`) —
  Space now freezes EVERYTHING (cubes + sky). Final formula:
  ```
  if animate {
      animation_time += dt * animation_speed;
      if env_animate {
          env_time += dt * animation_speed * env_speed;
      }
  }
  ```

### Emissive light perf — O(1) sampling (commits `9d1654a`,
`e952a9f`, `420651a`)

`sample_emissive_light` scanned every light linearly to pick by
weight. With ~4500 light cubes this dragged path-tracing to a crawl.

- CPU side (`pt-megakernel::compute::build_alias_table`) constructs
  Vose's alias table at scene-upload time from per-light weights.
- New `@binding(18)` on the megakernel BGL bound to
  `emissive_alias_buf`, sized to `max(1, light_count)`.
- WGSL `pick_alias_index` does two random draws + two memory loads
  regardless of light count. Same PDF (`weight / total_weight`), so
  NEE and MIS math unchanged.
- Bumped into two WGSL reserved-word collisions during implementation
  (`alias` and `target` are both reserved); final struct field is
  named `alt`.

### Stage G.A — ReSTIR plumbing in megakernel (commit `2151d04`)

Wiring for the upcoming megakernel-side ReSTIR DI port. No behaviour
change yet — bindings declared, structs in place, stubs not called.

- BGL gained `@binding(15)` cur_reservoirs (RW), `@binding(16)`
  prev_reservoirs (RO), `@binding(17)` motion_vectors (RO).
- Two separate fallback reservoir buffers (cur + prev) so wgpu's
  exclusive-RW rule doesn't reject the dispatch when ReSTIR is off.
- `bvh_traverse.wgsl` declares `Sample` / `Reservoir` / `MotionVector`
  structs (layout mirrors `restir/reservoir.rs`) plus
  `init_reservoir` / `update_reservoir` / `combine_reservoirs` stubs.
- `max_storage_buffers_per_shader_stage` bumped 8 → 16 on both device
  creation sites (`src/main.rs`, `crates/render-core/src/lib.rs`) —
  megakernel BGL now hits 11 storage buffers.

### Known follow-ups

- Spatial distribution for Color uses a deterministic path-hash
  wobble because the cube cache key has no per-instance position.
  Real spatial coherence (Perlin / blue-noise position field) needs
  position-keyed caching.
- Age source falls back to `name_hash` as a deterministic mtime
  proxy — real `mtime` plumbing through `DirEntry` and the scanner
  is the proper fix.
- Light optimisation queue: stochastic tile-based culling and
  ReSTIR-DI in the megakernel main loop (Stage G.B from HANDOFF.md)
  still pending.

---

## Unreleased — sprint-4 (2026-05-10) — wavefront race fix + spectral parity

End-of-day fix sprint targeting the visible wavefront tile-rendering bug
the user encountered (only the bottom-right tile rendered, rest black-
with-noise) and the longstanding `spectral.rs` stub that silently fell
back to megakernel.

### Stage F.1 — Wavefront tile race fix (commit `5ff8929`)

Root cause: WebGPU/wgpu flushes ALL `queue.write_buffer` calls *before*
any encoder commands at submit time, so per-tile writes to the shared
`dims_buf` and `count_buf` collapsed to last-tile values. Result: only
the last tile saw correct state; other regions of the image got
corrupted noise / black bands.

- `crates/pt-wavefront/src/wavefront/pipeline.rs`: replaced single-slot
  `dims_buf` / `count_buf` with three N-slot persistent buffers
  (`tile_dims_buf`, `tile_counts_buf`, `count_init_src`), each padded
  to 256-byte WebGPU dynamic-offset alignment. Capacity grows on demand
  (next-power-of-two) when tile count exceeds it.
- New API:
  - `prepare_tiles(device, queue, dims, count_inits) -> bool` — writes
    ALL per-tile state via exactly one `queue.write_buffer` per buffer
    per dispatch. Returns true if a buffer reallocation happened so the
    caller can rebuild bind groups.
  - `reset_tile_count(encoder, tile_idx)` — issues
    `encoder.copy_buffer_to_buffer` from `count_init_src` into
    `tile_counts_buf` for that slot. Encoder-ordered so dispatches see
    fresh counts (this is what fixes the race for count_in / count_out).
  - `tile_offset(idx) -> u32` — dynamic-offset byte index per slot.
  - `pack_tile_slots<T: Pod>` — pure helper for stride-aligned blob
    packing, unit-tested.
- Bind group layouts for dims (binding 1, raygen) and counts
  (bindings 3 / 4 / 6 / 0 across raygen / intersect / shade /
  count_swap) declare `has_dynamic_offset: true` with `min_binding_size`
  set to actual struct size; bind groups now use `BufferBinding{ offset:
  0, size: slot_size }` instead of `as_entire_binding`, so the dynamic
  offset selects exactly one slot's view.
- WGSL shaders **unchanged** — dynamic offset is transparent at the
  shader binding level.

In `compute.rs::dispatch_wavefront`:
- Pre-collects `Vec<WfDims>` + `Vec<[u32;4]>` for all tiles in a small
  pass before encoding, hands off to `prepare_tiles` once.
- If `prepare_tiles` reports a reallocation, `rebuild_wavefront_bind_groups`.
- Per tile: `reset_tile_count` (encoder-ordered) +
  `pass.set_bind_group(0, bg, &[tile_off, ...])` for the dynamic-offset
  slots. **No `queue.write_buffer` in the tile loop body.**
- Removed `wf.write_dims` and `wf.count_buf` accessors.

### Stage F.2 — Spectral PT actually runs in wavefront (commit `407ff73`)

`crates/render-3d/src/pt/spectral.rs` used to forcibly set
`pt_wavefront = false` and warn `Spectral backend stub: forcing
megakernel path`, hiding the fact that wavefront's `shade.wgsl` already
applies `spectral_tint` at sky-miss and emission events.

- Dropped the forced megakernel fallback; the dispatcher just
  normalises `pt_spectral_samples` (>=1) and routes through the user's
  selected backend.
- `crates/pt-wavefront/src/wavefront/shade.wgsl`: also applies
  `spectral_tint` to the transmission throughput (parity with
  megakernel's `compute.rs` spectral usage). Combined with the existing
  IOR-based dispersion `trans_tint`, gives wavelength-aware transmission
  tinting when `spectral_mode != Off`; when `Off` the helper returns
  `(1, 1, 1)` so the multiply is a no-op.

### Stage F.3 — Tile-size input safety (commit `ddbdd26`)

Typing a multi-digit tile size (e.g. "256") in the UI with rendering
active triggered a transient pass with `tile_size = 2`, producing
~520k tiles on FullHD and hanging the GPU command queue / staging
buffer allocator. Fixed with three layers:

1. `PathTraceCompute::set_wavefront_tile_size` clamps any non-zero
   value to >= 64 (with a debug log).
2. `WavefrontPipeline::prepare_tiles` asserts tile count <= 4096 (with
   the >=64 size clamp, FullHD produces at most 30 × 17 = 510 tiles).
3. The settings UI snaps the entered value to {0, >=64} on
   `.changed()` so the user sees the effective value immediately;
   helper text updated to "0 = full frame, min 64".

### Stage F.5 — Build fix (commit `b6e84e9`)

The prior WIP commit had renamed unused-on-Linux let-bindings to
`_path` / `_max_diag` / `_max_lp` / `_n` in `src/cli_test.rs`, but the
Windows-only `#[cfg(windows)]` arms still referenced them as
`path` / `max_diag` / `max_lp` / `n` — and the parser sees `path` as
the built-in `#[path]` attribute, not a value. Two related issues in
`src/scanner_ntfs.rs` (missing `use dirstat_core::DirEntry`) and
`src/app/scan_orchestration.rs` (`_path` parameter referenced as
`path` in body) had the same pattern. Fixed by moving the let-bindings
inside the `#[cfg(windows)]` arms (or restoring the parameter names).

### Tests added (commit `76c28f5`)

`crates/pt-wavefront/src/wavefront/pipeline.rs` gained six unit tests
covering the dynamic-offset slot layout invariants:
`TILE_SLOT_STRIDE == 256`, `WfDims` size match, `WF_COUNTS_SIZE` size,
`pack_tile_slots` layout / empty / round-trip cases. (Three of the
const-only ones were later folded into compile-time `const _: () =
assert!(...)` in Stage F.7 below.)

### Stage F.4 — ReSTIR/PathGuide/Adaptive coexist with tiling

All five advanced wavefront subsystems are now tile-safe; the force-
disable warnings in `compute.rs::dispatch_wavefront` are gone.

- **Adaptive sampling** (commit `43e9376`) — already tile-safe by
  construction (variance + allocate run once per frame on the full image
  *after* the tile loop). Just lifted the force-disable + warn.
- **F.4-A PathGuide sample** (commit `6ef6aac`) — `gid.x` is remapped
  from the tile-pixel range to a global pixel index so the per-pixel
  `guide` buffer (full-image sized) no longer aliases between tiles.
  `update.wgsl` is `workgroup_size(1)` and was always tile-safe.
- **F.4-B..F gbuffer + 4 ReSTIR shaders** (commit `0bec861`) — five
  WGSL kernels (`wavefront/gbuffer.wgsl`,
  `restir/{initial,temporal,spatial,shade}.wgsl`) now distinguish
  `local_id` (`gid.y * tile_w + gid.x`) for tile-sized rays/hits
  buffers from `pixel_id` (`gy * full_w + gx`) for full-image buffers
  (reservoirs, depth/normal/motion, sample_map, output). RNG seeding
  uses the global pixel_id so accumulation stays reproducible across
  tile boundaries. Motion-vector reprojection and ReSTIR spatial
  neighbor sampling switched to full-image coords.

Host plumbing (`compute.rs`, `restir/pipeline.rs`, `pathguide/
pipeline.rs`):

- Five subsystem params bindings (gbuffer@5, restir initial@2 /
  temporal@5 / spatial@4 / shade@3, pathguide sample@2) now use
  `has_dynamic_offset=true` with `min_binding_size` set to the WGSL
  struct size. Size constants exposed as `GBUFFER_PARAMS_SIZE=160`,
  `RESTIR_INITIAL_PARAMS_SIZE=32`, `RESTIR_TEMPORAL/SPATIAL/SHADE
  _PARAMS_SIZE=48`, `PG_SAMPLE_PARAMS_SIZE=96`.
- Each subsystem's params buffer is fixed-size at
  `MAX_TILE_CAPACITY * TILE_SLOT_STRIDE` (~1 MB per buffer, ~5 MB
  total). No bind-group rebuild when tile count changes.
- Per-tile params are packed once at the start of `dispatch_wavefront`
  via `pack_tile_slots(&Vec<T>)` (re-exported from `pt-wavefront`) and
  uploaded with a single `queue.write_buffer` per buffer. The per-tile
  dispatch sets dynamic offset = `tile_idx * TILE_SLOT_STRIDE`. This
  fixes the same queue-flush race that previously left only the last
  tile's values visible to all dispatches.
- Removed the per-tile struct construction + `queue.write_buffer` for
  RestirInitial/Temporal/Spatial/Shade params from the dispatch loop.
- Pub-exported `MAX_TILE_CAPACITY`, `DEFAULT_TILE_CAPACITY`, and
  `pack_tile_slots` from `pt-wavefront` so downstream crates can reuse
  the per-tile packing pattern.

**Bonus fix — ReSTIR motion vectors (commits `2767548`, `b312afc`):**
`prev_view_proj == curr_view_proj` because the matrix cache only
retained the latest frame; ReSTIR temporal reuse saw zero motion.
`PathTraceCompute` now keeps a `prev_view_proj` field; both renderer
entry points (`megakernel/render.rs`, `megakernel/render_no_readback.rs`)
roll the prior `last_view_proj` into `prev_view_proj` every frame
(unconditional, not gated on `cam_moved`) so a static-camera frame
after motion has a coherent prev/curr pair rather than a stale matrix
from an earlier session. First frame falls back to `prev = curr` =
zero motion (matching prior behaviour).

### Stage F.7 — Clippy cleanup (commit `b312afc`)

The unit-test module in `crates/pt-wavefront/src/wavefront/pipeline.rs`
sat in the middle of the file (before `create_finalize_pipeline`) and
contained three pure const-vs-const `assert!` invariants. Cleanup:

- Moved `mod tests` to the end of the file (clears `clippy::
  items_after_test_module`).
- Replaced the redundant runtime tests `tile_slot_stride_is_256`,
  `wf_dims_size_matches`, `wf_counts_size_matches` with compile-time
  `const _: () = assert!(...)` next to the constant declarations
  (clears `clippy::assertions_on_constants`, also strengthens the
  contract: failures become build errors, not test failures).
- The three real runtime tests (`pack_tile_slots_layout/empty/wf_dims`)
  stay; workspace test count: 38 → 35 (3 const-only tests folded into
  compile-time asserts).

Final workspace state: `cargo clippy --workspace --all-targets` zero
warnings, `cargo test --workspace` 21 test sets, 0 failures.

---

## Unreleased — sprint-3 (2026-05-09) — denoiser + monolith reduction

End-of-day rolling sprint added the PT denoiser (Stage D.2) and a
substantial modularization pass on the largest remaining monoliths.

### Stage D.2 — PT à-trous denoiser (Dammertz et al. 2010)

Full end-to-end implementation, ready for visual tuning by the user.

- New module `crates/pt-megakernel/src/denoiser/` with
  `atrous.wgsl` (compute kernel, color-only edge stop, 5x5 cubic
  B-spline at increasing stride) and `pipeline.rs` (DenoiserPipeline
  with two ping-pong Rgba32Float textures).
- `PathTraceCompute` integration: `set_denoise_enabled`,
  `set_denoise_options`, `apply_denoiser` (called between dispatch
  and blit; rewires `blit_bind_group` to read denoised texture).
- `Render3DOptions`: `pt_denoise_enabled`, `pt_denoise_iterations`,
  `pt_denoise_sigma_color`. CLI: `--pt-denoise / --no-pt-denoise`,
  `--pt-denoise-iterations N`, `--pt-denoise-sigma-color F`.
- New Settings tab "Denoise" (`src/app/settings/denoiser.rs`) with
  enable toggle, iterations slider, color sigma slider, and four
  preset buttons (Conservative / Balanced / Aggressive / Off).

MVP scope: color-only edge stopping. G-buffer guidance (normal/depth)
deferred — the wavefront PT already produces a G-buffer for ReSTIR
(`pt-megakernel/src/wavefront/gbuffer.wgsl`); plumbing it into the
à-trous kernel is a 1-2 commit follow-up.

### Modularization — large monoliths split

Per the user's "и модуляризируй большие монолиты" directive:

- **`src/main.rs`: 1102 → 159 LOC.** All CLI parsing
  (CliOptions struct, Default impl, parse_args, print_help,
  parse_height_mode, parse_color_mode, parse_hash_effect,
  parse_hover_mode, parse_materialize_mode, parse_spectral_mode)
  moved to a new `src/cli.rs` (954 LOC). main.rs now contains
  only `mod` declarations + `pub use cli::CliOptions` (so existing
  `crate::CliOptions` references in `app/cli_apply.rs` keep
  working) + `fn main()`.
- **`crates/render-3d/src/pt/megakernel.rs`: 1073 LOC → 3 files.**
  Was a single file with two large render orchestrators
  (`render_path_traced_no_readback` ~478 LOC, `render_path_traced`
  ~575 LOC) plus 7 LOC of `frame_count`/`pick`. Now:
    pt/megakernel/mod.rs                  26 LOC (imports + helpers + re-exports)
    pt/megakernel/render.rs              579 LOC
    pt/megakernel/render_no_readback.rs  483 LOC
  Submodules use `use super::*` to inherit the parent imports.
- **`crates/render-3d/src/lib.rs`: 1937 → 1797 LOC.** Eight
  free helper functions (`lerp`, `lerp4`, `hash_f32`, `mix_material`,
  `kelvin_to_rgb`, `apply_glass_controls`, `compute_slice_normal`,
  `compute_slice_position`) extracted to a new
  `renderer3d/helpers.rs` (150 LOC). They were only in `lib.rs`
  because the file used to be a 2335-LOC god-object before B.1.

### Out of scope for sprint-3

- **`crates/pt-megakernel/src/compute.rs` (3722 LOC) untouched.**
  Splitting the PathTraceCompute orchestrator into per-subsystem
  integration files is mechanically possible but high-risk without
  runtime verification — every method touches many private fields,
  and a silent breakage in dispatch_megakernel/dispatch_wavefront is
  visually invisible until path-traced output corrupts. Defer until
  there's appetite for runtime+visual UAT.
- **`src/scanner_ntfs.rs` (973 LOC)** is single-concern Win32 FFI
  for FSCTL_ENUM_USN_DATA — splitting harms cohesion. Leave.

### Verification

Each modularization commit ran:
  cargo build --workspace --all-targets       — ok
  cargo clippy --workspace --all-targets -- -D warnings  — 0 warnings
  cargo test --workspace                      — 24 unit tests pass

### E.3 — gitnexus embeddings — BLOCKED by environment

Tried `npx gitnexus analyze --embeddings --force`. The command exits
"successfully" (exit 0) but `embeddings: 0` afterwards because two
native-binary ABI conflicts surface on this WSL2 / conda-forge stack:

1. **ONNX runtime segfault**: `@huggingface/transformers`'s
   `onnxruntime-node` ships a `.node` napi-v6 binding compiled against
   a Node ABI incompatible with the Bun runtime that `bunx`/`npx`
   resolves to on this machine. Loading the binding causes a
   `panic(main thread): Segmentation fault at address 0x0`.

2. **Kùzu VECTOR extension undefined symbol**:
   `~/.lbdb/extension/0.15.0/linux_amd64/vector/libvector.lbug_extension`
   fails to load with `undefined symbol: _ZTIN4lbug7catalog12IndexAuxInfoE`
   — a C++ name-mangling mismatch between the shipped extension and
   the runtime's libstdc++.

Same category of issue as the GCC 15 / `mimalloc` ATOMIC_VAR_INIT
problem documented in CLAUDE.md: shipped binary artifacts assume an
ABI different from this machine's toolchain.

Workarounds (not applied — defer to user):
- Run gitnexus via plain `node` instead of `bunx`/`npx` if a non-Bun
  path can be forced.
- Upgrade gitnexus to a version where the extensions are recompiled
  against current GCC.
- Run the embedding step in Docker with a known-good toolchain.

Pragmatic: BM25-only ranking via `gitnexus_query` works very well on
this Rust codebase (expressive symbol names). The semantic embedding
upgrade is nice-to-have, not blocking.

### Stage D.2 (originally — sprint-3 part 1, kept here for completeness)



Single-thread post-sprint-2 batch. Closed Stage D.1 (zero-copy 2D-GPU
display) and refreshed all .planning/ docs + AGENTS.md to match the
post-sprint-2 codebase. New CHANGELOG.md, ~/.claude/CLAUDE.md
augmented with cross-project insights.

### Added
- `crates/treemap/src/wgpu.rs::GpuRenderer2D::render_to_texture(&mut self, ...) -> bool`
  — renders into the internal `render_texture` with no CPU readback.
- `crates/treemap/src/wgpu.rs::GpuRenderer2D::get_render_texture(&self) -> Option<&wgpu::Texture>`
  — borrows the rendered texture for egui registration.
- `src/app/treemap_view.rs::render_2d_callback` — mirrors
  `render_3d_callback` for the 2D-GPU zero-copy display path.
- `CHANGELOG.md` (this file).

### Changed
- `GpuRenderer2D` render-target texture usage now includes
  `TEXTURE_BINDING` so egui can sample it without a CPU round-trip.
- `treemap_view.rs::ui_treemap_pane` — `use_callback` extended to
  fire on Mode2D + Backend::Gpu (in addition to existing Mode3D),
  selecting between `render_2d_callback` and `render_3d_callback`.
- `GpuRenderer2D::render` (the legacy CPU-readback API) now
  delegates to `render_to_texture` + a separate readback encoder
  (two submits on the fallback path; readback dominates timing
  anyway).
- The two `TODO` markers in `src/app/mod.rs` (per CONCERNS.md the
  only `TODO` markers in source) replaced with accurate comments
  describing why this is now the CPU-readback fallback, not the
  primary path.
- `CONCERNS.md`, `STRUCTURE.md`, `TESTING.md`, `ARCHITECTURE.md`,
  `AGENTS.md` — sprint-2 state captured. Originals preserved as
  historical record where useful.

### Removed
- 4 blanket `#![allow(dead_code)]` belts in the PT pipeline.rs files
  (Stage C.3 audit found nothing was actually dead).

### Architectural prep for deferred Stage D.2 (denoiser)
- The `register_native_texture` infrastructure now used by both 3D
  and 2D-GPU paths is the integration point for the denoiser's
  output: PT pipeline gets a `get_denoised_texture() -> Option<&wgpu::Texture>`
  accessor; treemap_view registers it with egui via the existing
  `render_texture_id`. No new display path needed.

### Verified locally (2026-05-10: GCC 13 in conda env, no PATH workaround needed)
- `cargo build --workspace --all-targets` — ok in ~3-5s warm.
- `cargo clippy --workspace --all-targets -- -D warnings` — 0 warnings.
- `cargo test --workspace` — 24 unit tests passing.

### Open after sprint-3
- Stage 0.1 manual UAT (runtime: slider toggle vs `instance_rebuild_count()`).
- Stage A.1 visual diff (runtime: animate ON × PT ON × materialize FPS).
- Stage C.6 PT backend canonical-vs-fast-path policy (decision).
- Stage D.2 denoiser (deferred per user; architecturally prepared).
- Stage D.3 BVH refit runtime trace (runtime).
- Stage D.4 `auto-allocator secure` benchmark (runtime).
- Stage E.3 `npx gitnexus analyze --embeddings` (one user command).

---

## Unreleased — sprint-2 (2026-05-09)

Multi-agent + main-thread sprint. 16 commits on `main`. Closed 9 of the
original top-10 audit concerns (`CONCERNS.md`) and shipped CI plus
build/test verification.

### Added
- `.github/workflows/ci.yml` — Linux + Windows matrix CI:
  `cargo build --workspace --all-targets`,
  `cargo clippy --workspace --all-targets -- -D warnings`,
  `cargo test --workspace`. Uses `Swatinem/rust-cache@v2`. Plus
  `rustsec/audit-check@v2` on every push and weekly cron — this is the
  audit job that justifies keeping `auto-allocator = "*"` unpinned.
- `Renderer3D::instance_rebuild_count() -> u64` — public accessor for
  Stage A.1 verification: confirms that toggling `materialize_mix`
  (a shader-side uniform) does NOT trigger a CPU instance rebuild.
- `crates/render-3d/src/renderer3d/material_cache.rs` (123 LOC) —
  extracted `MaterialCache`, `MatGlobalUniform`, `mat_settings_hash`,
  `settings_from_opts` from the lib.rs god-object.
- `crates/render-3d/src/renderer3d/instance_collect.rs` (300 LOC) —
  extracted `Renderer3D::collect_cubes` and `collect_recursive`.
- `src/app/scan_orchestration.rs` — `start_scan`, `stop_scan`,
  `poll_scan`, `scan_engine_label_for_mode`.
- `src/app/render_loop.rs` — `run_frame`, `handle_events`,
  `sync_dock_tabs_visibility`.
- `src/app/screenshot.rs` — `handle_screenshot`, `capture_viewport`,
  `save_png`.
- `src/app/cli_apply.rs` (443 LOC) — single-source-of-truth applicator
  `apply_cli_overrides(&mut Render3DOptions, &CliOptions)` plus 2 unit
  tests verifying every CLI knob lands in the expected field.
- `src/app/shell.rs::shell_open()` — wrapper around `open::that` that
  logs failures via `log::warn!` instead of silently dropping them.
- 14 new unit tests across `pt-mats::tests` (9), `treemap::tests` (5),
  `app::cli_apply::tests` (incl. `none_flags_leave_existing_values_intact`).
- 16 `// SAFETY:` comments documenting the buffer-size,
  HSTRING-ownership, and handle-lifetime invariants of every Win32
  FFI block in `src/scanner_ntfs.rs`.
- `debug_assert!(rects_disjoint(&rects))` before the
  `par_iter().for_each` parallel-fill path in `crates/treemap/src/lib.rs`,
  with a `#[allow(dead_code)]` `rects_disjoint` helper.

### Changed
- **NTFS fallback bug fix**: `ScanMsg::NtfsFallback` handler in
  `src/app/mod.rs` no longer mutates `self.scanner_mode = Standard`.
  That mutation persisted into `PersistState` on next save, silently
  stripping the user's NTFS preference. Existing UI feedback via
  `progress.error` and `progress.scan_engine_label` retained.
- **GPU adapter failure path**: `crates/render-core/src/lib.rs::GpuContext::new()`
  now logs adapter and device failures via `log::error!` instead of
  silently propagating `None`. `log` added to `render-core/Cargo.toml`.
- **Lazy-init diagnostics**: 17 `.as_ref().unwrap()` sites in
  `crates/render-3d/src/lib.rs` and `pt/megakernel.rs` upgraded to
  `.as_ref().expect("<diagnostic>")`. (Stage B.2 typestate refactor
  was disqualified by lifecycle analysis — `cached_instances` and
  `instance_buffer` build per-frame; `targets` and `dyn_bgs` build
  in resize/init; the env-map-change path needs `targets=Some`
  + `dyn_bgs=None` simultaneously, breaking single-substruct
  invariant. Documented in TODO4.md and CONCERNS.md.)
- `crates/render-3d/src/lib.rs` size: **2335 → 1937 LOC** after the
  Stage B.1 extractions.
- `src/app/mod.rs` size: **1521 → 716 LOC** after Stage B.3 + B.4.
- `src/app/cli_apply.rs::tests` flag-mapping test: replaced ~31
  `assert_eq!(opts.x, true)` with `assert!(opts.x)` per
  `clippy::bool_assert_comparison`.

### Removed
- `task.md`, `TODO.md`, `TODO2.md`, `TODO3.md`, `plan1.md` —
  consolidated into `TODO4.md` (commit `398f566`, sprint-1).
- 4 blanket `#![allow(dead_code)]` belts in
  `crates/pt-megakernel/src/{pathguide,adaptive,restir}/pipeline.rs`
  and `crates/pt-wavefront/src/wavefront/pipeline.rs`. Removing the
  blankets surfaced **zero** dead-code warnings — every symbol is
  used. Allows were over-cautious historical guards from early PT
  scaffolding.

### Fixed
- 5 silent `let _ = open::that(...)` failures across `shell.rs`,
  `treemap_view.rs`, `mod.rs` — now route through `shell::shell_open()`.
- 4 unnecessary `as u64` casts in `src/app/helpers.rs::statvfs` path
  (auto-fixed by `cargo clippy --fix`).
- Treemap squarified-layout test: switched
  `let mut opts = TreeMapOptions::default(); opts.style = ...`
  to struct-update syntax to satisfy
  `clippy::field_reassign_with_default`.
- 3 `cfg(not(windows))` API-parity stubs in `scanner_ntfs.rs` annotated
  with `#[allow(dead_code)] // API-parity stub`.

### Verified, no code change needed
- **UI raw-pointer aliasing** (CONCERNS top-7): all 7
  `unsafe { &*ptr }` sites already carry `// Safety:` comments and
  follow the disciplined `&mut self`-scoped capture-and-deref pattern.
  CONCERNS' UAF concern requires a concurrent thread mutating
  `self.tree`, which is impossible under exclusive `&mut self` borrow.
- **GPU adapter `Option` propagation** (CONCERNS top-N): zero unwrap
  on `gpu_context` workspace-wide. All consumers use
  `.is_some()`/`.is_none()` checks.
- **`pt-megakernel → pt-wavefront` dep direction**: intentional
  orchestrator pattern (single import in `compute.rs:16`). Not
  "wrong direction" as suspected.
- **BVH refit fast-path**: `can_refit()` and `refit()` exist in
  `crates/bvh-gpu/src/bvh_gpu/mod.rs:329, :378`. Gated by
  `opts.pt_gpu_bvh && opts.pt_bvh_refit` at
  `crates/render-3d/src/pt/megakernel.rs:205, :692`. Falls back to
  full rebuild if `can_refit()` returns false. Runtime trace
  verification remains user work.

### Open / requires user attention
- Stage 0.1 manual UAT: slider toggle vs `instance_rebuild_count()`.
- Stage A.1 visual diff: animate ON × PT ON × materialize {None, On}
  FPS measurement.
- Stage C.6 PT backend canonical-vs-fast-path policy decision.
- Stage D.1 zero-copy treemap upload (the only two `TODO` markers in
  source: `src/app/mod.rs:1035, :1068`).
- Stage D.2 PT denoiser — **deferred per user; preserve G-buffer
  extension points when touching PT pipeline so it can land later
  without a rewrite**.
- Stage D.3 BVH refit runtime trace.
- Stage D.4 `auto-allocator secure` benchmark.
- Stage E.3 gitnexus embeddings.

### Local-environment footnote (not a project bug)
- `auto-allocator-0.1.0/build.rs::has_stdatomic_header()` test
  program uses `ATOMIC_VAR_INIT(0)` (deprecated in C17, removed in
  C23). Conda-forge GCC 15.1 defaults to C23 → test fails →
  build.rs incorrectly concludes "stdatomic.h unavailable". This is
  an upstream bug in `auto-allocator`, not this project.
  **Resolved 2026-05-10**: `conda install -c conda-forge gcc=13 gxx=13`
  in the local env. GCC 13.4 defaults to gnu17, `ATOMIC_VAR_INIT`
  works, mimalloc-sys compiles cleanly, plain `cargo build` works.
  CI runners on Linux + Windows were unaffected to begin with.

---

## Unreleased — sprint-1 (2026-05-09)

First batch of code-only quality work, ~12 commits, single-thread.

### Added
- TODO4.md (rev 1 → rev 4) — validated roadmap that supersedes the
  earlier docs and corrects several factual errors that had
  cascaded through plan1 → CONCERNS → TODO3.

### Changed
- Material migration (Stage A): completed. Steps 1–8 shipped
  per-instance materials via `material_id` slot 9, GPU
  `materials_buf` storage + `mat_global` UBO, `cube_pbr.wgsl`
  doing the `materialize_mix` blend in shader, CPU `color_f`
  blend dropped. Step 9 verification (FPS measurement, slider
  no-rebuild assertion) deferred to user.

### Verified discoveries
- `DirEntry::lod_expand` is a **field** (`Option<LodExpandInfo>`),
  not a method/function. plan1.md / CONCERNS.md / TODO3.md all
  treated it as testable code; the actual LoD-merge logic
  (`merge_tree_by_size_range`) lives in `src/app/filters.rs:212/258`
  and was already covered by 3 tests there.
- TODO3.md status snapshot was inaccurate in two material ways:
  - Claimed Stage A was "Step 0–3 partial" — actually Steps 1–8
    were already in code; only Step 9 verification remained.
  - Claimed `fix/remove-ui-raw-pointers` branch had been merged —
    branch did not exist; `display_root_of` did not exist;
    all 7 `unsafe { &*ptr }` sites were still in source. Re-evaluation
    showed they're correct as written (see sprint-2 entry above).

---

*Maintained by hand. Each sprint = one section. Behaviour-affecting
items go to ### Added / ### Changed / ### Removed / ### Fixed.
Refactors that don't change behaviour go in the prose summary.*
