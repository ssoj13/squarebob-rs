# Bug Hunt Plan 1 — squarebob-rs whole-crate sweep

**Date:** 2026-05-15
**Branch:** main
**Scope:** all crates + src/app, excluding _ref/, target/, tests
**Method:** grep + manual inspection (gitnexus TUI broken in non-TTY, agents OOM on init context)
**Files scanned:** ~85 Rust source files, ~40k LOC

---

## TL;DR
- 1 GPU readback panic on device-lost (CRITICAL)
- 1 tile-math overflow expect (HIGH)
- 2 ffmpeg FFI unsafe blocks missing `// SAFETY:` (HIGH)
- 30+ `Option<Buffer>::as_ref().unwrap()` lazy-slot pattern in PT hot paths (HIGH — class of bug)
- 9× identical `"targets not built"` expect in render-3d (HIGH — class of bug)
- 3× informal `// Safe:` should be `// SAFETY:` (MED — clippy)
- Several runtime asserts in hot path (MED)
- Some `let _ =`, `.expect`, `let _ =` are intentional / OK

**Clean areas:** scanner_ntfs (16/16 SAFETY coverage), pt-core (0 unwraps), pt-mats numerical guards (`.max(0.001)`), no `.lock().await`, no `todo!()/unimplemented!()`.

---

## Findings (severity-ranked, file:line)

### [CRITICAL] GPU readback double-unwrap on device-lost
- **Loc:** `crates/bvh-gpu/src/bvh_gpu/mod.rs:1478`
- **Cat:** 1, 4
- **Issue:** `rx.recv().unwrap().unwrap();` — outer unwraps `Result<MapAsyncError>` from sender-dropped channel; inner unwraps `Result<(), BufferAsyncError>` from wgpu callback. Both panic on GPU device lost (driver crash, hot-unplug, OOM).
- **Why bad:** BVH readback runs on user frame submission for CPU verification path. Single GPU hiccup → process exit, lost session, no useful error.
- **Fix:** Bubble `Result<Vec<u8>, BvhReadbackError>` from helper. Existing call sites must handle / log device-lost and continue with degraded mode. No catch-and-continue inside this helper — it's a `Result`, not a panic boundary.

### [HIGH] Tile offset overflow expect on extreme tile count
- **Loc:** `crates/pt-wavefront/src/wavefront/pipeline.rs:363`
- **Cat:** 5, 12
- **Issue:** `idx.checked_mul(TILE_SLOT_STRIDE as u32).expect("tile offset overflow")`. With huge resolutions (8K+ × tile_capacity grown beyond u32/STRIDE), this panics inside a per-frame dispatch.
- **Why bad:** Frame-time panic kills wavefront. User just rendering high-res.
- **Fix:** Return `Option<u32>` and let the caller skip the offending tile (or split). Alternatively change `TILE_SLOT_STRIDE` arithmetic to u64 and cast to u32 only at the wgpu boundary with a documented contract: if it doesn't fit in u32, refuse the resolution at dispatch setup.

### [HIGH] ffmpeg FFI unsafe blocks without `// SAFETY:` comment
- **Loc:** `crates/media-encoder/src/dialogs/encode/encode.rs:1402-1404` (`av_log_set_level`)
- **Loc:** `crates/media-encoder/src/dialogs/encode/encode.rs:1658-1661` (raw write through `as_mut_ptr()` to `(*params).codec_tag`)
- **Cat:** 2
- **Issue:** Two `unsafe { ... }` blocks. The second one dereferences a raw pointer and stores into ffmpeg's `AVCodecParameters`. No `// SAFETY:` line — clippy `undocumented_unsafe_blocks` will flag.
- **Why bad:** Pointer is from `parameters().as_mut_ptr()` — caller must guarantee no aliasing & valid lifetime tied to the muxer/stream. That contract belongs above the block.
- **Fix:** Add `// SAFETY:` describing (a) why the raw deref is sound (`ost` outlives this expression, ffmpeg-rust returns valid params), (b) why `av_log_set_level` is reentrant-safe in this init context.

### [HIGH] Lazy-init `Option<wgpu::Resource>` slots accessed via `as_ref().unwrap()` — class of bug
- **Cat:** 1, 8 (DRY)
- **Sites (incomplete — sample):**
  - `crates/bvh-gpu/src/bvh_gpu/mod.rs:528,530,532,534,535,600-606,745,747,847,879`
  - `crates/pt-megakernel/src/compute.rs:2110,2633,2700,2869,2872,3024-3026,3445,3834`
  - `crates/pt-megakernel/src/restir/pipeline.rs:170,171,219,223,227,231`
  - `crates/pt-megakernel/src/pathguide/pipeline.rs:99`
  - `crates/pt-megakernel/src/adaptive/pipeline.rs:93,97`
  - `crates/pt-wavefront/src/wavefront/pipeline.rs:263,264,275,276,286,294,300`
  - `crates/pt-denoise-oidn/src/lib.rs:297,518`
  - `crates/render-3d/src/pt/megakernel/render.rs:52`, `render_no_readback.rs:49`
- **Issue:** Pipelines hold `Option<wgpu::Buffer>` fields. Init code populates them; render code calls `self.foo.as_ref().unwrap()`. Invariant "you must call `ensure_resources()` before any render method" is enforced by panic.
- **Why bad:** Brittle. Refactors easily break this invariant. Hot-frame panic. The compiler could enforce it.
- **Fix (systemic):** Split type. Either
  - **typestate**: `Pipeline<Uninit>` → `ensure_resources` returns `Pipeline<Ready>`; only `Ready` exposes render methods, fields are `wgpu::Buffer` not `Option<...>`.
  - **resource bag**: lift all the `Option<Buffer>` fields into a single `PipelineResources` struct that is always constructed in one go; `Pipeline` holds `Option<PipelineResources>` and renderers receive `&PipelineResources` instead of `&Pipeline`.
  - **getter returning `Result`**: minimum-effort fallback — replace `as_ref().unwrap()` with `.ok_or(Error::NotInitialised)?` and propagate. Still leaves the runtime check, but at least no panic.

  Pick one approach, apply to all sites in one PR (don't spot-fix one slot).

### [HIGH] Identical "targets not built" expect spread across render-3d — class of bug
- **Cat:** 1, 8 (DRY)
- **Sites:**
  - `crates/render-3d/src/lib.rs:519,1310,1314,1323,1330`
  - `crates/render-3d/src/renderer3d/render.rs:85,225,229,230,259,263,275`
  - `crates/render-3d/src/pt/megakernel/render.rs:455`
  - `crates/render-3d/src/pt/megakernel/render_no_readback.rs:394`
- **Issue:** Same string `"targets not built — call ensure_render_targets before render"` copy-pasted 9+ times. Same as PT lazy-slot but on `dyn_bgs`/`cached_instances`/`targets`.
- **Why bad:** Same class as previous. Single grep target if message ever changes.
- **Fix:** Helper `fn require_targets(&self) -> &RenderTargets` returning `&` (or `Result`) once. Better: same typestate fix as above. Centralise the contract.

### [HIGH] `material_cache.rs` pair-invariant by panic
- **Loc:** `crates/render-3d/src/renderer3d/material_cache.rs:304`
- **Cat:** 1, 12
- **Issue:** `let class = class_opt.expect("is_light implies class_opt is Some");` — two parallel optional fields where one being `true` implies the other is `Some`. Encoded as runtime assert.
- **Why bad:** Wrong place to encode invariant. Caller can violate silently.
- **Fix:** Sum type: instead of `(is_light: bool, class_opt: Option<MaterialClass>)`, use `enum MaterialKind { Solid, Light(MaterialClass) }`. Single source of truth.

### [HIGH] Channel-recv unwrap on dropped sender
- **Loc:** `crates/render-3d/src/picking.rs:196`
- **Cat:** 1
- **Issue:** `if let Err(e) = rx.recv().unwrap() { ... }` — outer `.unwrap()` panics if sender dropped (e.g. GPU device lost callback drops without sending).
- **Why bad:** Picking is user-driven (mouse click on cube). One device hiccup during picking → panic.
- **Fix:** Match both outer recv-error and inner mapping error; on outer error, log and return "no pick".

### [MED] Informal `// Safe:` instead of `// SAFETY:`
- **Loc:** `src/app/screenshot.rs:72,83,108`, `src/app/treemap_view.rs:1090` (1× `unsafe { &*root_ptr }` with informal "Safe:"), `src/app/tree_panel.rs:114-115` (uses "Safety:")
- **Cat:** 2
- **Issue:** Clippy lint `undocumented_unsafe_blocks` wants the literal `// SAFETY:` prefix. Justification text is present but key is inconsistent.
- **Fix:** Rename consistently to `// SAFETY:` across the project. Existing justifications are fine in substance.

### [MED] `assert_eq!` length checks in per-frame paths
- **Loc:** `crates/pt-wavefront/src/wavefront/pipeline.rs:378` (`assert_eq!(dims.len(), count_inits.len())`)
- **Cat:** 12
- **Issue:** Length-mismatch assert at the start of `prepare_tiles` — called every frame when tile layout changes.
- **Why bad:** Asserting precondition the type system could ensure.
- **Fix:** Either combine the two slices into `&[(WfDims, [u32;4])]` (caller-side zip), or change the public signature to take a single `&[TileParam]` struct.

### [MED] `pt-megakernel/compute.rs:3834` — `.expect("buffer just ensured")`
- **Cat:** 1
- **Issue:** Pattern: `slot = Some(...); let buf = slot.as_ref().expect("buffer just ensured");`. Trivially refactorable.
- **Fix:** Use `let buf = slot.insert(...)` to bind without unwrap (returns `&mut T`).

### [LOW / informational] False positives & intentional patterns
- `crates/gpu-mem/src/lib.rs:654-656` — unwraps in `#[test]` block; safe.
- `src/app/shell.rs` (8× `let _ =`) — intentional fire-and-forget on `Command::spawn()` for opening files / external dirs. Module doc explicitly explains pattern.
- `crates/media-encoder/.../encode.rs:2456` `let _ = settings.compression;` — placeholder for unimplemented TIFF compression; has comment.
- `src/main.rs:111` `file.lock().unwrap()` — log file mutex init; acceptable.
- `crates/pt-mats/src/lib.rs:839-840` `warm / total` — guarded by `let total = (warm + cool + neutral).max(0.001);` line 836. Safe.
- `crates/render-core/src/lib.rs:151` `experimental_features: unsafe { ExperimentalFeatures::enabled() }` — already has correct multi-line `// SAFETY:` justification (lines 147-150).
- `src/scanner_ntfs.rs` — 16 `unsafe` blocks, all 16 `// SAFETY:` comments present. No leak.

---

## Dead code / unused
None confirmed without graph index (gitnexus broken on Windows non-TTY).
Candidate worth verifying once gitnexus works:
- `crates/pt-mats/src/lib.rs:854-858` `#[allow(dead_code)] fn classify_by_hash_with_palette` — explicit allow. Maybe delete or wire back in.

---

## Dedup candidates (DRY violations worth a single PR)

1. **`Option<wgpu::Buffer>` slot accessor pattern.** Sites listed in "[HIGH] Lazy-init" above. Wrap in a small trait or helper macro:
   ```rust
   fn req<T>(slot: &Option<T>, what: &'static str) -> &T {
       slot.as_ref().unwrap_or_else(|| panic!("{what} not initialised"))
   }
   ```
   Or better, typestate.

2. **`"targets not built — call ensure_render_targets before render"`** repeated 9× in render-3d. Centralise.

3. **Buffer-readback boilerplate** in `bvh-gpu/mod.rs:1471-1483` — `map_async + poll + recv` triplet. Likely duplicated in other crates (worth a separate grep). Wrap as `fn readback<T: bytemuck::Pod>(device, queue, src) -> Result<Vec<T>, ReadbackError>`.

---

## Notes / followups
- **gitnexus TUI broken**: `gitnexus analyze` exits with code 0 after only listing skipped files. TUI is not detecting non-TTY and bails. Worth filing upstream — or run interactively from regular terminal.
- **subagent context-bloat**: spawned `Explore` subagents OOM on init ("Prompt is too long") because MCP server list in their system prompt is huge (Canva / Gmail / HF / Shopify / filesystem / gitnexus + full skills registry). Either prune `.mcp.json` for subagent context, or wait for harness fix. This sweep ran inline as fallback.
- **No async-Mutex misuse**: 0 hits for `.lock().await` — that whole category clean.
- **No `todo!()` / `unimplemented!()`**: clean.
- **Cast count high** (`as u32` / `as usize` ~600 occurrences) — did not exhaustively audit narrowing. Worth a follow-up pass when graph is available to focus on hot paths.
- **clippy not run** here — these are findings from text-only sweep. Expect clippy `undocumented_unsafe_blocks`, `unwrap_used`, `expect_used`, `cast_possible_truncation` to surface most of these and many more on a real run.

---

## Recommended order of attack
1. **CRITICAL #1**: bvh-gpu readback → propagate Result. 1 file, ~30 LOC.
2. **HIGH #3**: ffmpeg unsafe SAFETY comments. 2 sites, trivial.
3. **HIGH #4 / #5 systemic**: pick one approach (typestate vs resource-bag vs Result-returning getter), apply to *all* sites in one PR. This is the biggest payoff — eliminates a class of frame-time panics.
4. **HIGH #2**: tile offset overflow — return `Option<u32>`.
5. **HIGH #6**: material_cache sum-type refactor.
6. **HIGH #7**: picking channel recv.
7. **MED**: SAFETY comment renames, slot.insert tidy, prepare_tiles signature.
8. Optional: dedup readback boilerplate after #1 is done.

---

## Status — what landed on `bughunt/plan1-fixes`

| Phase | Commit | Crate / files | Effect |
|------:|--------|---------------|--------|
| 1 | `7e93470` | 10 files (mixed) | Surgical fixes — CRITICAL #1 (bvh-gpu readback Result), HIGH #2 (tile_offset overflow), HIGH #3 (ffmpeg SAFETY x2), HIGH #6 (material_cache sum-type), HIGH #7 (picking recv), MED (SAFETY: renames, slot.insert tidy) |
| 2 | `ab05f70` | pt-megakernel adaptive/pathguide/restir | Removed 9 fake-`Option<Buffer>` slots; ReSTIR's 6 frame buffers collapsed into one `ReSTIRBuffers` bundle |
| 3 | `13e1755` | pt-wavefront | 5 fake-`Option<Buffer>` slots collapsed into one `WfBuffers` bundle |
| 4 | `70e845e` | pt-denoise-oidn, treemap_view | `result_texture` / `result_view` built up-front (was lazy via `_ctx`), `burn_device_ref` collapsed into match-binding (1 expect removed); 2 callers in treemap_view updated to `.map` |
| 5 | `c171bef` | render-3d (4 files) | Bundle `RenderState { targets, dyn_bgs }`; 9 `"targets not built"`/`"dyn_bgs not built"` expects collapsed into one bundle access; `on_env_map_changed` rebuilds bind groups in place on existing targets |
| 6 | `d496bf0` | bvh-gpu | Bundle `BvhBuffers` (7 fields); 15 `.as_ref().unwrap()` sites consolidated; dead is-none branch in `update_aabbs` removed |
| 7 | `7a1013e` | pt-megakernel/compute.rs, pathguide | Bundle `GbufferStack { pipeline, bgl }`; eliminates one of the two unwraps via `Option::get_or_insert_with`; unused `resolution` field on PathGuide dropped |

Each commit verified individually with `cargo check -p <crate>` (exit 0). Workspace-wide cargo check after the chain is green.

## Status of the compute.rs panic class — DONE via Path B variant

Earlier draft of this doc listed 7 remaining `.as_ref().unwrap()` calls
in `dispatch_wavefront` as "not safe to do inline." Commit `ecbfa0e`
landed Path B in a lighter form than originally described:

* Each of the 7 sites now uses `let Some(x) = ... else { log + return false }`
  (or the `match` equivalent for the `as_mut` site). The function's
  existing `bool` return signals "false ⇒ nothing dispatched", so a
  drift between the entry check and a downstream access degrades into
  a logged skip instead of a frame-time panic.
* Each site logs an `error!` describing *which* slot drifted and *which*
  pass would have failed — far better debug signal than a bare unwrap
  panic site.

What this commit did **not** do (and why):
* The field types are still `Option<WavefrontPipeline>` etc. They have
  to be — the feature toggles at runtime.
* Dispatch helpers still take `&mut self`. Threading resolved refs
  through every internal pass helper in the 4525-LOC file would be the
  "full" Path B and remains a follow-up. The let-Some pattern at each
  site captures the same intent locally — at the cost of a few extra
  early-return checks the compiler can see don't fire when the entry
  check still proves the invariant.

Three sub-problems still motivate a future signature-threading pass:
1. Pipelines and their bind groups have **independent invalidation
   triggers** — `wavefront_bind_groups = None` at line 1658 when scene
   buffers aren't ready, even though `wavefront` stays `Some`. Bundle-
   then-eager-rebuild would make this go away.
2. The feature toggle (`wavefront_config.enabled`) lives next to the
   data (`wavefront: Option<_>`); a desync is a code-logic bug, not
   one the type system catches.
3. Dispatch is `&mut self` and intermixes immutable reads of resolved
   slots with mutable updates of other fields. Splitting the
   dispatch body into helpers that take resolved refs would lift the
   let-Some pattern out of the hot path entirely.

Tracked but not blocking.

