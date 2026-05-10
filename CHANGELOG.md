# Changelog

All notable behaviour-affecting changes to this project. Refactors that
preserve behaviour are summarised at the end of each sprint section.

Format inspired by [Keep a Changelog](https://keepachangelog.com/) but
adapted for a single-developer workflow that batches by sprint.

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
