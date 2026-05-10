# HANDOFF — dirstat-rs cross-machine resume

**Date:** 2026-05-10 (rev 2)
**Last commit:** see `git log -1`
**Branch:** main

## 2026-05-10 update — Wavefront race + spectral fixes

Stage F.1 wavefront tile race + spectral parity landed in this session
(commits between `51eaf11` and HEAD):

- **F.1 wavefront tile race fix** — per-tile `dims_buf` / `count_buf`
  state was collapsing to last-tile values because wgpu flushes all
  `queue.write_buffer` calls before encoder commands at submit. Replaced
  the single-slot buffers with N-slot persistent buffers
  (`tile_dims_buf`, `tile_counts_buf`, `count_init_src`) addressed via
  dynamic-offset bind groups. Per-frame upload is now ONE
  `queue.write_buffer` per buffer (`prepare_tiles`); per-tile init goes
  through `encoder.copy_buffer_to_buffer` (`reset_tile_count`),
  encoder-ordered with the dispatches. WGSL shaders unchanged.
  See `crates/pt-wavefront/src/wavefront/pipeline.rs` and the
  refactored `dispatch_wavefront` in
  `crates/pt-megakernel/src/compute.rs`.

- **F.2 spectral really runs in wavefront** —
  `crates/render-3d/src/pt/spectral.rs` no longer forces
  `pt_wavefront=false`; the wavefront `shade.wgsl` already applied
  `spectral_tint` at sky/emission and now also at transmission events
  (parity with megakernel).

- **F.3 unit tests** — `crates/pt-wavefront/src/wavefront/pipeline.rs`
  has 6 new tests for the dynamic-offset slot layout invariants
  (`TILE_SLOT_STRIDE == 256`, `pack_tile_slots` correctness, etc.).

### Open follow-up: F.4 — ReSTIR/PathGuide/Adaptive in tiled wavefront

Currently force-disabled when wavefront tiling is on, with an explicit
comment in `compute.rs::dispatch_wavefront` near the warnings.
**Reason:** their WGSL shaders index `reservoirs[pixel_id]` etc. with
`pixel_id = gid.y * params.width + gid.x`, where `params.width` is the
PER-TILE width but the reservoir / sample-map / variance buffers are
full-image-sized. Different tiles would alias into the same slots.

To re-enable them in tiled mode (separate phase, ~300-500 LOC):
1. Add `tile_x, tile_y, full_width, full_height` fields to:
   - `RestirInitialParams`, `RestirTemporalParams`, `RestirSpatialParams`,
     `RestirShadeParams`
   - `PathGuideSampleParams`, `PathGuideUpdateParams`
   - Adaptive variance/allocate params
   - And matching WGSL `struct Params` in 7 shaders.
2. Remap `pixel_id = (params.tile_y + gid.y) * params.full_width +
   (params.tile_x + gid.x)` everywhere; replace `params.width` with
   `params.full_width` for indexing reservoirs / neighbors.
3. Route per-tile param uploads through the same dynamic-offset (or
   `encoder.copy_buffer_to_buffer` from a pre-filled staging buffer)
   pattern used for wavefront dims/count.
4. Delete the force-disable warnings in `compute.rs::dispatch_wavefront`.
5. Verify with both visual UAT and a megakernel-vs-wf-tiled-with-ReSTIR
   parity test.

The wavefront `gbuffer.wgsl` (used in the ReSTIR primary pass) has the
same tile-local pixel_id pattern at line 45; that needs the same fix.

### What still needs YOUR eyes (UAT)

The 4 prior "needs your eyes" items from the original handoff still
apply (Stage 0.1 slider rebuild, Stage A.1 FPS replay, Stage D.2
denoiser tuning, Stage D.3 BVH refit trace, Smoke 8 2D-GPU). Plus:

- **F.1 visual UAT** — set `WF Tile = 256` (or any non-zero value smaller
  than your viewport) with `Backend = Wavefront` and `Spectral = Off`.
  Before the fix this produced black-with-noise tile borders + only the
  bottom-right corner rendered. After the fix the whole viewport should
  render cleanly, matching `WF Tile = 0` output (just with the same
  small per-tile bookkeeping cost).
- **F.2 visual UAT** — `Backend = Wavefront` + `Spectral = Hero` should
  no longer print "Spectral backend stub: forcing megakernel path" in
  the console; the rendered image should show transmission tinting on
  glass-like materials.

---

This file is the cross-machine handoff: WSL2 (where this work was done)
→ Windows (where you continue).

## TL;DR

Two productive coding sessions landed across `2026-05-09 → 2026-05-10`.
**29 commits**, **32 unit tests passing**, **`cargo clippy -- -D warnings`
clean**. The build runs vanilla on a clean machine (no PATH gymnastics
needed since 2026-05-10 conda gcc downgrade).

What's left to do is **runtime / visual verification** that I can't do
from a sub-agent context — see "Open work for you" below.

---

## Where we are

### Project state (sprint-1 + sprint-2 + sprint-3)

| Stage | Status |
|-------|--------|
| 0.1 UI raw-pointer | ✅ pattern is the fix (SAFETY comments + disciplined `&mut self` scope) |
| 0.2 NTFS fallback | ✅ no longer mutates scanner_mode |
| 0.3 SSOT tests | ✅ 32 unit tests across 6+ files |
| A material migration | ✅ shipped (Steps 1–8 in code; Step 9 visual diff = your UAT) |
| B.1 render-3d split | ✅ MaterialCache + collect_recursive + helpers + render_to_view + cpu_pick → submodules |
| B.2 unwrap → expect | ✅ 17 sites with diagnostic messages (full typestate disqualified by lifecycle analysis) |
| B.3 app/mod.rs split | ✅ scan_orchestration + render_loop + screenshot |
| B.4 CLI applicator | ✅ src/app/cli_apply.rs |
| C.1 open::that errors | ✅ shell_open() helper |
| C.2 GPU adapter logging | ✅ log::error! on adapter/device failure |
| C.3 dead_code allows | ✅ removed 4 blanket allows (nothing was actually dead) |
| C.4 SAFETY annotations | ✅ 16 unsafe blocks in scanner_ntfs.rs |
| C.5 treemap debug_assert | ✅ + tests for layout/disjoint |
| C.6 PT backend canonical | ⏳ **your decision** (wavefront vs megakernel canonical) |
| D.1 zero-copy 2D-GPU | ✅ shipped, mirrors 3D zero-copy path |
| D.2 PT denoiser | ✅ à-trous shipped + dedicated UI tab; **visual tuning is on you** |
| D.3 BVH refit gating | ✅ code-verified; runtime trace = your UAT |
| D.4 allocator benchmark | ⏳ your runtime |
| E.1 CI workflow | ✅ Linux + Windows matrix |
| E.2 cargo-audit | ✅ in CI |
| E.3 gitnexus embeddings | ⏳ blocked by Kùzu/ONNX ABI conflicts (Linux-specific, may work on Windows) |

### Modularization

| File | LOC | Note |
|------|----:|------|
| `crates/render-3d/src/lib.rs` | 1341 | was 2335 (sprint-1 start) — −43% |
| `src/main.rs` | 159 | was 1102 — CLI moved to src/cli.rs |
| `src/app/mod.rs` | 716 | was 1521 — split into 4 submodules |
| `crates/render-3d/src/pt/megakernel/` | 26+579+483 | was a single 1073-LOC file |
| `crates/pt-megakernel/src/compute.rs` | 3722 | unchanged — too risky without runtime UAT |
| `src/scanner_ntfs.rs` | 973 | unchanged — single concern, splitting would harm |

---

## Build setup on Windows

This project should build cleanly on Windows with stock Rust + MSVC.
The Linux/conda-forge GCC issues are environmental and don't apply.

### Prerequisites

1. **Rust toolchain** (matches `rust-toolchain.toml`, currently 1.95.0):
   ```powershell
   # If rustup is installed, this auto-resolves from rust-toolchain.toml.
   rustup show
   ```
   If you don't have rustup yet: https://rustup.rs

2. **MSVC build tools** (for `wgpu` and the `windows` crate):
   - Install "Visual Studio 2022 Build Tools" → "Desktop development with C++"
   - Or `winget install Microsoft.VisualStudio.2022.BuildTools`

3. **Git** — `winget install Git.Git` if not present.

### First build

```powershell
cd path\to\dirstat-rs
cargo build --workspace --all-targets
cargo test --workspace
cargo clippy --workspace --all-targets -- -D warnings
```

Expect a cold build of ~5–15 minutes (wgpu, eframe, egui pull a lot).
After the first build, target/ cache makes subsequent builds fast (1–6s).

### Run the app

```powershell
cargo run --release -- C:\path\to\some\dir
```

For UAT, you'll typically want logs:

```powershell
$env:RUST_LOG = "render_3d=debug,pt_megakernel=info"
cargo run --release -- C:\
```

To clear: `Remove-Item Env:RUST_LOG`.

### Run a single test crate

```powershell
cargo test -p pt-mats              # the 9 classify_path_filtered tests
cargo test -p treemap              # the 5 squarified-layout tests
cargo test -p render-3d            # the 8 ray-AABB tests
cargo test -p render-shared        # 2 serde round-trip tests
```

### Windows-specific code paths

`src/scanner_ntfs.rs` (936 LOC, 16 `unsafe` Win32 FFI blocks) only
compiles under `cfg(windows)`. Now is when this code actually runs —
the NTFS fast-path scan via `FSCTL_ENUM_USN_DATA`. If the scan picks
NTFS mode and fails, it falls back to jwalk gracefully (commit
`ce6ae3c` — no longer silently flips your saved preference).

### Note on `auto-allocator = "*"`

It's a wildcard version on purpose (track latest). On Windows the MSVC
toolchain doesn't have the C23/`ATOMIC_VAR_INIT` issue Linux had, so
mimalloc compiles cleanly. The `cargo audit` job in CI is the safety
net for any breaking version drift.

---

## Open work for you

These need actual runtime / your eyes / your call. None can be done
from a sub-agent context.

### High priority — Stage 0.1 + A.1 + D.2 visual UAT

These three close out major in-flight work.

**Stage 0.1 — slider rebuild test (3 min)**
1. `$env:RUST_LOG="render_3d=debug"; cargo run --release -- C:\`
2. Mode 3D (no PT needed)
3. Settings → Rendering → drag the **Materialize Mix** slider
4. **Expected**: console does NOT print `collect_cubes rebuild #N` lines
   from slider movement (only on first frame / scene change)
5. **Failure mode**: every slider tick adds a new `rebuild #N` line →
   Stage A migration regression, the shader-side `mat_global` UBO is
   not actually feeding the blend.

**Stage A.1 — FPS replay (5 min)**
1. PT ON, Animate ON, varying Materialize Mode
2. Read `avg FPS:` from the status bar
3. Record:
   - animate ON × PT ON × Materialize=None: `___`
   - animate ON × PT ON × Materialize=ByExtension: `___`
   - animate ON × PT ON × Materialize=ByPath: `___`
4. **Expected**: ≤ 5% variance — the Stage A claim is "materialize_mix
   moved to shader, no CPU rebuild cost".
5. **Failure mode**: > 10% drop with Materialize ON → Step 7 of the
   migration (drop CPU `color_f` blend) didn't fully take effect.

**Stage D.2 — denoiser tuning (5–10 min, please report back)**
1. PT ON, wait for visible noise (status: samples > 1)
2. Settings → **Denoise** tab → click **Balanced**
3. Image should clean up immediately
4. Try **Conservative** (light), **Aggressive** (heavy)
5. **Tell me** if defaults are bad: does Balanced look right? Does
   Aggressive smear edges visibly? Is Color Sigma slider responsive?
6. If it crashes / shows black / broken: console will have a `WARN`
   line with `wgpu uncaptured error` — copy that exactly.

**Architectural reminder**: the denoiser MVP is color-only edge
stopping. G-buffer guidance (normal/depth) is Stage D.2.b, deferred —
the wavefront PT already produces a G-buffer at
`crates/pt-megakernel/src/wavefront/gbuffer.wgsl`, so plumbing it into
`crates/pt-megakernel/src/denoiser/atrous.wgsl` is a 1–2 commit
follow-up after you've tuned the basic version.

### Medium priority — Stage D.3 + 8

**Stage D.3 — BVH refit trace (3 min)**
1. `$env:RUST_LOG="bvh_gpu=info"; cargo run --release -- C:\`
2. Mode 3D, PT ON, Animate ON, Hash Effect ≠ None
3. Console: should show `refit` lines, NOT `build_gpu` every frame
4. **Failure mode**: every frame logs `GpuBvhBuilder::build_gpu n=...`
   → refit fast-path isn't triggering, animated PT will be slow.

**Smoke test 8 — 2D-GPU zero-copy (2 min)**
1. Mode 2D, Backend GPU
2. Treemap renders correctly
3. **Failure mode**: black screen / wrong colors / wgpu validation
   error → Stage D.1 broken.

### Lower priority — Stage D.4 + C.6

**Stage D.4 — allocator benchmark (15 min)**
- Time `cargo run --release -- C:\large\dir` with `auto-allocator`'s
  `secure` feature ON (current default) vs OFF.
- Decide: keep `secure`, drop, or feature-gate per-arch.

**Stage C.6 — PT backend policy (decision, not runtime)**
- Decide: "wavefront is canonical, megakernel is fast-path for simple
  scenes" OR the inverse.
- I'll write the docs commit once you tell me which way.

### E.3 retry on Windows (optional)

The Linux ABI block (Kùzu VECTOR + ONNX/Bun) likely doesn't apply on
Windows. Try:

```powershell
npm install -g gitnexus@latest
npx gitnexus analyze --embeddings --force
```

If `embeddings: <non-zero>` afterwards in `.gitnexus/meta.json`, it
worked. Otherwise same blockers — defer.

---

## Known issues & workarounds

### Resolved on Linux but irrelevant on Windows

- conda-forge GCC 15 + `ATOMIC_VAR_INIT` C23 removal — fixed by
  downgrading conda gcc to 13.4 on 2026-05-10. Windows uses MSVC, so
  this never applied.

### Still open

- **Kùzu VECTOR extension** undefined-symbol on Linux — upstream
  gitnexus issue. Try latest gitnexus on Windows; binaries may be
  re-built for Windows MSVC and work there.
- **ONNX runtime via Bun** segfault on Linux — Bun is Linux-installed.
  On Windows `npx` will use Node, which loads the napi binding fine.
  E.3 may "just work" on Windows.
- **GitHub Actions CI** has a Windows runner but I haven't watched a
  green build go through end-to-end. First push of any future PR will
  prove or fail this. Workflow: `.github/workflows/ci.yml`.

---

## Repo structure pointers

Where to look for what:

```
src/
  main.rs                 — fn main() only (159 LOC)
  cli.rs                  — CliOptions struct + parse_args + print_help (954 LOC)
  cli_test.rs             — `dirstat-rs test ...` headless harness
  scanner.rs              — jwalk parallel scan
  scanner_ntfs.rs         — Windows MFT fast scan (cfg(windows))
  cache.rs                — bincode tree cache
  exclusions.rs           — .dirstat-exclusions.json persistence
  events.rs               — type-erased event bus
  path_key.rs             — sha256 cache keys
  renderer.rs             — re-exports + binary-side render enums
  app/
    mod.rs                — App impl + render_treemap dispatch (716 LOC)
    state.rs              — App struct + PersistState + SettingsTab
    scan_orchestration.rs — start/stop/poll_scan
    render_loop.rs        — run_frame + handle_events + dock visibility
    screenshot.rs         — handle_screenshot + capture_viewport + save_png
    cli_apply.rs          — apply_cli_overrides(&mut Render3DOptions, &CliOptions)
    dock.rs               — egui_dock layout
    toolbar.rs / status_bar.rs / tree_panel.rs / ext_panel.rs / treemap_view.rs
    filters.rs            — tree filter / mask / glob / size-range / LoD merge
    helpers.rs            — compute_ext_stats, find_node_by_path, fmt_size
    shell.rs              — OS shell ops + shell_open() helper
    presets.rs            — settings preset save/load
    settings/
      mod.rs              — settings panel + tab dispatch
      view.rs             — View / Layout / LoD
      appearance.rs       — colors / fonts
      scanner.rs          — Scanner mode + path
      renderer.rs         — Render3DOptions sliders
      denoiser.rs         — NEW Stage D.2 denoiser controls
      exclusions.rs       — .dirstat-exclusions UI

crates/
  dirstat-core/           — DirEntry tree model
  treemap/                — squarified layout + CPU/GPU rasterizer
  render-core/            — GpuContext + Viewport
  render-shared/          — Render3DOptions + enums + uniforms
  bvh-gpu/                — GPU LBVH build (Morton + radix sort + Karras)
  pt-core/                — PT scene + CPU SAH BVH + GpuMaterial
  pt-mats/                — material classifier + library
  pt-wavefront/           — wavefront PT pipelines + WGSL stages
  pt-megakernel/          — megakernel PT + ReSTIR + path-guide + adaptive
    src/denoiser/         — NEW Stage D.2 (atrous.wgsl + pipeline.rs)
  render-3d/              — integrated 3D renderer (PBR + PT dispatch)
    src/renderer3d/       — submodules from sprint-2/sprint-3 splits
      mod.rs              — declarations
      material_cache.rs   — Stage A material classification cache
      instance_collect.rs — collect_cubes + collect_recursive
      helpers.rs          — lerp, hash, kelvin_to_rgb, mix_material, slice
      render.rs           — render_to_view + render_path_traced_no_readback
      cpu_pick.rs         — cpu_pick + pick_recursive + ray_aabb_intersect tests
    src/pt/megakernel/    — PT backend dispatchers (mod / render / render_no_readback)
```

### Key planning docs

- `TODO4.md` — validated roadmap (rev 5). All stages tracked with
  honest status indicators.
- `CHANGELOG.md` — sprint-1, sprint-2, sprint-3 entries with
  Added/Changed/Removed/Fixed sections.
- `CONCERNS.md` — original 2026-05-09 audit + post-sprint-2 status
  header showing what's resolved vs open.
- `.planning/codebase/{ARCHITECTURE,STRUCTURE,TESTING,STACK,...}.md`
  — codebase-mapper output, refreshed in sprint-3.

### Project-level CLAUDE.md

`CLAUDE.md` at repo root has gitnexus integration mandates. Not
strictly necessary if you're not using gitnexus from this checkout,
but useful if you want impact analysis before edits.

---

## Quick reference

```powershell
# Standard dev loop
cargo build --workspace --all-targets
cargo test --workspace
cargo clippy --workspace --all-targets -- -D warnings

# Run with logging
$env:RUST_LOG = "render_3d=debug,pt_megakernel=info"
cargo run --release -- C:\

# Push (when ready)
git push origin main

# Status
git log --oneline -5
git status --short
```

---

## What I'd do first if I were you on Windows

1. `cargo build --workspace --all-targets` — proves the codebase
   compiles cleanly on a fresh OS/toolchain. ~10 min cold.
2. `cargo test --workspace` — confirms the 32 unit tests still pass.
3. Quick smoke run: `cargo run --release -- C:\` — make sure the app
   starts and shows treemap.
4. Then go down the UAT list above (D.2 denoiser tuning is most
   interesting / actionable).

If anything in steps 1–3 fails, paste the error and we figure it out.
If 1–3 work, the project is genuinely portable across the WSL/Windows
boundary and the code-only work landed correctly.

---

*Last updated: 2026-05-10. If you read this much later, run
`git log --oneline origin/main..HEAD` to see what's diverged.*
