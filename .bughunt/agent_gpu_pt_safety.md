# GPU / Path-Tracing Safety Audit

Date: 2026-07-14
Scope: `bvh-gpu` GPU builder, `pt-megakernel::compute`, all `pt-megakernel` WGSL, all `render-3d` Rust, and requested repository-wide unsafe anchors.
Mode: static audit only. No source edits, tests, or builds.

## GitNexus status

`graph_status` reported a fresh HEAD index (`5befc15`) with a dirty worktree. `node .gitnexus/run.cjs analyze` completed successfully and reported the index up to date. Every required downstream `impact` call still failed before query execution: database storage version 42, MCP reader storage version 40. Attempted owners included `PathTraceCompute::create_variance_buffer`, `PathTraceCompute::resize`, `GpuBvhBuilder::build_gpu`, `PickingState::ensure_readback`, both megakernel render entrypoints, `EnvironmentMap::load_from_file`, `treemap::render`, `env_remove_var`, and affected WGSL files. Blast radius/risk therefore could not be graph-verified. Ratings below derive from traced source and sister-site inspection.

## Findings

### GPU-01 — CRITICAL — Megakernel variance storage is half-sized

Evidence:

- `crates/pt-megakernel/src/bvh_traverse.wgsl:127` defines `VarianceData` as two 16-byte lanes: 32 bytes.
- `crates/pt-megakernel/src/bvh_traverse.wgsl:970` reads `variance[pixel_idx].count`.
- `crates/pt-megakernel/src/bvh_traverse.wgsl:1492` updates the same element.
- `crates/pt-megakernel/src/compute.rs:3848` allocates only 16 bytes per pixel.
- Correct sister layout already exists at `crates/pt-megakernel/src/adaptive/pipeline.rs:11` and is sized with `size_of::<VarianceData>()` at `crates/pt-megakernel/src/adaptive/pipeline.rs:84`.

Consequence: runtime array length equals half the pixel count. Lower-half reads become robust-access zeros; writes are discarded. Adaptive SPP caps never terminate those pixels, while upper-half behavior differs. Duplicate variance trackers also create two sources of truth.

Systemic resolution: one host-owned `VarianceData` ABI type; checked `pixel_count * size_of::<T>()`; explicit minimum binding sizes; one variance/count buffer shared by megakernel and adaptive passes. Remove duplicate tracker only after caller/feature confirmation.

### GPU-02 — HIGH — ReSTIR temporal/spatial reuse is biased and reused samples are not revalidated for visibility

Evidence:

- Spatial target uses only radiance magnitude: `crates/pt-megakernel/src/restir/spatial.wgsl:149`–`151`.
- Spatial final weight repeats that approximation: `crates/pt-megakernel/src/restir/spatial.wgsl:157`–`160`.
- Temporal target/final target use only radiance magnitude: `crates/pt-megakernel/src/restir/temporal.wgsl:142`–`153`.
- Spatial compatibility checks only depth and normal: `crates/pt-megakernel/src/restir/spatial.wgsl:58`–`72`; material/instance compatibility is absent.
- Initial emissive candidates are shadow-tested at `crates/pt-megakernel/src/restir/initial.wgsl:501`–`510`.
- Final shade evaluates BSDF and accumulates directly at `crates/pt-megakernel/src/restir/shade.wgsl:219`–`260`; no shadow revalidation exists after temporal/spatial relocation.

Consequence: reservoir weights do not represent the target distribution at the receiving surface. Reused light samples can cross occluders/material boundaries, producing bias, leaks, ghosting, and unstable energy.

Systemic resolution: evaluate the full receiver target (`Li * BSDF * cosine`, proposal/Jacobian terms); validate depth, normal, material/instance; perform visibility at the final receiver; keep reservoir equations in one shared shader module/contract.

### GPU-03 — HIGH — Fixed traversal stacks silently discard BVH subtrees

Evidence:

- Main hit traversal drops both children when stack capacity is exhausted: `crates/pt-megakernel/src/bvh_traverse.wgsl:467`.
- Shadow traversal does the same: `crates/pt-megakernel/src/bvh_traverse.wgsl:505`.
- Picking uses a smaller 32-entry stack and the same silent drop: `crates/pt-megakernel/src/pick.wgsl:57`, `crates/pt-megakernel/src/pick.wgsl:149`.
- ReSTIR initial visibility repeats the 32-entry version: `crates/pt-megakernel/src/restir/initial.wgsl:147`, `crates/pt-megakernel/src/restir/initial.wgsl:215`.
- The main shader itself records prior depth failure at `crates/pt-megakernel/src/bvh_traverse.wgsl:202`–`209`.

Consequence: deep/adversarial LBVH trees produce false misses, incorrect picking, and false “unoccluded” results. Increasing the constant only moves the failure boundary.

Systemic resolution: stackless/rope traversal, or builder-enforced maximum depth with explicit validation and recoverable fallback. Share one traversal implementation/contract across main, shadow, pick, and ReSTIR.

### GPU-04 — HIGH — Russian roulette is biased, can create NaN, and the UI switch does not control megakernel RR

Evidence:

- Continuation probability is raw max throughput, unclamped: `crates/pt-megakernel/src/bvh_traverse.wgsl:1091`.
- Throughput divides by that value: `crates/pt-megakernel/src/bvh_traverse.wgsl:1095`.
- A value above 1 survives with probability 1 but is divided by a value above 1: downward bias.
- Zero throughput plus an exact-zero random sample can reach `0 / 0`.
- Host option only writes `wavefront_rr_enabled`: `crates/pt-megakernel/src/compute.rs:1654`–`1655`.
- Both render paths call only that wavefront setter: `crates/render-3d/src/pt/megakernel/render.rs:379` and `crates/render-3d/src/pt/megakernel/render_no_readback.rs:331`.
- Megakernel shader applies roulette unconditionally at `crates/pt-megakernel/src/bvh_traverse.wgsl:1089`.

Systemic resolution: common integrator settings consumed by both backends; explicit enable/start-bounce; terminate non-finite/zero throughput; clamp continuation probability to a valid interval before reciprocal weighting.

### GPU-05 — HIGH — Readback and zero-copy PT entrypoints have already drifted functionally

Evidence:

- Zero-copy sets tile size twice: `crates/render-3d/src/pt/megakernel/render_no_readback.rs:330` and `crates/render-3d/src/pt/megakernel/render_no_readback.rs:337`.
- Readback path has no corresponding tile-size update around `crates/render-3d/src/pt/megakernel/render.rs:378`–`384`.
- Readback path implements auto-SPP feedback at `crates/render-3d/src/pt/megakernel/render.rs:405`–`420`.
- Primary zero-copy path only reuses the stored value at `crates/render-3d/src/pt/megakernel/render_no_readback.rs:358`–`362`; it never measures/adapts it.
- Both files duplicate initialization, scene/BVH upload, camera/options propagation, dispatch, and blit.

Consequence: the same options mean different behavior by presentation path. Wavefront tiling is ignored in legacy/readback mode; auto-SPP is effectively static in zero-copy mode. Further drift is structurally likely.

Systemic resolution: one `prepare_and_dispatch` path returning render metadata/output view. Readback becomes a final optional presentation step, not a second renderer.

### GPU-06 — HIGH — Display and environment color-space contracts are inconsistent

Environment input:

- LDR PNG/JPEG values are treated as linear for luminance at `crates/render-3d/src/env_map.rs:143`–`150`.
- They are uploaded as `Rgba8Unorm`, not an sRGB-decoding format, at `crates/render-3d/src/env_map.rs:152`.
- That encoded-domain luminance builds the importance CDF at `crates/render-3d/src/env_map.rs:196`.

Display output:

- PT manually applies display encoding at `crates/pt-megakernel/src/blit.wgsl:321`–`351`.
- Render target is `Rgba8Unorm`: `crates/render-3d/src/targets.rs:44`–`54`; it is later sampled by egui and composed to an sRGB surface.
- Shader admits HDR PQ is destroyed by the current 8-bit sRGB surface: `crates/pt-megakernel/src/blit.wgsl:329`–`334`.
- OCIO clamps scene-linear HDR to `[0,1]`: `crates/pt-megakernel/src/blit.wgsl:301`–`315`.
- SDR uses a `1/2.2` approximation instead of the sRGB transfer function: `crates/pt-megakernel/src/blit.wgsl:350`.

Consequence: LDR environment radiance/CDF are wrong; SDR composition risks double transfer; HDR/OCIO modes destroy highlights despite exposed functionality.

Systemic resolution: explicit end-to-end color contract. Decode LDR input to linear before radiance/CDF use. Keep intermediate render target linear and let exactly one stage apply the surface transfer. Negotiate a real HDR surface for HDR modes. Use shaper-aware OCIO LUTs.

### GPU-07 — MEDIUM — Unchecked size/count arithmetic is a repository-wide class

Verified sites:

- Shared readback row/total arithmetic: `crates/render-core/src/lib.rs:263`–`268`, `crates/render-core/src/lib.rs:308`–`317`.
- Legacy 3D empty image: `crates/render-3d/src/lib.rs:1364`.
- Picking row alignment: `crates/render-3d/src/picking.rs:100`, `crates/render-3d/src/picking.rs:148`.
- Megakernel allocations/readback: `crates/pt-megakernel/src/compute.rs:1218`, `crates/pt-megakernel/src/compute.rs:2811`, `crates/pt-megakernel/src/compute.rs:3690`, `crates/pt-megakernel/src/compute.rs:3825`–`3859`, `crates/pt-megakernel/src/compute.rs:3914`, `crates/pt-megakernel/src/compute.rs:4065`.
- Adaptive/ReSTIR sister allocations: `crates/pt-megakernel/src/adaptive/pipeline.rs:83`, `crates/pt-megakernel/src/restir/pipeline.rs:232`.
- Wavefront sister allocations: `crates/pt-wavefront/src/wavefront/pipeline.rs:249`, `crates/pt-wavefront/src/wavefront/pipeline.rs:471`.
- BVH lossy `usize -> u32` and pre-cast multiplication: `crates/bvh-gpu/src/bvh_gpu/mod.rs:628`, `crates/bvh-gpu/src/bvh_gpu/mod.rs:668`, `crates/bvh-gpu/src/bvh_gpu/mod.rs:683`, `crates/bvh-gpu/src/bvh_gpu/mod.rs:689`, `crates/bvh-gpu/src/bvh_gpu/mod.rs:823`, `crates/bvh-gpu/src/bvh_gpu/mod.rs:871`.

Consequence: wrap can under-allocate buffers, truncate dispatch counts, or panic during CPU extraction. Current practical texture/device limits reduce reachability, but no invariant is encoded and sister paths already diverge.

Systemic resolution: central checked helpers for pixel count, row pitch/alignment, typed byte size, and checked `u32` GPU counts. Return recoverable errors before allocation/dispatch.

### GPU-08 — HIGH — Solid treemap parallel fill is not memory-safe by construction

Evidence:

- `render` creates a normal `Vec<u8>`: `crates/treemap/src/lib.rs:461`–`464`.
- Disjointness is checked only by `debug_assert!`: `crates/treemap/src/lib.rs:498`–`504`.
- Rayon closures derive a mutable raw pointer from `buf.as_ptr()` and write through it: `crates/treemap/src/lib.rs:525`–`545`.
- Bounds and non-overlap are runtime layout properties; release builds enforce neither at the unsafe boundary.

Consequence: any overlap/out-of-range layout regression becomes data-race/out-of-bounds UB, not a recoverable rendering error. The cast also mutates storage reached through a shared reference.

Systemic resolution: use safe disjoint mutable partitioning. The existing row-parallel `par_chunks_exact_mut(w * 4)` model at `crates/treemap/src/lib.rs:512`–`520` already supplies the correct ownership structure and can serve both cushion and solid modes.

### GPU-09 — MEDIUM — Shader input domains and ray offsets lack robust contracts

- IOR is consumed without validation in Fresnel/refraction: `crates/pt-megakernel/src/bvh_traverse.wgsl:1101`, `crates/pt-megakernel/src/bvh_traverse.wgsl:1423`, and `crates/pt-megakernel/src/restir/shade.wgsl:244`.
- Public material construction accepts arbitrary IOR: `crates/standard-surface/src/params.rs:91`–`96`.
- Fixed world-space ray offsets appear throughout, e.g. `crates/pt-megakernel/src/bvh_traverse.wgsl:1126`, `crates/pt-megakernel/src/bvh_traverse.wgsl:1381`, `crates/pt-megakernel/src/bvh_traverse.wgsl:1439`, and `crates/pt-megakernel/src/restir/initial.wgsl:502`.

Consequence: zero/negative/non-finite IOR can generate infinities/NaNs. Fixed offsets cause acne on large coordinates and detached shadows/light leaks on small geometry.

Systemic resolution: validate/sanitize material domains once at the material boundary and defensively in shaders; use scale/ULP-aware normal offsets shared by all ray producers.

### GPU-10 — MEDIUM — RNG implementations are duplicated, collide predictably, and some return 1.0

- Megakernel affine seed: `crates/pt-megakernel/src/bvh_traverse.wgsl:974`; pairs separated by `(+6133 pixels, -1973 frames)` collide before hashing.
- ReSTIR seeds use direct XOR products: `crates/pt-megakernel/src/restir/initial.wgsl:460`, `crates/pt-megakernel/src/restir/temporal.wgsl:99`, `crates/pt-megakernel/src/restir/spatial.wgsl:99`.
- ReSTIR/path-guide RNG divides by `4294967295.0`, allowing 1.0: `crates/pt-megakernel/src/restir/initial.wgsl:131`, `crates/pt-megakernel/src/restir/temporal.wgsl:69`, `crates/pt-megakernel/src/restir/spatial.wgsl:55`, `crates/pt-megakernel/src/pathguide/sample.wgsl:31`.
- Main megakernel correctly uses the half-open denominator at `crates/pt-megakernel/src/bvh_traverse.wgsl:229`.

Consequence: repeatable spatiotemporal correlation and endpoint-sensitive selection/sampling artifacts.

Systemic resolution: one shared WGSL RNG module keyed by pixel, frame, sample, bounce, and dimension; strong tuple mixing; guaranteed `[0,1)` conversion.

## Unsafe-anchor disposition

- Missing required safety rationale: `crates/xtask/src/env_setup.rs:64`. Sister wrapper has the exact concurrency invariant at `crates/xtask/src/env_setup.rs:57`.
- Inadequate precondition-focused rationale: `crates/render-core/src/lib.rs:152`–`158` explains why experimental features are needed, not the unsafe API contract.
- Detailed safety rationale present: `crates/gpu-mem/src/lib.rs:370`–`374`, `crates/media-encoder/src/dialogs/encode/encode.rs:1402`–`1406`, `crates/media-encoder/src/dialogs/encode/encode.rs:1661`–`1668`.
- No `unsafe` blocks found in the scoped `render-3d`, `bvh-gpu`, or `pt-megakernel` source.
- Treemap anchor is a correctness defect, not only a missing-comment defect; see GPU-08.

## Recommended implementation order

1. GPU-01 variance ABI/ownership.
2. GPU-08 memory safety.
3. GPU-02 ReSTIR estimator/visibility.
4. GPU-03 traversal correctness.
5. GPU-04 roulette control/math.
6. GPU-05 unify PT entrypoints.
7. GPU-06 color pipeline contract.
8. GPU-07 checked sizing/count infrastructure.
9. GPU-09 material/ray robustness.
10. GPU-10 shared RNG.

No deletion recommended. Placeholder/dead-code candidates require restored GitNexus caller analysis plus TODO/FIXME cross-reference and user approval.
