# TODO4 — Validated Roadmap

**Date:** 2026-05-09
**Supersedes:** `TODO.md`, `TODO2.md`, `TODO3.md`, `plan1.md`, `task.md` (deleted in
the same commit).
**Validation method:** every "already done" claim cross-checked against the
current source tree; line numbers re-verified; gitnexus impact run on key
seams (`classify_path_filtered`, `collect_recursive`, `lod_expand`).

This document replaces TODO3 because TODO3's status snapshot diverges from
reality in two material ways:

1. **TODO3 claims the unified material migration (Stage A) is "Step 0–3
   partial, Step 4+ pending."** Validation shows **Steps 3, 4, 5, 6, 7 are
   already implemented in code.** Only verification (Step 9) remains; the
   "Step 8 cleanup" target (`MaterialParamsUniform`, legacy `material_buf`)
   is already absent from the codebase.
2. **TODO3 claims "branch `fix/remove-ui-raw-pointers` removed all 7 unsafe
   `&*ptr` sites via `display_root_of`."** Validation: that branch does not
   exist (only `main` locally and on origin), `display_root_of` does not
   exist anywhere in the source, and all 7 raw-pointer dereferences are
   present at the exact lines CONCERNS.md flagged them. **The fix was
   planned but never implemented.**

The corrected view drives the new ordering below.

---

## Validated state — what is actually done

Cross-checked against `crates/`, `src/`, branches, and gitnexus.

### PT quality (`TODO.md`)
- **DONE.** NEE for emissive cubes, full MIS, ReSTIR DI (wavefront-only),
  R2 low-discrepancy sampling, adaptive sampling polish, firefly per-sample
  clamp, environment MIS PDF audit, material lobe PDF audit, wavefront
  parity scope marking. **Open:** denoiser only.

### Unified material migration (`TODO2.md`)
- ✅ **Step 1** — `material_id: u32` field in `CubeInstance`, vertex slot
  9 = `Uint32` (`crates/render-3d/src/geometry.rs:14, 44`).
- ✅ **Step 2** — `MaterialCache { settings_hash, classes_pbr, classes_pt }`
  + `mat_settings_hash` + `settings_from_opts` at
  `crates/render-3d/src/lib.rs:101–193`. `classify_or_get` lives at
  `lib.rs:121` (signature drifted slightly from the plan: it takes
  `&Path, size: u64, opts, is_pt`, not `&DirEntry`).
- ✅ **Step 3** — PBR `collect_recursive` calls
  `self.mat_cache.classify_or_get(...)` at `lib.rs:1364`. Top-level entry
  primes the cache via `self.mat_cache.ensure(opts)` at `lib.rs:1102`.
- ✅ **Step 4** — PT scene build calls `renderer.mat_cache.classify_or_get(
  path, size, opts, true)` at `crates/render-3d/src/pt/megakernel.rs:140`
  (first instance build) and `:620` (second instance build). Both preceded
  by `renderer.mat_cache.ensure(opts)` at `:120` and `:600`.
- ✅ **Step 5** — `materials_buf: wgpu::Buffer` and
  `mat_global_buf: wgpu::Buffer` are real fields on `Renderer3D`
  (`lib.rs:214, 215`), created at `:490, :495`, bound in `pbr_bg0` at
  `:539, :543`.
- ✅ **Step 6** — `crates/render-3d/shaders/cube_pbr.wgsl` has
  `struct GpuMaterial { ... }` (line 44),
  `@binding(2) var<storage, read> materials: array<GpuMaterial>` (line 69),
  `@binding(3) var<uniform> mat_global: MatGlobal` (line 70), per-instance
  `material_id: u32 @location(9)` flowing through `VertexOutput` flat to
  fragment, `resolve_material` doing
  `mix(instance_color, m.base_color_weight.rgb, mat_global.materialize_mix)`
  used in `fs_main`, `fs_gbuffer`, `fs_wireframe`.
- ✅ **Step 7** — CPU `color_f` material blend removed.
  `lib.rs:1360` carries the comment "The shader handles albedo blending via
  `mat_global.materialize_mix`" before the `mat_cache.classify_or_get` call.
- ✅ **Step 8** — `MaterialParamsUniform` and the legacy
  `material_buf: wgpu::Buffer` are gone (grep returns zero hits in
  `crates/` and `src/`).
- ⏳ **Step 9** — verification pending: animate-on × PT-on × materialize
  toggle FPS measurement, slider-no-rebuild assertion via debug counter,
  visual diff vs pre-migration. **This is the only remaining migration
  work** and it's quick.

**gitnexus invariant check:** `classify_path_filtered` has exactly one
direct upstream caller (`MaterialCache.classify_or_get`) with LOW risk —
the "single classify call site" architectural goal of Stage A is held in
code.

### Bug-hunt deferrals (`plan1.md`)
- ⏳ **D1 NTFS fallback** — open. `src/app/mod.rs:619–623` still does
  `self.scanner_mode = ScannerMode::Standard` on `ScanMsg::NtfsFallback`,
  which persists. `ntfs_last_error` field does not exist.
- ⏳ **D2 `auto-allocator = "*"`** — open. `Cargo.toml:23` still wildcard
  with `secure` feature. (Decision: keep wildcard per architecture
  watchpoint; benchmark `secure` feature in Stage D.4.)
- ⏳ **D3 split `app/mod.rs`** — open. Still 1518 LOC.
- ⏳ **D4 single CLI applicator** — open.
- ⏳ **D5 zero-copy treemap** — open. The two `TODO` markers at
  `src/app/mod.rs:1035` and `:1068` are still there with original wording.

### Visual regression (`task.md` — cube gaps)
- ✅ Fixed 2026-05-09. `task.md` is gone from the working tree.

### UI raw-pointer aliasing (CONCERNS top-7)
- ❌ **NOT DONE.** All 7 sites still present:
  `src/app/tree_panel.rs:115, 219, 226`,
  `src/app/mod.rs:1186, 1197, 1222`,
  `src/app/treemap_view.rs:978`. `display_root_of` does not exist;
  `fix/remove-ui-raw-pointers` branch does not exist. **Promoted from
  TODO3's false-completed list to active high-priority work in Stage 0
  below**, because the audit (`CONCERNS.md` "Aliased raw pointers in UI")
  flagged this as **the highest-risk unsafe class in the codebase — more
  than the Win32 FFI** (use-after-free if the tree is dropped/rebuilt
  while a UI panel holds the raw pointer).

### Clippy hygiene (`plan1.md` "Implemented fixes")
- ✅ DONE. `is_some_and`, `clamp(1, MAX_…)`, struct-literal in tests,
  removed unused bindings — visible in current code.

---

## Verified architectural invariants

These are checked, not claimed. Future work must keep them.

| Invariant | How verified |
|-----------|--------------|
| `classify_path_filtered` has exactly 1 direct caller (`MaterialCache.classify_or_get`) | gitnexus impact upstream depth=2: 4 total impacted, 1 direct. |
| `collect_recursive` has 1 direct caller (`collect_cubes`) | gitnexus impact upstream depth=2: LOW risk. Mechanical extraction safe. |
| `MaterialParamsUniform` and `material_buf` are gone | grep in `crates/` and `src/`: 0 hits. |
| `cube_pbr.wgsl` reads `materials[material_id]` per fragment | direct WGSL read at lines 44, 69–70, 257–259. |
| 16 `unsafe` blocks in `scanner_ntfs.rs`, no SAFETY comments | grep `unsafe ` count = 16; no `// SAFETY:` matches. |
| 4 `#![allow(dead_code)]` belts in `pt-megakernel`/`pt-wavefront` | exact lines unchanged from CONCERNS.md. |

### File LOC (unchanged since 2026-05-09 audit)

| File | LOC | Status |
|------|----:|--------|
| `crates/render-3d/src/lib.rs` | 2309 | Stage B.1 target — extract `MaterialCache` + `collect_recursive`. |
| `src/app/mod.rs` | 1518 | Stage B.3 target — split scan/render/screenshot. |
| `src/main.rs` | 1077 | Stage B.4 target — CLI applicator. |
| `crates/render-3d/src/pt/megakernel.rs` | 1067 | Active churn; second-largest. |
| `src/scanner_ntfs.rs` | 936 | Stage C.4 target — SAFETY annotations. |
| `crates/treemap/src/lib.rs` | 823 | Stage C.5 target — `unsafe` parallel write at `:498–520`. |

---

## Stage 0 — Safety net (do first)

### 0.1 Minimum-viable CI
- Add `.github/workflows/ci.yml` running `cargo build --workspace`,
  `cargo clippy --workspace --all-targets -- -D warnings`,
  `cargo test --workspace`.
- Linux runner only at first. Add Windows once `scanner_ntfs` has tests.
- Cache target dir via `Swatinem/rust-cache@v2`.

### 0.2 Pure-CPU tests for SSOT functions
- `dirstat-core::DirEntry::lod_expand` — gitnexus shows 0 callers in the
  call graph (it's classified as a Property, not a Function), so the
  compiler-level safety net is *zero*. 2–3 unit tests covering merge
  thresholds are the only way to lock semantics.
- `pt-mats::classify_path_filtered` — table-driven over `materialize_mode`,
  `mat_allow_lights`, `mat_allow_glass`. The function has exactly one
  caller (`MaterialCache`), so any behavior change is silent until users
  see a visual diff.
- `treemap` squarified-layout — input tree → expected rectangle set. This
  test would have caught the recently-fixed "cube gaps" regression in CI.
- **Note:** `crates/render-shared`, `src/app/filters.rs`, and
  `crates/bvh-gpu/src/bvh_gpu/mod.rs` already have tests (3 files total —
  `plan1.md` only mentioned `render-shared`).

### 0.3 NTFS fallback no longer rewrites user prefs (`plan1.md` D1)
- Add `ntfs_last_error: Option<String>` (transient, not persisted) to
  `App`.
- In `ScanMsg::NtfsFallback` handler at `src/app/mod.rs:619–623`: set the
  transient error, **do not** mutate `scanner_mode`.
- Add a non-modal banner in the UI when `ntfs_last_error.is_some()`.
- Manual QA on Windows.

### 0.4 Remove UI raw-pointer aliasing (CRITICAL — was falsely marked done)
**This is in Stage 0, not Stage C, because the audit ranked it as the
highest-risk `unsafe` class in the codebase.** TODO3 marked it complete on
a fictional branch.

- 7 sites of `let root = unsafe { &*root_ptr };` to fix:
  - `src/app/tree_panel.rs:111` (stores), `:115, 219, 226` (deref).
  - `src/app/mod.rs:1181, 1190, 1216` (stores), `:1186, 1197, 1222`
    (deref).
  - `src/app/treemap_view.rs:978` (deref).
- **Approach options** (decision needed before implementation):
  1. `Arc<DirEntry>` — clone-on-need, simple, costs one Arc per
     borrow site.
  2. Take-and-put pattern with a helper that exposes
     `&DirEntry` by routing through `&self`. This is what TODO3
     described as `display_root_of` — implementable, just not yet
     implemented.
  3. Generation counter check before deref — keeps the raw pointer
     but adds runtime safety.
- Whichever approach, this must land before Stage B (which restructures
  these files mechanically).

> **Stage 0 exit criteria:** CI green on every PR; 3 new tests passing;
> NTFS regression closed; zero `unsafe { &*ptr }` for borrow-checker
> bypass in `src/app/`.

---

## Stage A — Close out the material migration

Steps 1–8 already shipped. Only verification remains.

### A.1 Verification pass (was Step 9)
- Animate ON × PT ON × materialize {None, On} — measure FPS delta.
  Expect near-zero now that `materialize_mix` is shader-side.
- Toggle `materialize_mix` slider — confirm `cached_instances` is **not**
  rebuilt. Add a debug counter or log line to `collect_cubes` to verify.
- Visual diff vs pre-migration on 2–3 known directories.
- Commit a `MIGRATION_NOTES.md` snippet describing the shipped contract
  for future readers.

> **Stage A exit criteria:** verification artifacts (FPS numbers + visual
> diff notes) committed under `docs/` or `.planning/`; rebuild-counter
> confirms slider liveness.

---

## Stage B — Decompose the god-objects

`render-3d/src/lib.rs` is 2309 LOC with `MaterialCache` already a clean
type — natural seam.

### B.1 Extract `Renderer3D` substructs (CONCERNS top-1)
- `crates/render-3d/src/lib.rs` →
  - `renderer3d/material_cache.rs` — move `MaterialCache`,
    `mat_settings_hash`, `settings_from_opts`, `MatGlobalUniform` (currently
    `lib.rs:84–193`).
  - `renderer3d/instance_collect.rs` — move `collect_recursive` and
    `collect_cubes` (currently `lib.rs:1102–1397`).
  - `renderer3d/pipelines.rs` (already exists) — push remaining init code
    here.
- Behaviour-preserving moves only; one commit per extraction.
- gitnexus confirms LOW risk for `collect_recursive` extraction.

### B.2 Lazy-init typestate or `RendererInited` substruct
- 15+ `as_ref().unwrap()` sites at `lib.rs:741, 869–871, 944, 2088, 2210`
  etc. all encode "this method is only valid after `init_pipelines` ran".
- Two viable approaches:
  1. Wrap `cached_instances`, `targets`, `dyn_bgs`, `instance_buffer` in a
     single `RendererInited` substruct; methods take
     `&mut self.inited.as_ref()?`.
  2. Split `Renderer3D` into `Uninit { ... }` / `Ready { ... }` typestate.
- Decide based on call-graph after B.1 lands.

### B.3 Split `src/app/mod.rs` (`plan1.md` F3 / D3)
- Extract `scan_orchestration` (scan kickoff + `ScanMsg` pump),
  `render_loop` (`run_frame` + capture viewport), `screenshot`
  (`handle_screenshot` + `save_png`).
- Mechanical moves with `pub(super)` API; verify with `git diff --stat`
  per move.

### B.4 Single CLI → `Render3DOptions` applicator (`plan1.md` F4 / D4)
- `impl Render3DOptions { fn apply_cli_overrides(&mut self, cli: &CliOptions) }`
  in `render-shared` or a new `src/app/cli_apply.rs`.
- Replace inline mirroring in `App::new`.
- Add a unit test exercising every CLI knob → option field.

> **Stage B exit criteria:** `lib.rs` < 1000 LOC; `app/mod.rs` < 800 LOC;
> CLI/PersistState mirroring single-sourced.

---

## Stage C — Polish, hardening, audit

Small self-contained items; can run in parallel.

### C.1 Surface `open::that` failures
- 5 sites confirmed: `src/app/shell.rs:94, :100`,
  `src/app/treemap_view.rs:799, :804`, `src/app/mod.rs:1319` — all
  `let _ = open::that(...)`.
- Replace each with logged + status-bar surfaced errors.
- **Security note:** `open::that` on user-selected paths invokes
  `xdg-open` on Linux; CONCERNS.md flagged a low-likelihood / high-impact
  attack vector on `.desktop` files. Consider whitelisting MIME categories
  while we're touching this code.

### C.2 GPU adapter failure path
- `crates/render-core/src/lib.rs:90, :94` — wrap `request_adapter`
  returning `None` in a user-facing message, not a silent panic in a
  downstream `unwrap`.
- Optional: prefer-list of backends (DX12 → Vulkan → Metal → GL) with
  logging of selected adapter (partly already present).

### C.3 Audit `#![allow(dead_code)]` belts (`plan1.md` F5)
- 4 belts confirmed:
  `crates/pt-wavefront/src/wavefront/pipeline.rs:2`,
  `crates/pt-megakernel/src/{pathguide,adaptive,restir}/pipeline.rs:2`.
- Either feature-gate (`#[cfg(feature = "pathguide")]`, etc.) or delete
  what no active pipeline references. CONCERNS.md flags `pathguide` and
  `adaptive` as suspect specifically.

### C.4 SAFETY annotations on Win32 FFI
- `src/scanner_ntfs.rs` — 16 `unsafe` blocks confirmed at lines
  `:39, :57, :84, :322, :355, :362, :383, :417, :451, :495, :515, :576,
  :619, :654, :672, :697`.
- One `// SAFETY:` line per block documenting buffer-size and
  record-walking invariants.

### C.5 `treemap/lib.rs` parallel raw write
- Actual location is `crates/treemap/src/lib.rs:498–520` (TODO3 said
  `:507`; the `unsafe` block starts ~line 507 but the `par_iter` opens
  at `:498`).
- Add `debug_assert!` that pixel rectangles are disjoint, or replace
  `*mut u8` + `par_iter().for_each` with `chunks_mut`.

### C.6 Two PT backends policy decision
- Document and pick: "wavefront is canonical; megakernel is fast-path for
  simple scenes" (or invert).
- Verify the `pt-megakernel → pt-wavefront` dep direction
  (`CONCERNS.md` Dead/duplicate code suspicions) — surprising, may be
  wrong way around.
- Gate megakernel-only / wavefront-only UI controls per the chosen
  policy (already partially done per the original `TODO.md` line 52).

> **Stage C exit criteria:** zero silent error paths in user-facing
> actions; no untyped lazy-init unwraps remain (those moved to Stage B);
> `pt-*` allows are either gated or removed.

---

## Stage D — Performance & visual polish

### D.1 2D treemap zero-copy upload (`plan1.md` F6 / D5)
- Goal: stop allocating `ColorImage` + `load_texture` per frame
  (`src/app/mod.rs:1035, :1068` carry the only two `TODO` markers in
  source).
- Step 1: share eframe's `wgpu::Device` with `Renderer3D`'s context.
- Step 2: 2-buffer texture pool for ping-pong upload.
- Visible win on resize and high-DPI displays.

### D.2 PT denoiser (only `TODO.md` unfinished item)
- Add normal/depth/albedo G-buffers in the shared PT output.
- Implement SVGF (or à-trous with variance guidance) post-pass.
- UI control: denoise toggle + strength.
- Big visual win at low sample counts.

### D.3 BVH rebuild gating under animation (`TODO2.md` line 21 carry-over)
- Verify that `opts.animate=true` does not trigger per-frame BVH rebuild
  in `crates/render-3d/src/pt/megakernel.rs`. If it does, gate on
  cube-count changes only (TRS-only animation should not rebuild).

### D.4 `auto-allocator` `secure` feature benchmark
- Measure scan throughput with `secure` on vs off (jwalk-heavy workload).
- Decision: keep, drop, or feature-gate. Wildcard version stays — that
  part is by design (see Watchpoints).

> **Stage D exit criteria:** PT output usable at low spp; treemap CPU
> usage drops on resize; allocator decision recorded.

---

## Cross-cutting hygiene

- Every commit on a refactor branch keeps
  `cargo build --message-format short` and
  `cargo clippy --workspace --all-targets -- -D warnings` green.
- Every PR adds at least one test if possible (Stage 0.2 sets the floor).
- **No new `unsafe { &*ptr }` for borrow-checker bypass.** Use a typed
  helper (Arc / take-and-put / generation counter — pick one in Stage 0.4
  and apply consistently).
- **Run `gitnexus_impact` before editing any function or method, and
  `gitnexus_detect_changes` before committing**, per project CLAUDE.md
  rules.

---

## Architecture decisions and watchpoints

- **`auto-allocator = "*"` is intentional.** Track latest; do not pin.
  The `secure` feature is a separate question, scheduled in D.4 for
  benchmarking.
- **`DirEntry` (`dirstat-core`) is the SSOT scan tree shape.** Filters in
  `src/app/filters.rs` and LoD merging must stay consistent with
  `DirEntry::lod_expand` semantics. Stage 0.2 tests enforce this.
- **Only 2 `TODO` markers exist in the source** (`src/app/mod.rs:1035,
  :1068`). Both relate to the zero-copy treemap path scheduled in D.1;
  do not add new TODO markers — file unfinished work in this document
  instead.
- **No `unsafe { &*ptr }` for borrow-checker bypass** (target state — 7
  current violations to remove in Stage 0.4).
- **Two-domain workspace** (dirstat + path tracer) is intentional. Do
  not collapse the crate split — `dirstat-core`, `pt-core`, `pt-mats`,
  `render-core` are designed to be reusable from other tools.
- **Single `classify_path_filtered` call site** (gitnexus-verified). Any
  future code that adds a second direct caller should be reviewed; the
  cache is the only legitimate path.

---

## gitnexus-derived observations

Recorded so future planning has codebase-wide context.

- **Cohesion outliers** (from `gitnexus://repo/dirstat-rs/clusters`):
  `Wavefront` cluster is at **71% cohesion** (vs `Settings` 91%, `Bvh_gpu`
  86%). Low cohesion in a wavefront pipeline is a signal — kernels and
  their queue/scheduler glue may be tangled with adjacent code. Run
  `gitnexus_context` on `Wavefront` cluster before any wavefront-side
  refactor.
- **Process density:** 224 execution flows over 2579 symbols means
  cross-community flows are the rule, not the exception. Renames and
  signature changes touch many flows even when they look local — always
  use `gitnexus_rename`, never find-and-replace.
- **Top execution flows pass through** UI settings → cache/exclusions
  paths and `Render → Normalize_color`. The UI settings layer is a
  high-fan-out hub: changes to settings field names ripple wider than
  they appear. Stage B.4 (single CLI applicator) is the right place to
  centralize this.
- **CPU↔GPU bridges** (`Upload_scene_smart → GpuAabb`,
  `Scan_ntfs_bg → Mask_frn`) are exactly where layout-bug regressions
  hide. Stage 0.2 tests don't cover these, and they shouldn't — these
  are integration points, validated by visual diff and Windows manual
  QA respectively.

---

## Index of references

- `.planning/codebase/CONCERNS.md` — top-concerns list and per-area audit.
- `.planning/codebase/ARCHITECTURE.md`, `STRUCTURE.md` — orientation
  before any Stage B extraction.
- `.planning/codebase/STACK.md`, `INTEGRATIONS.md`, `CONVENTIONS.md`,
  `TESTING.md` — supporting reference written by `gsd-codebase-mapper`.
- `CHANGELOG.md` — running log of behaviour-affecting changes.
- `AGENTS.md`, `DIAGRAMS.md` — system overview diagrams.
- `README.md` — user-facing overview.
- `gitnexus://repo/dirstat-rs/context` — index status, tools available.
- `gitnexus://repo/dirstat-rs/clusters` — functional areas + cohesion.
- `gitnexus://repo/dirstat-rs/processes` — execution flows.
