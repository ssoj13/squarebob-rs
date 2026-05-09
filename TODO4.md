# TODO4 — Validated Roadmap (rev 2)

**Date:** 2026-05-09
**Supersedes:** `TODO.md`, `TODO2.md`, `TODO3.md`, `plan1.md`, `task.md`
(deleted in commit `398f566`).
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
- ❌ NOT DONE. 7 sites confirmed:
  `src/app/tree_panel.rs:115, 219, 226`,
  `src/app/mod.rs:1186, 1197, 1222`,
  `src/app/treemap_view.rs:978`. `display_root_of` does not exist.
  Audit ranks this as **highest-risk unsafe class in the codebase, more
  than the Win32 FFI**.

### Visual regression (`task.md`, cube gaps)
- ❓ UNVERIFIED. No code commit on 2026-05-09. Triage required.

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

### 0.0 Triage cube-gaps regression (was claimed fixed)
- Reproduce with a known directory tree.
- If still present: bisect against the layout code in
  `crates/treemap/src/lib.rs` (last touched `ec08ace`, 2026-05-08) and
  cube model-matrix code in `crates/render-3d/src/lib.rs`
  (`collect_recursive` area).
- If actually fixed: find the fix commit, document it here, close.

### 0.1 Remove UI raw-pointer aliasing (CRITICAL)
**Highest-risk unsafe class per CONCERNS.md.** 7 sites of
`let root = unsafe { &*root_ptr };`:
- `src/app/tree_panel.rs:111` (stores), `:115, 219, 226` (deref).
- `src/app/mod.rs:1181, 1190, 1216` (stores), `:1186, 1197, 1222` (deref).
- `src/app/treemap_view.rs:978` (deref).

**Approach options** (decision needed before implementation):
1. `Arc<DirEntry>` — clone-on-need, simple, one Arc per borrow site.
2. Take-and-put pattern with a typed helper exposing `&DirEntry` via
   `&self`. (TODO3 named this `display_root_of` but never wrote it.)
3. Generation counter check before deref — keeps the raw pointer but adds
   runtime safety.

Lands before Stage B (which restructures these files mechanically).

### 0.2 Stage A.1 — close out the material migration
The migration is 8/9 done. Step 9 verification:

- Animate ON × PT ON × materialize {None, On} — measure FPS delta.
  Expect near-zero now that `materialize_mix` is shader-side.
- Add a debug counter or log line to `collect_cubes` to count rebuilds.
  Toggle the `materialize_mix` slider — confirm `cached_instances` is
  **not** rebuilt.
- Visual diff vs pre-migration on 2–3 known directories.
- Commit a `MIGRATION_NOTES.md` snippet (or a section in
  `.planning/`) describing the shipped contract.

### 0.3 NTFS fallback no longer rewrites user prefs
- Add `ntfs_last_error: Option<String>` (transient, not persisted) to `App`.
- In `ScanMsg::NtfsFallback` handler at `src/app/mod.rs:619–623`: set the
  transient error, **do not** mutate `scanner_mode`.
- Non-modal banner when `ntfs_last_error.is_some()`.
- Manual QA on Windows.

### 0.4 Tests for SSOT functions
**Note:** rev-2 finding — `lod_expand` is a field, not a function. Step
1 is to identify the actual function that produces `LodExpandInfo` and
test that.

- Identify the LoD-merge producer (probably in `dirstat-core` or scanner
  code that populates `DirEntry::lod_expand`). Read first; test second.
- Table-driven tests for `pt_mats::classify_path_filtered` over
  `materialize_mode`, `mat_allow_lights`, `mat_allow_glass`. The function
  has exactly one caller (`MaterialCache`); silent behavior changes are
  the failure mode.
- Treemap squarified-layout: input tree → expected rectangle set. This
  is the test that would have caught the cube-gaps regression in CI.

> **Stage 0 exit criteria:** cube-gaps triage closed; zero
> `unsafe { &*ptr }` for borrow-checker bypass in `src/app/`; Stage A
> verification artifacts committed; NTFS regression closed; 3 SSOT
> tests passing.

---

## Stage A — RETIRED

Stages A.1–A.6 from TODO3 (Steps 4–9 of the migration) are either shipped
already or merged into Stage 0.2 above. No standalone Stage A remains.

---

## Stage B — Decompose the god-objects

Unblocked once Stage 0.1 (raw-pointer fix) lands; the helper introduced
there is the natural seam.

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
- **No new `unsafe { &*ptr }` for borrow-checker bypass.** Use the
  helper introduced in Stage 0.1.
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
- **No `unsafe { &*ptr }` for borrow-checker bypass** (target — 7
  current violations to remove in Stage 0.1).
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
