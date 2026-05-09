# Codebase Concerns

**Analysis Date:** 2026-05-09

This document audits technical debt, fragility, and risk surfaces in the
`dirstat-rs` workspace. Concerns are grouped by category. Each item cites file
paths (and line numbers where useful) so future planning can navigate directly.

---

## Hot spots — large monolithic files

The following files are oversized for clean review and concentrate most of the
project's churn:

| File | Lines | Concern |
|------|------:|---------|
| `crates/render-3d/src/lib.rs` | **2309** | Renderer3D god-object: scene rebuild, instance caching, PBR/wireframe/object-id pipelines, PT orchestration, material cache. Touched ~60x recently per project memory. |
| `src/app/mod.rs` | **1518** | UI core: scan orchestration, render loop, docking, screenshot, event pump interleaved (already flagged in `plan1.md` F3). |
| `src/main.rs` | **1077** | CLI parsing + bootstrap; mirrors many `Render3DOptions` fields (drift risk — `plan1.md` F4). |
| `crates/render-3d/src/pt/megakernel.rs` | **1067** | PT backend orchestration; second active churn point per project memory. |
| `src/scanner_ntfs.rs` | **936** | Windows MFT/USN enumeration; 16 `unsafe` blocks (see below). |
| `crates/treemap/src/lib.rs` | **823** | CPU treemap rasterizer with `unsafe` parallel write. |
| `crates/treemap/src/wgpu.rs` | **608** | GPU treemap path. |

**Risk:** Every change to `render-3d/src/lib.rs` risks merge conflicts and
makes review hard. The plan in `TODO2.md` explicitly notes "MaterialCache
helpers inside lib.rs" — adding *more* surface to an already-too-large file.

**Fix approach:** Mechanical extraction (no behavior change) into:
- `renderer3d/material_cache.rs` (new MaterialCache + settings hash)
- `renderer3d/instance_collect.rs` (the `collect_recursive` + cube building)
- `renderer3d/pipelines.rs` already exists — push more init code into it.

---

## Unfinished / in-progress work

Active threads extracted from `TODO.md`, `TODO2.md`, `plan1.md`, `task.md`:

### Path tracer quality (`TODO.md`)

Most items are checked off (NEE for emissive cubes, full MIS, ReSTIR DI,
low-discrepancy sampling, adaptive sampling, firefly filter, env MIS audit,
material lobe PDF audit, wavefront parity).

**Open:**
- **Denoising — paused.** Need normal/depth/albedo G-buffers and SVGF/à-trous
  filtering with variance guidance. Listed as "paused for now; finish
  non-denoiser fixes first." Without a denoiser the PT output remains noisy at
  interactive sample counts → users see grain.
- **ReSTIR is wavefront-only**; megakernel has parity gaps. UI marks the scope
  but the dual-backend split adds maintenance cost (see "Dead/duplicate code
  suspicions" below).

### Unified material system (`TODO2.md`)

A 9-step migration plan to thread `material_id` per cube and move the
`materialize_mix` blend from CPU into the shader. Step 0 done (file written).
Step 1+ status from code:
- `crates/render-3d/src/lib.rs:121` shows `MaterialCache::classify_or_get` is
  in place — Steps 2/3 partially landed.
- `classify_path_filtered` is still called in `crates/pt-mats/src/lib.rs:621`
  and re-exported / referenced from `crates/render-3d/src/lib.rs:25`,
  consistent with the cache funneling through it.
- Steps 5–8 (GPU `materials_buf`, switching `cube_pbr.wgsl`, dropping CPU
  `color_f` material blend, removing dead `MaterialParams` UBO) are not yet
  evidenced and remain pending work.

**Risk:** The migration has 9 sequential steps; partial completion leaves
duplicate state (CPU lerp + GPU blend) and a fragile mid-state.

### Bug-hunt deferrals (`plan1.md`)

D1–D5 are flagged but not actioned:
- **D1: NTFS fallback persists Standard mode silently** — see "Platform
  fragility" below. Highest UX-risk item.
- **D2: Pin `auto-allocator`** — see "Dependency concerns".
- **D3: Split `app/mod.rs`** — see "Hot spots".
- **D4: Single `CliOptions → Render3DOptions` applicator.**
- **D5: Zero-copy / texture reuse with eframe** — referenced by the only two
  `TODO` markers in source (`src/app/mod.rs:1035`, `:1068`).

### Visual regression (`task.md`)

`task.md` is one Russian sentence reporting that recent changes "thinned out"
the geometry — gaps now appear between all cubes. This is an **active visual
regression** with no investigation notes attached. Likely related to either
the `CubeInstance` layout extension in TODO2.md Step 1 (vertex attribute slot
9 added) or the model matrix scale in `collect_recursive`. Untriaged.

---

## `unsafe` blocks

**28 occurrences across 7 files.** Concentrated in two areas:

### Win32 FFI (necessary, but auditable)

`src/scanner_ntfs.rs` — 16 `unsafe` blocks for `CreateFileW`,
`DeviceIoControl`, raw FILE_ID_BOTH_DIR_INFO walking, NTFS USN/MFT IOCTLs:

- `:39, :57, :84` — directory handle open + GetFileInformationByHandleEx loop.
- `:322, :355, :362, :383, :417` — first MFT/USN scan path.
- `:451, :495, :515, :576` — second IOCTL path.
- `:619, :654, :672, :697` — third IOCTL path.

These are textbook Win32 wrappers; the risk is low if the buffer-size and
record-walking arithmetic is correct, but the file has no `// SAFETY:`
comments documenting invariants. **Recommend** annotating each block.

### POSIX FFI

`src/app/helpers.rs:220, :253, :255` — `libc::statvfs` for free-space query.
Small surface, idiomatic. `MaybeUninit::assume_init()` after a successful
return code is correct.

### Aliased raw pointers in UI

`src/app/treemap_view.rs:978`, `src/app/mod.rs:1186, :1197, :1222`,
`src/app/tree_panel.rs:115, :219, :226` — all of the form
`let root = unsafe { &*root_ptr };` to dereference a stored `*const` of the
scan tree root. **Risk:** classic single-threaded "I know this pointer is
valid" pattern. If the underlying tree is dropped or rebuilt while a UI panel
holds the raw pointer, this is a use-after-free. **Recommend** replacing with
`Arc<DirEntry>` or a generation counter; this is the highest-risk `unsafe`
class in the codebase (more than the Win32 FFI).

### Parallel raw write

`crates/treemap/src/lib.rs:507` — uses `*mut u8` from a slice base pointer
inside `par_iter().for_each` to write disjoint pixel rectangles. Comment
asserts rects don't overlap, but there is no debug assertion enforcing it. A
future change to layout could silently introduce overlap → data race UB.
**Recommend** swap to `chunks_mut` row-by-row, or add a `debug_assert!` that
rectangles are disjoint.

---

## `unwrap()` / `expect()` density

**67 `.unwrap()` calls across 15 files; 9 `.expect(...)` across 4 files.**

Density is highest in `crates/render-3d/src/lib.rs` (15 unwraps), all of the
form `self.cached_instances.as_ref().unwrap()` /
`self.targets.as_ref().unwrap()` / `self.dyn_bgs.as_ref().unwrap()` /
`self.instance_buffer.as_ref().unwrap()` (e.g. lines 741, 869–871, 944,
2088, 2210). Examples:

- `crates/render-3d/src/lib.rs:741` — `Arc::clone(self.cached_instances.as_ref().unwrap())`
- `crates/render-3d/src/lib.rs:869–871` — three sequential `as_ref().unwrap()` for `targets`, `dyn_bgs`, `instance_buffer`.
- `crates/render-3d/src/lib.rs:2210–2211` — repeats the targets/dyn_bgs unwrap pair.

These encode "this method is only valid after init" without typing it.
**Risk:** A future caller invoking these methods before `init_pipelines` or
before the first scene build will panic in production. **Fix approach:** wrap
the lazily-built state in a single `RendererInited` substruct and gate the
methods on `&mut self.inited.as_ref()?`, or split `Renderer3D` into
`Uninit`/`Ready` states.

Other counts (no individual concern, but noted):
- `crates/pt-megakernel`: 15 unwraps (mostly post-`create_shader_module` /
  pipeline build — fail-fast is acceptable but message quality varies).
- `src/app/mod.rs`: 0 unwraps (good).
- `src/scanner_ntfs.rs`: 0 unwraps (good — uses `?` and `anyhow`).
- `src/app/treemap_view.rs`: 2 unwraps.
- `src/app/filters.rs`: 1 unwrap.
- `crates/render-shared/src/lib.rs`: 5 expects (in tests — fine).

`panic!` / `todo!` / `unimplemented!` / `unreachable!`: **none** in source
code. Good — no parking-lot stubs hidden in the binary.

---

## TODO/FIXME/XXX/HACK markers

Only **2 in source**, both in `src/app/mod.rs`:

- `src/app/mod.rs:1035` — "Zero-copy rendering requires using eframe's device
  for all rendering"
- `src/app/mod.rs:1068` — "Zero-copy path disabled - needs double-buffering to
  avoid blocking"

Both refer to the same concern (`plan1.md` F6, D5): the 2D treemap GPU path
allocates a `ColorImage` + `load_texture` per frame because we cannot share
eframe's `wgpu::Device` from the custom render path without race risk.

**Risk:** Per-frame heap allocation + GPU upload of full-resolution images is
CPU/GPU-bound; visible at high resolutions or fast resize.

**Fix approach:** Two milestones:
1. Share eframe's wgpu device with the renderer (already partially noted in
   the plan).
2. Add a 2-buffer texture pool for ping-pong upload.

The low marker count is misleading — actual unfinished work is documented in
the standalone `TODO.md` / `TODO2.md` / `plan1.md` files instead.

---

## Performance-sensitive areas

### Two PT backends (megakernel vs wavefront)

`crates/pt-megakernel/` and `crates/pt-wavefront/` are both maintained.
`render-3d` depends on both (`crates/render-3d/Cargo.toml:17–18` only lists
`pt-megakernel` and `pt-mats`, but `pt-megakernel` re-exports / chains to
`pt-wavefront`). Tradeoffs that need a documented decision:

- Megakernel wins on simple scenes / few materials (one shader, GPU
  scheduler-friendly).
- Wavefront wins on heavy divergence and is the **only** backend with ReSTIR
  (per `TODO.md` line 18).
- Maintaining both adds 2 sets of pipeline init, 2 sets of `#![allow(dead_code)]`
  belts (`crates/pt-wavefront/src/wavefront/pipeline.rs:2`,
  `crates/pt-megakernel/src/pathguide/pipeline.rs:2`,
  `crates/pt-megakernel/src/adaptive/pipeline.rs:2`,
  `crates/pt-megakernel/src/restir/pipeline.rs:2`).

**Risk:** Feature drift (ReSTIR only on one side, denoiser will land on
whichever is convenient → inconsistency).

**Fix approach:** Document a "wavefront is canonical, megakernel is the
fast-path for simple scenes" policy, and gate megakernel-only/wavefront-only
UI controls explicitly (already partially done per `TODO.md` line 52).

### BVH build cost

`crates/bvh-gpu/` builds the BVH on CPU (depends on `pt-core`). Per-frame
rebuild is gated by `pt_scene_dirty` in `crates/render-3d/src/pt/megakernel.rs`
— good. **Risk:** With `opts.animate=true` in the PBR path,
`cached_instances` invalidates every frame (`TODO2.md` line 21); if PT
likewise rebuilds the BVH on every animated frame, the path tracer is
unusable while animation is on. Verify the animation gating on PT scene
dirtiness; a TRS animation should not require a BVH rebuild if cube count is
stable.

### `jwalk` parallel scan

`src/scanner.rs` uses `jwalk = "0.8"` with `.follow_links(false)` (line 80).
Symlink loops are not a concern, but jwalk's parallel walker fights with the
NTFS path's internal queue when both are active. **Risk:** None today (mode
is exclusive), but if a future "auto-mode" tries both, contention is real.

### Stable-path treemap upload

`src/app/mod.rs` ~L1035–L1074 — per-frame `ColorImage::from_rgba_unmultiplied`
+ `ctx.load_texture`. See "TODO markers" above.

---

## Platform fragility

### Windows-only NTFS path

`src/scanner_ntfs.rs` (936 lines, 16 unsafe FFI calls) is a complete
parallel scanner gated on `cfg(windows)`. **Concern F1 from `plan1.md`** is
unresolved: on MFT failure, `src/app/mod.rs:619–623` silently flips
`scanner_mode = ScannerMode::Standard`, which is then persisted on next save
— the user loses their NTFS preference without consent.

**Fix approach:** introduce `ntfs_last_error: Option<String>` as a transient
flag; do NOT mutate `scanner_mode` on fallback. Show the fallback as a
non-modal banner.

### Cross-platform free-space probe

`src/app/helpers.rs` uses `libc::statvfs` on Unix and a `windows`-crate path
on Windows; two code paths to keep in sync. Currently small.

### Trash/open shell integration

- `trash::delete` at `src/app/shell.rs:187` — relies on shell COM on Windows
  / `gio trash` (or equivalents) on Linux. **Risk:** silent failure modes
  vary; only one error path logs.
- `open::that` at `src/app/shell.rs:94, :100`, `src/app/treemap_view.rs:799,
  :804`, `src/app/mod.rs:1319` — return values are discarded with `let _ =
  ...`. **Risk:** users get no feedback when "Open" silently fails (e.g. no
  registered handler, or path with unusual characters on Linux).

**Fix approach:** Capture and surface the error in a status bar.

### File dialog `rfd`

Pulled in at root `Cargo.toml:37`. On Linux requires either GTK or
`xdg-desktop-portal`; on minimal containers this fails at runtime.

---

## GPU / WGSL fragility

- `pt-megakernel` calls `create_shader_module` 8x and `pt-wavefront` /
  adaptive / pathguide / restir pipelines another ~20x (25 wgsl/shader
  references total). Each is a panic surface if the shader fails to compile
  on the user's GPU/driver combo. WGSL validation differs subtly between
  Vulkan, DX12, Metal — no fallback / capability check is evidenced.
- `crates/render-core/src/lib.rs:90` uses `wgpu::Backends::all()` with no
  per-backend probe — first adapter wins. On Windows machines with both DX12
  and Vulkan installed, behavior depends on enumeration order.
- `pollster::block_on(instance.request_adapter(...))` at
  `crates/render-core/src/lib.rs:94` blocks the calling thread; on a system
  where adapter selection stalls (e.g. driver bug) the UI thread freezes.
- No graceful "no compatible adapter" message is wired through from
  `request_adapter` — failure path needs verification.

**Fix approach:** Add adapter probing with a preferred-backend list (DX12 →
Vulkan → Metal → GL), explicit logging of the selected adapter (already
partially done), and a user-visible error when `request_adapter` returns
None.

---

## Dependency concerns

### `auto-allocator = "*"` (root `Cargo.toml:23`)

Wildcard major version. Already flagged as `plan1.md` F2/D2.
**Risk:** Reproducible builds break when upstream publishes a major version
(any breaking change immediately leaks into our build). Lockfile mitigates
for committed-lock workflows but not for fresh `cargo update`.

The crate is loaded with the **`secure`** feature, which enables hardened
allocator paths (zeroing, guard pages on some allocators). **Implication:**
non-trivial perf cost on hot allocation paths (the scanner itself is
allocation-heavy via jwalk). Worth measuring whether `secure` is justified
for a local directory-stat tool that never handles secrets in memory.

**Fix approach:** Pin to a semver range (e.g. `"0.1"` or current major), and
revisit the `secure` feature in a benchmark phase.

### Other dependencies — no concerns

`wgpu = "29"`, `eframe = "0.34"`, `egui = "0.34"` are all consistent.
`bincode = "1"` (legacy v1 API; bincode 2.x has rewritten the API, but v1 is
still supported). `directories = "6"`, `rfd = "0.17"`, `trash = "5"`,
`open = "5"`, `sha2 = "0.11"` — all current.

### `dead_code` belts in PT crates

`#![allow(dead_code)]` at:
- `crates/pt-wavefront/src/wavefront/pipeline.rs:2`
- `crates/pt-megakernel/src/pathguide/pipeline.rs:2`
- `crates/pt-megakernel/src/adaptive/pipeline.rs:2`
- `crates/pt-megakernel/src/restir/pipeline.rs:2`

Same as `plan1.md` F5. **Risk:** masks the difference between
"intentionally-unused helper for feature X" and "actual dead code from a
half-done refactor." Hard to spot rot.

---

## Build / CI gaps

**No `.github/` directory exists** in the repo (confirmed via Glob). There
is no GitHub Actions workflow, no automated `cargo check`, no `clippy`, no
`cargo test`, no `cargo audit`, no Windows/Linux/macOS matrix.

**Risk:** All quality is enforced manually. Recent history shows
WIP commits dominating (`d9b0f2f`, `ec08ace`, `88d0b0c`, `7e506f0`,
`98543fe` are all `chore: WIP …` per the prompt). Without CI, regressions
land silently.

**Fix approach:** Minimum viable CI:
- `cargo check --workspace` on Linux + Windows.
- `cargo clippy --workspace --all-targets -- -D warnings`.
- `cargo test --workspace` (the only existing tests appear to be in
  `render-shared` per `plan1.md` table).
- `cargo deny` or `cargo audit` weekly for the `*` wildcard concern.

---

## Security surface

| Area | Surface | Concern |
|------|---------|---------|
| FS walking | `jwalk` with `.follow_links(false)` (`src/scanner.rs:80`) | Symlink loops avoided. Good. |
| Symlink traversal | NTFS path | Reads MFT directly; bypasses any reparse-point loops. Good. |
| Trash | `trash::delete` (`src/app/shell.rs:187`) | Per-OS shell. Failure handling is one log. Acceptable. |
| Open external | `open::that` 5 call sites | Return value discarded. Path is user-controlled (selected from scan). On Linux `xdg-open` honors `.desktop` files — running a malicious `.desktop` from a scanned directory is a known attack class. **Mitigation:** the call uses the platform "open" semantic, which on Linux is `xdg-open` — verify it does not directly exec `.desktop` files; consider whitelisting MIME categories. |
| sha2 | `src/path_key.rs:7` | SHA-256 used for cache keys (not security-critical). Acceptable. |
| GPU shader source | All `.wgsl` files | Bundled, not user-controlled. No injection risk. |
| Env / secrets | None — no `.env` file in repo. | OK. |

**Highest-risk item:** `open::that` on user-selected file/directory paths
without whitelisting. Low likelihood, high impact (arbitrary file open with
the OS handler).

---

## Dead / duplicate code suspicions

### Multiple `pt-*` crates — overlap?

Workspace has **5** PT-related crates plus `render-3d`:

| Crate | Lines (rs files) | Purpose |
|-------|-----------------:|---------|
| `pt-core` | small lib (no wgpu) | Types: `Instance`, BVH data, `GpuMaterial`. |
| `pt-mats` | ~800 | `MaterialClass`, `MaterializeMode`, `MaterialLibrary`, `classify_path_filtered`. CPU side. |
| `bvh-gpu` | small | GPU BVH upload helpers. Depends on `pt-core`. |
| `pt-wavefront` | 5 files | Wavefront PT pipelines. |
| `pt-megakernel` | 13 files | Megakernel + adaptive + ReSTIR + path-guide pipelines. Depends on `pt-wavefront`. |
| `crates/render-3d/src/pt/megakernel.rs` | 1067 | Renderer3D-side orchestration that *uses* `pt-megakernel`. |

**Concerns:**

1. `pt-megakernel` depending on `pt-wavefront` (line 13 of its Cargo.toml) is
   surprising — typically the megakernel is the simpler standalone path.
   Either there is shared infrastructure that should live in `pt-core`, or
   the dependency direction is wrong. **Investigate.**
2. `crates/render-3d/src/pt/megakernel.rs` (1067 lines) is roughly the size
   of the entire `pt-megakernel` crate. The orchestration / glue is so heavy
   it competes with the backend itself for line count. Likely candidates to
   move into `pt-megakernel`: PT camera uniform building, instance GPU data
   marshalling, environment CDF setup (line 45 area).
3. `pathguide/` directory exists (`pt-megakernel/src/pathguide/`) but is not
   mentioned anywhere in the active TODOs. It is silently masked by
   `#![allow(dead_code)]` at `pathguide/pipeline.rs:2`. **Likely
   experimental/unfinished and should either be feature-gated or removed.**
4. `adaptive/` directory same situation (`adaptive/pipeline.rs:2`
   `#![allow(dead_code)]`). The adaptive sampler is referenced in `TODO.md`
   as completed, so this allow is suspect — it may be hiding stale code from
   the previous adaptive iteration.

**Fix approach:** Audit `pathguide` and `adaptive` modules — either
feature-gate them and remove the blanket `dead_code` allow, or delete what is
not referenced from any pipeline.

---

## Test coverage gaps

Per `plan1.md`: only `render-shared` has tests at all (2 tests). No tests
were found in:

- `dirstat-core` — the canonical `DirEntry` shape. **Risk:**
  `DirEntry::lod_expand` semantics are referenced as the SSOT but are
  unverified.
- `pt-mats` — `classify_path_filtered` is the central material decision
  function. Untested.
- `pt-core` BVH builder — geometric correctness is structural, not
  ray-traced; a bug in build orientation/order is invisible until the
  visual output looks wrong.
- `dirstat-rs` (root binary) — scanner_ntfs MFT parsing. **High-risk
  untested area** (Windows-only, FFI-heavy, 16 unsafe blocks).
- `treemap` layout — squarified treemap algorithm is non-trivial; a
  regression like the one in `task.md` ("gaps between cubes") may originate
  here and is currently undetectable in CI.

**Priority:** High for `dirstat-core::lod_expand`, `pt-mats::classify_*`, and
treemap layout — these are pure-CPU functions that are cheap to test and
underpin both visual correctness and "did we break the scan tree."

---

## Summary — top concerns to plan for

1. **Visual regression** in `task.md` (cube gaps) — untriaged, blocks
   user-visible quality.
2. **`render-3d/src/lib.rs` (2309 LOC)** — extract MaterialCache and instance
   collection into separate modules before adding more state from `TODO2.md`.
3. **NTFS fallback persists silently** (`plan1.md` D1) — UX bug, easy fix.
4. **`auto-allocator = "*"`** with `secure` feature — pin and benchmark.
5. **No CI** — add minimum viable workflow before more refactors.
6. **`pathguide/` and `adaptive/` dead-code allows** — audit, gate, or delete.
7. **Raw-pointer aliasing in UI panels** (`tree_panel.rs`, `treemap_view.rs`,
   `app/mod.rs`) — replace with `Arc<DirEntry>`.
8. **Test coverage** for `dirstat-core::lod_expand`, `pt-mats::classify_*`,
   treemap layout.
9. **`open::that` discarded errors** — surface failures to user.
10. **Two PT backends maintained** — document policy and gate UI controls.

---

*Concerns audit: 2026-05-09*
