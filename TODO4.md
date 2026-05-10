# TODO4 — Validated Roadmap (rev 5)

**Date:** 2026-05-09
**Supersedes:** `TODO.md`, `TODO2.md`, `TODO3.md`, `plan1.md`, `task.md`
(deleted in commit `398f566`).

**rev 5 changes (Stage D.1 + Stage C.3 cleanup + docs refresh):**
Sprint 3: zero-copy 2D-GPU display path implemented as
`render_2d_callback` (mirrors the existing `render_3d_callback`).
`GpuRenderer2D::render_to_texture` + `get_render_texture` are now
public; render-target texture has `TEXTURE_BINDING` usage. The two
stale `TODO` markers in `src/app/mod.rs` (the ONLY TODO markers in
source per CONCERNS.md) replaced with accurate comments describing
the CPU-readback fallback path.

Architectural note for deferred Stage D.2 (denoiser): the
`register_native_texture` infrastructure used by both 3D and 2D-GPU
paths is the natural integration point for the denoiser's RGBA output.
Add a `get_denoised_texture()` accessor on the PT pipeline → register
with egui → display via the existing `render_texture_id`. No new
display infrastructure needed.

Stage C.3 audit closed: removed all four blanket `#![allow(dead_code)]`
belts in `pt-megakernel`/`pt-wavefront` pipeline.rs files. With the
allows removed, `cargo clippy --workspace --all-targets -- -D warnings`
produces 0 warnings — every symbol is reachable.

CONCERNS.md, STRUCTURE.md, TESTING.md, ARCHITECTURE.md, AGENTS.md
refreshed with post-sprint state. New `CHANGELOG.md` covers sprints
1–3. ~/.claude/CLAUDE.md updated with cross-project insights (GCC 15
+ libmimalloc workaround, multi-agent orchestration patterns,
plans-inherit-errors discipline, Rust gotchas).

**rev 4 changes (sprint 2 — multi-agent Stage B + parallel polish):**
Massive batch of Stage B + C + E work shipped via 2 parallel sub-agents
in worktrees plus several main-thread atomic commits. Status:

- ✅ **Stage B.1** DONE (Agent A): `crates/render-3d/src/lib.rs` 2335→1937 LOC.
  `MaterialCache` extracted to `renderer3d/material_cache.rs` (123 LOC),
  `collect_cubes` + `collect_recursive` extracted to
  `renderer3d/instance_collect.rs` (300 LOC). cargo check passed in
  agent worktree.
- ⚠️ **Stage B.2** PARTIAL: lifecycle analysis disqualified the
  RendererInited substruct (cached_instances/instance_buffer build
  per-frame; targets/dyn_bgs build in resize path; env-map path needs
  targets=Some + dyn_bgs=None). Full typestate too invasive. Compromise:
  17 `.unwrap()` sites upgraded to `.expect()` with field-specific
  diagnostic messages. Full Uninit/Ready typestate remains open if the
  user wants it later.
- ✅ **Stage B.3** DONE (Agent B): `src/app/mod.rs` 1521→716 LOC after
  B.4. Three submodules created: `scan_orchestration.rs` (218),
  `render_loop.rs` (278), `screenshot.rs` (139).
- ✅ **Stage B.4** DONE (Agent B.4): `src/app/cli_apply.rs` (443 LOC,
  90 field copies + 2 unit tests). `App::new` no longer mirrors
  CliOptions inline; one `apply_cli_overrides()` call.
- ✅ **Stage C.2** DONE: `GpuContext::new()` graceful Option propagation
  verified (zero unwrap on `gpu_context` in entire codebase). Added
  `log::error!` on adapter/device failure so env_logger now distinguishes
  "no compatible adapter" from "adapter ok but device init failed".
- ✅ **Stage C.3** investigated (no code change): pathguide and adaptive
  are NOT dead — both are imported from `pt-megakernel/src/compute.rs:9-10`
  and contain WGSL shaders. The `#![allow(dead_code)]` belts mask
  individual unused symbols inside pipeline.rs, not whole modules.
  Targeted cleanup deferred (needs working cargo check; this WSL
  environment can't build the binary due to GCC 15 / mimalloc
  incompatibility).
- ✅ **Stage C.6** investigated (no code change): `pt-megakernel`
  depends on `pt-wavefront` via single import in `compute.rs:16`
  (`use pt_wavefront::{WavefrontConfig, WavefrontPipeline, WfDims}`).
  This is an intentional orchestrator pattern — megakernel knows about
  both backends. Not "wrong direction" as CONCERNS.md suspected. Policy
  text ("canonical = ?") still requires user decision.
- ✅ **Stage D.3** code-verified: BVH refit fast-path is implemented at
  `crates/bvh-gpu/src/bvh_gpu/mod.rs:329` (`can_refit`) + `:378`
  (`refit`), gated by `opts.pt_gpu_bvh && opts.pt_bvh_refit` at
  `crates/render-3d/src/pt/megakernel.rs:205, :692`. Falls back to full
  rebuild if `can_refit()` returns false. Runtime trace verification
  remains user work.
- ✅ **Stages E.1 + E.2** DONE: `.github/workflows/ci.yml` shipped with
  Linux + Windows matrix, Swatinem/rust-cache, and rustsec/audit-check.
  Weekly cron for advisory check.

**rev 3 changes:** removed Stage 0.0 (cube-gaps fixed per user); marked
Stage 0.1 DONE — the SAFETY-comment + disciplined `&mut self`-scoped
pattern *is* the fix; located the LoD-merge producer in
`src/app/filters.rs` (not `dirstat-core`) and confirmed 3 tests already
cover it; added an "Execution sequence" section grouping work by file
area, not stage, to amortize gitnexus impact analysis and code-read
costs.
**Validation method:** every "already done" claim cross-checked against the
current source tree; line numbers re-verified; gitnexus impact run on key
seams (`classify_path_filtered`, `collect_recursive`, `lod_expand`).
A second-pass re-verification (rev 2) caught additional drift; see
"Re-verification findings" below.

---

## Why this document replaces TODO3

TODO3's status snapshot diverges from the code in three material ways:

1. **TODO3 claims Stage A (unified material migration) is "Step 0–3 partial,
   Step 4+ pending."** Validation: **Steps 1–8 are already in code.** Only
   verification (Step 9) remains.
2. **TODO3 claims "branch `fix/remove-ui-raw-pointers` removed all 7 unsafe
   `&*ptr` sites via `display_root_of`."** Validation: that branch does not
   exist, `display_root_of` does not exist, all 7 sites are present.
3. **TODO3 claims "Visual regression (cube gaps, `task.md`): fixed
   2026-05-09."** Validation (rev 2): the only commit on 2026-05-09 is
   `2b8c98d`, containing only `.omc/` session-state JSON. No code changes
   were committed on that day before `task.md` was removed. **No evidence
   the regression is fixed.** Treemap layout was last touched in
   `ec08ace` (2026-05-08).

The corrected priorities below reorder around real quality work
(bugs, verifications, tests). Process tooling (CI, audit) moved to Stage E
because it's enforcement, not introduction — without tests it is an empty
harness.

---

## Re-verification findings (rev 2)

These corrections came from a second-pass code read after the rev-1 draft.

| Claim (rev 1 / TODO3) | Reality (verified) | Action |
|-----------------------|-------------------|--------|
| `DirEntry::lod_expand` is a method to test. | It's a **field** `pub lod_expand: Option<LodExpandInfo>` at `crates/dirstat-core/src/lib.rs:46`. There is no `lod_expand()` function or method. | Stage 0.4: test the producer of `LodExpandInfo` (LoD-merge logic), not `lod_expand` itself. Identify producer first. |
| Cube-gaps regression fixed 2026-05-09. | No code commit on 2026-05-09 — only session-state JSON. Treemap layout untouched since 2026-05-08. | Stage 0.0 (NEW): triage the regression — does it still reproduce? If yes, find root cause before touching any visual code. |
| BVH rebuild gating under animation needs implementation. | Already implemented: `GpuBvhBuilder` has refit-vs-rebuild path at `crates/bvh-gpu/src/bvh_gpu/mod.rs:327`, with `is_refit: u32` flag in WGSL (`aabb_compute.wgsl:37`). | Stage D.3 reduced to **verification only** — confirm refit fast-path triggers when cube count is stable. |
| GPU adapter failure path needs wrapping. | `crates/render-core/src/lib.rs:88: pub fn new() -> Option<Self>` returns `None` gracefully via `.ok()?`. | Stage C.2 reduced to **investigate downstream** — find caller of `GpuContext::new()` and check what it does with `None`. May already be handled. |
| `pathguide` / `adaptive` look like dead code. | Both directories have substantial content: `config.rs`, `mod.rs`, `pipeline.rs`, plus WGSL shaders (`pathguide/{sample,update}.wgsl`, `adaptive/{allocate,variance}.wgsl`). | Stage C.3: don't delete — feature-gate or audit which symbols are referenced. The `#![allow(dead_code)]` belt may hide unfinished integration, not abandonment. |

---

## Validated state — what is actually done

### PT quality (`TODO.md`)
- ✅ DONE: NEE for emissive cubes, full MIS, ReSTIR DI (wavefront-only),
  R2 low-discrepancy sampling, adaptive sampling polish, firefly
  per-sample clamp, environment MIS PDF audit, material lobe PDF audit,
  wavefront parity scope marking. Open: denoiser only.

### Unified material migration (`TODO2.md`) — Steps 1–8 shipped
Confirmed by direct code read:

- ✅ `material_id: u32` field + vertex slot 9 = `Uint32`
  (`crates/render-3d/src/geometry.rs:14, 44`).
- ✅ `MaterialCache { settings_hash, classes_pbr, classes_pt }` with
  `mat_settings_hash` and `settings_from_opts` at
  `crates/render-3d/src/lib.rs:101–193`. `classify_or_get` at `lib.rs:121`.
- ✅ PBR `collect_recursive` calls `mat_cache.classify_or_get(...)` at
  `lib.rs:1364`; `mat_cache.ensure(opts)` primer at `lib.rs:1102`.
- ✅ PT scene build uses cache at
  `crates/render-3d/src/pt/megakernel.rs:140` and `:620`, ensure-primers at
  `:120` and `:600`.
- ✅ `materials_buf` and `mat_global_buf` are real fields on `Renderer3D`
  (`lib.rs:214, 215, 490, 495, 539, 543`).
- ✅ `cube_pbr.wgsl`: `GpuMaterial` struct (line 44),
  `@binding(2) materials: array<GpuMaterial>` (line 69),
  `@binding(3) mat_global` UBO (line 70), `material_id` flat through
  `VertexOutput`, `resolve_material` mixing
  `instance_color` and `m.base_color_weight.rgb` by `materialize_mix`.
- ✅ CPU `color_f` blend removed (comment at `lib.rs:1360` confirms).
- ✅ `MaterialParamsUniform` and legacy `material_buf: wgpu::Buffer` are
  gone (grep returns 0 in `crates/` and `src/`).

**gitnexus invariant:** `classify_path_filtered` has exactly one direct
upstream caller (`MaterialCache.classify_or_get`), risk LOW —
the architectural goal of "single classify call site" is held in code.

### Bug-hunt deferrals (`plan1.md`)
- ⏳ D1 NTFS fallback — open. `src/app/mod.rs:619–623` still does
  `self.scanner_mode = ScannerMode::Standard`. No `ntfs_last_error`.
- ⏳ D2 `auto-allocator = "*"` — kept by design; benchmark in Stage D.4.
- ⏳ D3 split `app/mod.rs` — open (1518 LOC).
- ⏳ D4 single CLI applicator — open.
- ⏳ D5 zero-copy treemap — open. Two `TODO` markers at
  `src/app/mod.rs:1035, :1068`.

### UI raw-pointer aliasing (CONCERNS top-7)
- ✅ DONE — pattern is the fix.
  All 7 `unsafe { &*ptr }` sites still exist in source
  (`src/app/tree_panel.rs:115, 219, 226`, `src/app/mod.rs:1186, 1197,
  1222`, `src/app/treemap_view.rs:978`), but each carries a
  `// Safety:` comment documenting the invariant: the raw pointer is
  captured at the start of an `&mut self` method, used only within
  that single method call, never stored across calls. The owning
  field (`self.tree` / `self.display_tree_cache`) is not mutated
  between capture and last deref. CONCERNS.md overstated the risk —
  no concurrent thread can hold `&mut self`, and the disciplined
  in-method pattern eliminates the use-after-free scenario it
  worried about.
- **Optional future refactor (Stage B):** switch to `Arc<DirEntry>`
  to remove the `unsafe` entirely. Cosmetic improvement, not a bug
  fix.

### Visual regression (`task.md`, cube gaps)
- ✅ Fixed (per user, 2026-05-09). No further triage.

### Existing tests
- 8 `#[test]` declarations across 3 files:
  `crates/render-shared/src/lib.rs`,
  `src/app/filters.rs`,
  `crates/bvh-gpu/src/bvh_gpu/mod.rs`.
- SSOT functions (`classify_path_filtered`, treemap layout, LoD-merge
  producer) have **zero tests**.

### Lazy-init unwraps in `Renderer3D` (CONCERNS)
- 15 sites confirmed in `crates/render-3d/src/lib.rs`:
  741, 869, 870, 871, 944, 945, 954, 1067, 2088, 2126, 2130, 2210, 2211,
  2217, 2221.

---

## Stage 0 — Real quality work (do first)

Reordered around the principle: **introduce quality before enforcing it.**
CI is moved to Stage E.

### 0.1 Stage A.1 — close out the material migration
The migration is 8/9 done. Step 9 verification:

- Animate ON × PT ON × materialize {None, On} — measure FPS delta.
  Expect near-zero now that `materialize_mix` is shader-side.
- Add a debug counter or log line to `collect_cubes` to count rebuilds.
  Toggle the `materialize_mix` slider — confirm `cached_instances` is
  **not** rebuilt.
- Visual diff vs pre-migration on 2–3 known directories.
- Commit a `MIGRATION_NOTES.md` snippet (or a section in
  `.planning/`) describing the shipped contract.

### 0.2 NTFS fallback no longer rewrites user prefs
- Add `ntfs_last_error: Option<String>` (transient, not persisted) to `App`.
- In `ScanMsg::NtfsFallback` handler at `src/app/mod.rs:619–623`: set the
  transient error, **do not** mutate `scanner_mode`.
- Non-modal banner when `ntfs_last_error.is_some()`.
- Manual QA on Windows.

### 0.3 Tests for SSOT functions
**rev-3 finding:** the LoD-merge producer is in
`src/app/filters.rs:212, :258` (`merge_tree_by_size_range`), **not** in
`dirstat-core`. Three tests already exist at
`src/app/filters.rs:530–600` (`merge_buckets_outside_range`,
`merge_expanded_small_is_directory`, `count_outside_range`). Coverage
is adequate; further LoD-merge tests are optional bonus, not required.

- ✅ LoD-merge: already covered. (Optional bonus: boundary edge cases —
  `large_n == 1` plural form, `small_sum == max` exact-equal threshold.)
- ⏳ `pt_mats::classify_path_filtered`: table-driven tests over
  `materialize_mode`, `mat_allow_lights`, `mat_allow_glass`. Function
  has exactly one direct caller (`MaterialCache`); silent behavior
  changes are the failure mode.
- ⏳ `treemap` squarified-layout: input tree → expected rectangle
  set. This is the test that would have caught the cube-gaps
  regression in CI.

> **Stage 0 exit criteria:** Stage A verification artifacts committed;
> NTFS regression closed; 2 new SSOT tests added (classify, squarified).

---

## Stage A — RETIRED

Stages A.1–A.6 from TODO3 (Steps 4–9 of the migration) are either shipped
already or merged into Stage 0.2 above. No standalone Stage A remains.

---

## Stage B — Decompose the god-objects

`render-3d/src/lib.rs` is 2309 LOC with `MaterialCache` already a clean
type — natural seam.

### B.1 Extract `Renderer3D` substructs (CONCERNS top-1)
- `crates/render-3d/src/lib.rs` (2309 LOC) →
  - `renderer3d/material_cache.rs` — move `MaterialCache`,
    `mat_settings_hash`, `settings_from_opts`, `MatGlobalUniform`
    (`lib.rs:84–193`).
  - `renderer3d/instance_collect.rs` — move `collect_recursive` and
    `collect_cubes` (`lib.rs:1102–1397`).
  - `renderer3d/pipelines.rs` (already exists) — push remaining init
    code here.
- gitnexus confirmed LOW risk for `collect_recursive` extraction
  (1 direct caller).

### B.2 Lazy-init typestate or `RendererInited` substruct
- 15 `as_ref().unwrap()` sites at lines listed above.
- Two viable approaches:
  1. Wrap `cached_instances`, `targets`, `dyn_bgs`, `instance_buffer`
     in a single `RendererInited` substruct.
  2. Split `Renderer3D` into `Uninit { ... }` / `Ready { ... }` typestate.
- Decide based on call-graph after B.1.

### B.3 Split `src/app/mod.rs` (1518 LOC)
- Extract `scan_orchestration` (scan kickoff + `ScanMsg` pump),
  `render_loop` (`run_frame` + capture viewport), `screenshot`
  (`handle_screenshot` + `save_png`).
- Mechanical moves with `pub(super)` API.

### B.4 Single CLI → `Render3DOptions` applicator
- `impl Render3DOptions { fn apply_cli_overrides(&mut self, cli: &CliOptions) }`
  in `render-shared` or new `src/app/cli_apply.rs`.
- Replace inline mirroring in `App::new`.
- Unit test exercising every CLI knob.

> **Stage B exit criteria:** `lib.rs` < 1000 LOC; `app/mod.rs` < 800 LOC;
> CLI/PersistState mirroring single-sourced.

---

## Stage C — Polish, hardening, audit

Small self-contained items; can run in parallel.

### C.1 Surface `open::that` failures
- 5 sites confirmed: `src/app/shell.rs:94, :100`,
  `src/app/treemap_view.rs:799, :804`, `src/app/mod.rs:1319` — all
  `let _ = open::that(...)`.
- Replace with logged + status-bar surfaced errors.
- Security note: on Linux `xdg-open` honours `.desktop` files —
  consider whitelisting MIME categories while editing.

### C.2 GPU adapter failure — investigate downstream
**Rev-2 correction:** `request_adapter` *does* propagate `None` via
`.ok()?` and `new() -> Option<Self>`. The work is to find
`GpuContext::new()` callers and confirm they handle `None` gracefully
(not unwrap to panic).
- Optional: prefer-list of backends (DX12 → Vulkan → Metal → GL) with
  logging of selected adapter.

### C.3 Audit `#![allow(dead_code)]` belts
4 belts confirmed at:
- `crates/pt-wavefront/src/wavefront/pipeline.rs:2`
- `crates/pt-megakernel/src/{pathguide,adaptive,restir}/pipeline.rs:2`

**Rev-2 note:** `pathguide` and `adaptive` have substantial code
including WGSL shaders, not just stubs. Audit which symbols are
referenced from active pipelines; feature-gate the rest. Don't delete
without checking.

### C.4 SAFETY annotations on Win32 FFI
- 16 `unsafe` blocks in `src/scanner_ntfs.rs` at lines
  `:39, :57, :84, :322, :355, :362, :383, :417, :451, :495, :515, :576,
  :619, :654, :672, :697`.
- One `// SAFETY:` line per block.

### C.5 `treemap/lib.rs` parallel raw write
- `crates/treemap/src/lib.rs:498–520` (loop start `:498`, unsafe block
  starts `:507`).
- Add `debug_assert!` that pixel rectangles are disjoint, or replace
  `*mut u8` + `par_iter().for_each` with `chunks_mut`.

### C.6 Two PT backends policy decision
- Document and pick: "wavefront is canonical; megakernel is fast-path
  for simple scenes" (or invert).
- Verify the `pt-megakernel → pt-wavefront` dep direction — surprising,
  may be wrong way around.
- Gate megakernel-only / wavefront-only UI controls per the chosen policy.

> **Stage C exit criteria:** zero silent error paths in user-facing
> actions; no untyped lazy-init unwraps remain (those moved to Stage B);
> `pt-*` allows are either gated or removed.

---

## Stage D — Performance & visual polish

### D.1 2D treemap zero-copy upload
- Stop allocating `ColorImage` + `load_texture` per frame
  (`src/app/mod.rs:1035, :1068` carry the only two `TODO` markers).
- Step 1: share eframe's `wgpu::Device` with `Renderer3D`'s context.
- Step 2: 2-buffer texture pool for ping-pong upload.

### D.2 PT denoiser (only `TODO.md` unfinished item)
- Add normal/depth/albedo G-buffers in shared PT output.
- SVGF (or à-trous with variance guidance) post-pass.
- UI control: denoise toggle + strength.

### D.3 BVH rebuild gating — VERIFICATION ONLY
**Rev-2 correction:** refit-vs-rebuild is already implemented
(`crates/bvh-gpu/src/bvh_gpu/mod.rs:327`, `is_refit` flag at
`aabb_compute.wgsl:37`).
- Verify the fast-path triggers when cube count is stable under
  `opts.animate=true`. Add a log/metric if needed.
- If full rebuild fires every animated frame, that's a wiring bug —
  trace from `megakernel.rs:223` and `:710` (call sites of `pt.build_bvh`).

### D.4 `auto-allocator` `secure` feature benchmark
- Measure scan throughput with `secure` on vs off (jwalk-heavy workload).
- Decision: keep, drop, or feature-gate. Wildcard version stays.

> **Stage D exit criteria:** PT output usable at low spp; treemap CPU
> usage drops on resize; allocator decision recorded.

---

## Stage E — Process tooling (formerly Stage 0.1)

Moved here because process is enforcement, not introduction. CI without
tests is an empty harness; with tests (Stage 0.4 lands them) it becomes
real value.

### E.1 Minimum-viable CI
- `.github/workflows/ci.yml` running `cargo build --workspace`,
  `cargo clippy --workspace --all-targets -- -D warnings`,
  `cargo test --workspace`.
- Linux runner first; **Windows runner essential** because
  `scanner_ntfs.rs` (936 LOC, 16 unsafe FFI) is Windows-only and gets
  zero coverage from a WSL dev loop.
- Cache target dir via `Swatinem/rust-cache@v2`.
- Order matters: do this **after** Stage 0.4 ships SSOT tests, so
  `cargo test --workspace` is non-empty.

### E.2 `cargo audit` / `cargo deny`
- Weekly job for the `auto-allocator = "*"` wildcard concern.
- This is the actual reason wildcard pinning is acceptable —
  the audit job catches breaking-version drift.

### E.3 Embeddings build for gitnexus
- Current state: `embeddings: 0`. Hybrid ranking falls back to
  BM25-only.
- Run `npx gitnexus analyze --embeddings` once. Cost: longer index
  time, ongoing better natural-language `gitnexus_query` results.

### E.4 Upstream gitnexus bug (optional)
- File issue: FTS index ensure write attempt while DB is held read-only
  by the running MCP server, with no backoff between retries → 5
  warning lines per Bash/Grep/Glob hook firing.
- Local mitigation already applied: per-line filter in
  `~/.claude/hooks/gitnexus/gitnexus-hook.cjs` (out-of-tree).

> **Stage E exit criteria:** CI green on every push; weekly audit job
> running; embeddings populated; upstream bug acknowledged or
> wontfixed.

---

## Cross-cutting hygiene

- Every commit on a refactor branch keeps
  `cargo build --message-format short` and
  `cargo clippy --workspace --all-targets -- -D warnings` green.
- Every PR adds at least one test if possible (Stage 0.4 sets the
  floor).
- **No new `unsafe { &*ptr }` for borrow-checker bypass without a
  matching `// Safety:` comment.** Match the existing disciplined
  pattern: capture pointer at start of `&mut self` method, use only
  within that call, never store across calls. See existing sites in
  `tree_panel.rs`, `mod.rs:capture_viewport`, `treemap_view.rs` for
  examples.
- **Run `gitnexus_impact` before editing any function or method, and
  `gitnexus_detect_changes` before committing**, per project CLAUDE.md.

---

## Architecture decisions and watchpoints

- **`auto-allocator = "*"` is intentional.** Track latest; do not pin.
  The `secure` feature is a separate question, scheduled in D.4.
- **`DirEntry` is the SSOT scan tree shape.** Note:
  `DirEntry::lod_expand` is a field, not a method — test the producer
  of `LodExpandInfo`, not `lod_expand` itself.
- **Only 2 `TODO` markers exist in source** (`src/app/mod.rs:1035,
  :1068`). Both relate to D.1; do not add new TODO markers — file
  unfinished work in this document.
- **`unsafe { &*ptr }` is acceptable only with `// Safety:` comment
  documenting the invariant**, and only in the disciplined
  in-method-scope pattern. Do not introduce new sites without
  matching the existing convention.
- **Two-domain workspace** (dirstat + path tracer) is intentional.
- **Single `classify_path_filtered` call site** (gitnexus-verified). Any
  future code that adds a second direct caller should be reviewed.
- **BVH refit fast-path exists.** When extending PT scene management,
  preserve the `is_refit` capability — losing it means animated PT
  rebuilds the BVH every frame.

---

## gitnexus-derived observations

- **Cohesion outliers**: `Wavefront` cluster at 71% (vs `Settings` 91%,
  `Bvh_gpu` 86%). Run `gitnexus_context` on `Wavefront` cluster before
  any wavefront-side refactor.
- **Process density**: 224 execution flows over 2582 symbols (post
  rev-2 re-analyze). Cross-community flows are the rule. Always use
  `gitnexus_rename`, never find-and-replace.
- **Top execution flows** pass through UI settings → cache/exclusions
  paths and `Render → Normalize_color`. The UI settings layer is a
  high-fan-out hub. Stage B.4 (single CLI applicator) is the right
  place to centralize.
- **CPU↔GPU bridges** (`Upload_scene_smart → GpuAabb`,
  `Scan_ntfs_bg → Mask_frn`) are where layout-bug regressions hide.

---

## Execution sequence — current sprint (rev 3)

Code-only work, grouped by file area to amortize gitnexus impact
analysis and code-read costs. Visual / runtime verification deferred.
Each wave = one atomic commit.

| Wave | Area | Tasks | Commit msg seed |
|------|------|-------|-----------------|
| **A** | `src/scanner_ntfs.rs` + `src/app/mod.rs` (NTFS handler) + `src/app/state.rs` (App fields) | 0.2 NTFS fallback fix; C.4 SAFETY annotations on 16 unsafe FFI blocks | `fix(ntfs): preserve user scanner_mode; document FFI safety` |
| **B** | `crates/treemap/src/lib.rs` | C.5 `debug_assert!` for disjoint rects; 0.3c squarified-layout test | `treemap: assert disjoint rects + add layout regression test` |
| **C** | `crates/pt-mats/src/lib.rs` | 0.3a `classify_path_filtered` table-driven tests | `pt-mats: table-driven tests for classify_path_filtered` |
| **D** | `src/app/shell.rs` + `src/app/treemap_view.rs` + `src/app/mod.rs` | C.1 surface 5 `open::that` failures via log + status bar | `ux: surface open::that failures instead of silently dropping` |
| **E** | `crates/render-3d/src/lib.rs` | 0.1 (Stage A.1 partial) instance-rebuild counter | `render-3d: instrument cached_instances rebuild counter` |

**Out of scope for this sprint** (need user input or runtime):
- Stage 0.1 manual FPS measurement + visual diff (Stage A close-out)
- Stage B.1–B.4 god-object decomposition (mechanical refactor, larger
  blast radius)
- Stage C.2 GPU adapter downstream investigation (read + decide)
- Stage C.3 pathguide/adaptive feature-gate audit (decision needed)
- Stage C.6 PT backend canonical-vs-fast-path policy (decision needed)
- Stage D.1–D.4 performance work
- Stage E.1–E.4 process tooling

After all waves land: run `cargo build --workspace` and
`cargo clippy --workspace --all-targets -- -D warnings` once across the
batch, plus `npx gitnexus analyze` to refresh the index for any new
symbols (tests).

---

## Stage F — Wavefront race fix + parity (sprint-4, 2026-05-10)

| Stage | Status |
|-------|--------|
| F.1 wavefront tile-state race | ✅ shipped (commit `5ff8929`) — N-slot persistent buffers + dynamic-offset bind groups + `prepare_tiles` / `reset_tile_count` API |
| F.2 spectral wavefront (drop stub) | ✅ shipped (commit `407ff73`) — `spectral.rs` no longer forces megakernel; `shade.wgsl` applies `spectral_tint` at transmission too |
| F.3 tile-size input safety | ✅ shipped (commit `ddbdd26`) — UI snap + code clamp + hard assert (`MAX_TILE_CAPACITY = 4096`) so transient `WF Tile=2` cannot hang the GPU |
| F.4 ReSTIR/PG/Adaptive in tiled mode | ✅ shipped — Adaptive (`43e9376`), F.4-A PathGuide (`6ef6aac`), F.4-B..G gbuffer + 4 ReSTIR shaders (`0bec861`); force-disable lifted |
| F.5 Windows build fix | ✅ shipped (commit `b6e84e9`) — `cli_test.rs` / `scanner_ntfs.rs` / `scan_orchestration.rs` after WIP unused-var rename broke Windows-only paths |
| F.6 wavefront unit tests | ✅ shipped (commit `76c28f5`) — 6 new tests on tile-slot layout invariants; workspace 32→38 tests |

### F.4 — ReSTIR / Path Guide / Adaptive in tiled wavefront mode (shipped)

Closed in the order proposed (lowest risk first), each shipped with
build/test/clippy green:

1. **Adaptive** (`43e9376`) — already tile-safe by construction
   (variance/allocate run once per frame on the full image after the
   tile loop). Lifted force-disable + warn only.
2. **F.4-A Path Guide sample** (`6ef6aac`) — added `tile_pos` to
   `PathGuideSampleParams`, remapped `gid.x` to a global pixel index in
   `pathguide/sample.wgsl`, migrated its params binding to dynamic-offset
   with per-tile pre-packing (`PG_SAMPLE_PARAMS_SIZE=96`). Update.wgsl
   is `workgroup_size(1)`, no change needed. Force-disable removed for
   PathGuide. Side effect: `MAX_TILE_CAPACITY`, `DEFAULT_TILE_CAPACITY`,
   `pack_tile_slots` are now `pub` from `pt-wavefront`.
3. **F.4-B..G gbuffer + 4 ReSTIR shaders** (`0bec861`) — five WGSL
   kernels distinguish `local_id` (tile-sized rays/hits) from `pixel_id`
   (full-image reservoirs / depth / normal / motion / sample_map /
   output). Five params bindings (gbuffer@5, restir initial@2 /
   temporal@5 / spatial@4 / shade@3) migrated to dynamic-offset with
   per-tile pre-packing. ReSTIR force-disable + warn removed; tiling
   now coexists with the whole stack.

Total: ~370 LOC of net additions across 7 WGSL files +
`compute.rs` + `restir/pipeline.rs` + `pathguide/pipeline.rs`.

Visual UAT for the user (HANDOFF.md "F.4 UAT" section): render WF
Tile=0 vs WF Tile=256 with each of {PathGuide, Adaptive, ReSTIR DI,
ReSTIR DI+GI} and with all of them on at once — each pair should
converge to the same image. No "X disabled" warning in the console.

Bonus fix (commit `2767548`): the prior `prev_view_proj ==
curr_view_proj` caveat was resolved by adding a `prev_view_proj`
matrix cache to `PathTraceCompute`. The two renderer entry points
(`render.rs`, `render_no_readback.rs`) roll the existing matrix into
prev before storing the new one on a camera-move. ReSTIR temporal
reuse now reprojects against the real previous-frame projection.

---

## Index of references

- `.planning/codebase/CONCERNS.md` — top-concerns list and per-area
  audit.
- `.planning/codebase/ARCHITECTURE.md`, `STRUCTURE.md` — orientation
  before any Stage B extraction.
- `.planning/codebase/STACK.md`, `INTEGRATIONS.md`, `CONVENTIONS.md`,
  `TESTING.md` — supporting reference.
- `CHANGELOG.md` — running log of behaviour-affecting changes.
- `AGENTS.md`, `DIAGRAMS.md` — system overview diagrams.
- `README.md` — user-facing overview.
- `gitnexus://repo/dirstat-rs/context` — index status, tools available.
- `gitnexus://repo/dirstat-rs/clusters` — functional areas + cohesion.
- `gitnexus://repo/dirstat-rs/processes` — execution flows.
