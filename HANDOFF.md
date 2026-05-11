# HANDOFF — dirstat-rs cross-session resume

**Date:** 2026-05-11 (sprint-5)
**Last commit:** `1a53b23 docs(todo4): sprint-5 Stage G section + open follow-ups for next session`
**Branch:** `main`
**Author/operator:** ssoj13

This file captures everything a fresh session needs to pick up work
without re-reading the full conversation history. Read this first,
then `TODO4.md` Stage G section for the technical roadmap, then
`CHANGELOG.md` sprint-5 entry for "what shipped today."

## TL;DR for the next session

1. **Megakernel ReSTIR-DI lives at bounce 0** (Stage G.B, shipped).
   When the UI ReSTIR DI checkbox is on, the megakernel does RIS over
   M candidates from the existing Vose alias table, shadow-tests one
   surviving candidate, writes the reservoir to `cur_reservoirs`. No
   wavefront round-trip needed. See `bvh_traverse.wgsl` ~line 1106
   for the RIS branch.

2. **BVH traversal stack 32 → 64** fixed the camera-rotation block-
   flicker (cubes disappearing with env map peeking through). 
   `crates/pt-megakernel/src/bvh_traverse.wgsl:179`.

3. **Open: animation block-flicker.** With `Effects=Ocean,
   Strength≥1, Animation=on`, cubes shift ±30 units along Z per
   frame; the GPU BVH refit appears to lag behind the instance
   upload so rays use stale AABBs. NOT a stack-depth issue. Refit-
   sequencing investigation needed in `bvh_gpu` crate.

4. **Open: Stage G.C — temporal reuse.** ~150 LOC. Plan in TODO4.md.
   Bindings (cur_reservoirs / prev_reservoirs / motion_vectors) are
   already in place from G.A.

5. **Open: Stage G.D — spatial post-pass.** Optional, after G.C.

6. **Open: Stage G.E — UI default = megakernel.** Cleanup commit.

## Repo state

- **HEAD:** `1a53b23` on `main`
- **Latest commits (read chronological top→bottom):**
  - `f03707c docs(changelog): Stage G.B ReSTIR-DI in megakernel + BVH stack fix`
  - `2bdd9fe fix(pt): BVH stack 32→64 fixes block-flicker; Stage G.B ReSTIR-DI RIS`
  - `3e2088b chore: WIP` — actually contains the bulk of Stage G.B (auto-WIP captured the WGSL RIS block + compute.rs uniform changes that the dedicated commit only added the stack-depth bump on top of). Treat as a paired commit with `2bdd9fe`.
  - `008aac3 fix(ui): thinner Effects/Animation/Path Tracer headers, full-width`
  - `dab4590 docs(changelog): sprint-5 (2026-05-11) — palettes, viz abstraction, light perf`
  - `2151d04 feat(materials): perceptual palettes + scene-aware Depth/Size + ReSTIR megakernel plumbing` ← Stage G.A landed here

- **Release exe** at `target/release/dirstat-rs.exe` is current with
  the latest changes (rebuilt after `2bdd9fe`).

## What this session did

### UI

- Made `tinted_section` (Effects / Animation / Path Tracer header
  bands) much thinner: inner_margin (3, 1), interact_size.y = 14,
  spans full panel width via `ui.set_min_width(ui.available_width())`.
- Same compact treatment for `compact_section` (nested
  Materials / Lighting / Sampling under Path Tracer).
- File: `src/app/settings/mod.rs` (tinted_section),
  `src/app/settings/renderer.rs` (compact_section).

### Megakernel ReSTIR-DI (Stage G.B)

**Why:** Earlier (sprint-4) we had ReSTIR running inside the wavefront
pipeline. User found wavefront materially slower than megakernel on
this scene (simple cubes, low ray divergence) and the quality win
didn't pay back the per-dispatch overhead. Decision: port ReSTIR-DI
into the megakernel itself.

**Architecture target:**

- Bounce 0: RIS-resample one direct-light sample over M candidates
  (replaces the old multi-sample MIS-NEE block).
- Bounce 1+: existing MIS-NEE estimator stays, so indirect bounces +
  glass transmission render exactly as before.
- Reservoir is persisted to `cur_reservoirs[pixel_idx]` so the next
  frame's temporal step (Stage G.C, not done yet) can resample it.
- All inside one compute dispatch — megakernel speed preserved.

**Host plumbing (compute.rs):**

- `EmissiveLightUniform.params0.w` carries `di_enabled` (0/1) and
  `params1.z` carries `initial_candidates` as `f32`.
- `write_emissive_light_uniform()` writes both.
- `dispatch()` calls `write_emissive_light_uniform(queue)` every
  frame so toggles propagate without a dedicated setter.
- `set_restir_enabled(device, di, gi)` still triggers BG rebuild on
  None→Some transition (allocates `ReSTIRPipeline` lazily).
- The megakernel BG bindings 15/16/17 (cur_res / prev_res / motion)
  point at `ReSTIRPipeline.reservoir_a/b` and `.motion_buffer()`
  when ReSTIR is active, fallback buffers (`restir_fb_reservoir_cur
  /prev`, `restir_fb_motion`) otherwise. Wired in `rebuild_bind_group`
  at ~line 3926.

**WGSL (`bvh_traverse.wgsl`):**

- ~line 1106, inside the existing `if transmission_weight < 0.5 && ...`
  guard, branch on `let restir_di = bounce == 0u &&
  emissive_light_params.params0.w != 0u`.
- If true: RIS over M candidates, target function
  `luminance(emission) * cos_theta`, stream sampler
  (`rand · w_sum < w_i`), one shadow ray on the surviving candidate,
  contribution applied with `W = w_sum / (m · target_selected)`,
  reservoir written to `cur_reservoirs[pixel_idx]`.
- Else: the original multi-sample MIS-NEE block runs (unchanged).
- Bounce 1+: untouched.

**Known caveats:**

- Target function is `luminance(emission) · cos_theta` — cheap but
  ignores BSDF angular term. Possible refinement: include scalar
  BSDF magnitude. Not critical for current scenes.
- No MIS between RIS contribution and BSDF-sample-hits-light at
  next bounce. In theory minor double-count when a BSDF-sampled ray
  randomly lands on an emissive surface, but rare (emissive cubes
  ~1.7% of scene). Bias visible only on extreme close-ups.

### BVH stack overflow fix (Stage G.X)

- `MAX_STACK_DEPTH: u32 = 32u` → `64u` in `bvh_traverse.wgsl:179`.
- Cost: ~256 B/thread of register-mapped private storage. Negligible
  at 8×8 workgroups.
- **Fixed:** camera-rotation block-flicker (cubes disappearing,
  env map showing through).
- **NOT fixed:** animation block-flicker (separate issue, refit
  sequencing — see Open Issues below).

## Open issues / what's next

### A. Animation block-flicker (high prio for next session)

**Repro:** Path Tracer ON, Animation=on, Effects=Ocean,
Strength≥1. During animation, entire blocks of cubes flicker on/off
each frame, env map peeks through holes.

**Hypothesis:** Ocean effect displaces cubes by ±30 units along Z per
frame (`crates/render-shared/src/lib.rs:1498-1510`,
`HashTransformEffect::Ocean` returns `offset = Vec3(0, 0, -(wave1
+ wave2 + wave3) * strength * 15.0)`). Each frame the host re-uploads
the instance buffer with new model_inv matrices. The GPU LBVH refit
needs to recompute AABBs from the new instances. If refit runs AFTER
PT dispatch, or doesn't run at all when only positions change, the
PT reads stale AABBs and rays miss the moved cubes.

**Investigation plan:**

1. Read `bvh_gpu` crate (location: `crates/bvh-gpu/src/`). Find the
   refit entry point.
2. Trace when refit is called relative to `dispatch()`. Both
   `crates/render-3d/src/pt/megakernel/render.rs` and
   `render_no_readback.rs` orchestrate this.
3. Verify refit runs AFTER instance upload and BEFORE PT dispatch.
   With wgpu, this means the refit compute pass needs to be in the
   same encoder as the PT dispatch, with a barrier (or just same
   pass — wgpu serializes dispatches within an encoder).
4. Check if `pt_bvh_refit` opt is even enabled by default. UI: Path
   Tracer → Advanced → BVH Refit checkbox.

**Possible fixes:**

- Force refit every frame when animation is on.
- Switch to full BVH rebuild when displacements exceed a threshold.
- Add a sync/barrier between instance upload and PT dispatch.

### B. Stage G.C — temporal reservoir reuse (~150 LOC)

Plan in TODO4.md Stage G.C section. Steps:

1. **Write motion vectors and current depth inline** at the primary
   hit in `bvh_traverse.wgsl`. Compute prev_pixel via
   `prev_view_proj` (already in camera uniform from sprint-4 commit
   `2767548`). Write to `motion_vectors[pixel_idx]` (binding 17 —
   currently bound RO, will need to change to RW for megakernel) and
   `cur_depth_buf` (need to add a new binding, OR repurpose the
   existing `prev_depth_buf` from ReSTIRBindGroups).
2. **Disocclusion check:** depth difference between current hit and
   `prev_depth_buf[prev_pixel]` > threshold → reject prev reservoir.
3. **RIS-combine current reservoir with prev** (clamp prev `m` to
   `m_max` to avoid bias). Same pattern as
   `crates/pt-megakernel/src/restir/temporal.wgsl`.
4. **End-of-frame:** copy cur_depth → prev_depth_buf, swap
   reservoir_a/b ping-pong via `rs.swap_bufs()`.

Bindings already in place (G.A). The hard parts are:

- Motion vector binding currently RO — needs RW for megakernel to
  write. Change in `compute.rs::rebuild_bind_group` + WGSL declaration.
- ReSTIRPipeline currently allocates `gbuf_depth` (4 B/pixel) and
  `prev_depth_buf`. The wavefront code copies cur→prev at end of
  frame. For megakernel: same idea, copy `gbuf_depth` → `prev_depth_buf`
  at frame end.

### C. Stage G.D — spatial post-pass (optional, later)

After the megakernel dispatch, run `crates/pt-megakernel/src/restir/
spatial.wgsl` once on the full image, reading the just-written
`cur_reservoirs` and writing to a spatial output. The spatial output
feeds NEXT frame's temporal step (one-frame lag, acceptable). This
keeps the megakernel single-dispatch and gets the full ReSTIR quality
trifecta (initial + temporal + spatial).

`spatial.wgsl` is already wavefront-compatible from sprint-4 work
(post Phase 1 refactor). It binds reservoirs_in / reservoirs_out /
depth_buf / normal_buf / params. For megakernel we'd bind the same
buffers; the shader doesn't care which integrator wrote them.

### D. Stage G.E — UI / cleanup (later)

- Make `pt_wavefront` default `false` in `Render3DOptions`
  (`crates/render-shared/src/lib.rs:879`).
- Rename "Wavefront" UI label to clarify it's the legacy path
  (e.g., "Wavefront (legacy)").
- Update HANDOFF, TODO4, CHANGELOG to reflect the canonical backend
  switch.

## Architecture notes for relevant files

### `crates/pt-megakernel/src/compute.rs` (~4500 LOC)

The megakernel host. Owns `PathTraceCompute`, the BGL, all buffer
allocations, dispatch logic. Key sections:

- Struct definitions (PtCameraUniform, EmissiveLightUniform, etc.) at top.
- `PathTraceCompute::new` ~590 — creates BGL with bindings 0-18.
- `rebuild_bind_group` ~3920 — recreates the megakernel BG; this is
  where you swap fallback buffers for real ReSTIR buffers.
- `write_emissive_light_uniform` ~1434 — host-side ReSTIR-DI flag
  goes through this.
- `dispatch` ~4289 — single-shader path-traced frame.

### `crates/pt-megakernel/src/bvh_traverse.wgsl` (~1750 LOC after sprint-5)

The megakernel itself. Single big shader. Key sections:

- Struct declarations (BVHNode, Instance, Ray, HitInfo, Material,
  EnvParams, EmissiveLight, EmissiveLightParams, Sample, Reservoir,
  MotionVector) — ~lines 1-180.
- Bindings 0-18 — ~lines 147-178.
- `MAX_STACK_DEPTH = 64u` — line 187.
- `pick_alias_index` / `sample_emissive_light` — ~750.
- `trace_ray` / `trace_shadow_ray` — ~380 / ~426.
- `main` — ~895, with the bounce loop at ~935.
- ReSTIR-DI RIS block — ~line 1106, inside the existing emissive NEE
  guard. The else-branch is the original multi-sample NEE.

### `crates/pt-megakernel/src/restir/` 

Wavefront-era ReSTIR scaffolding. Still used: the buffers from
`ReSTIRPipeline` (reservoir_a/b, motion_buf, gbuf_depth, gbuf_normal,
gbuf_instance_id, prev_depth_buf). The compute pipelines
(`initial.wgsl`, `temporal.wgsl`, `spatial.wgsl`, `shade.wgsl`) are
unused by the megakernel path but kept because the wavefront opt-in
backend still uses them. Stage G.D will reuse `spatial.wgsl` as a
post-pass.

### `crates/bvh-gpu/src/`

GPU LBVH builder + refit. Not yet read this session. Animation block-
flicker investigation needs to start here.

## Build / test commands

```powershell
# Release build (the user runs this; can't build while .exe is running)
cargo build --release --message-format=short 2>&1 | grep -E "error|warning:" | head -10

# Clippy on the path tracer crate
cargo clippy -p pt-megakernel --all-targets --message-format=short 2>&1 | grep -E "error|warning:" | head -10

# Tests
cargo test -p pt-megakernel -p pt-wavefront --message-format=short 2>&1 | grep -E "test result|FAILED" | head -10

# Profile-style run with PT logging
.\target\release\dirstat-rs.exe --log-modules pt 2>&1 | Tee-Object profile.log | Select-String "upload_scene|WF dispatch|cache MISS|scene_upload|bvh_build"
```

## User context / collaboration patterns

- **Language:** RU in chat, EN in code/comments/commits. Operator
  prefers terse responses, no fluff.
- **Decision-making:** prefers honest pushback over agreeable
  half-solutions. If a refactor is too large for one session, SAY
  SO and commit a checkpoint rather than risk broken state.
- **Frustration triggers:** flapping behaviour (commit clamp, revert
  clamp, re-add clamp — operator called this "снапить блять"); long
  speculative responses without concrete progress.
- **Verification style:** operator runs the release exe themselves,
  shows screenshots, points at visual artifacts. They diagnose the
  USER side; we diagnose the CODE side.
- **Snapping/UI:** WF Tile is clamped {0} ∪ [64, 8192] in the UI AND
  host. 0 = full frame (no tiling). Drag-down uses halfway split
  (<32 → 0, 32..63 → 64) so the user can drag to "off" via mouse.

## Things to NOT do

- Don't add new features speculatively. Operator wants concrete
  progress on known bugs / Stage G plan.
- Don't blindly re-run `npx gitnexus analyze` — last session it
  segfaulted. The "GitNexus index stale" warning from hooks can be
  ignored unless explicitly asked.
- Don't auto-rebuild release while operator might be running the
  exe — it'll fail to write the .exe. Wait for explicit "rebuild"
  signal or just cargo check first.
- Don't undo / partially-undo recently shipped commits. Operator
  asked twice in a row "don't snap" then "add clamp back" — go with
  the LATER instruction.
