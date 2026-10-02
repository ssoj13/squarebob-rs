# plan16 — Progressive highlight noise: OIDN bridge audit and verification

Updated: 2026-10-02. This report follows [plan15.md](plan15.md) and preserves its historical results. Renderer baseline reviewed: `5f73509`; OIDN dependency remains `bebbfc5`. User symptom: noise localized to highlights and fine details grows during one progressive render. Production fixes await approval. Dependency updates and a diagnostic GPU example were authorized; no audited production Rust logic was changed.

## Work checklist

- [x] Read existing operating notes, architecture docs, and prior plan.
- [x] Trace all 1041 bridge source lines and app invocation/settings/display.
- [x] Check model roles, exposure, AOV normalization, row alignment, cache ownership, and GPU synchronization against pinned CubeCL.
- [x] Record source findings with exact anchors and a frozen/progressive input test matrix.
- [x] Check all 11 SSH Git dependency refs and update outdated ones; retain intentional source choices.
- [x] Restore gpu-allocator/wgpu-hal Windows type compatibility through normal Cargo resolution.
- [x] Resolve locked/offline metadata; 21 workspace members, 880 resolved packages reported.
- [x] Run bridge unit test: 1 passed after Windows dependency repair.
- [x] Append architecture/codepath diagrams without deleting previous documentation.
- [x] Check diagnostic rustfmt and documentation whitespace; parent reports rustfmt --check passed, git diff --check passed.
- [x] Finish initial diagnostic GPU run and record command, adapter, settings, metrics, and limits.
- [x] Finish 32-repeat frozen-input stability extension; all finite, max difference 0.
- [x] Finish workspace compilation: cargo check --workspace --locked --quiet, exit 0.
- [x] Finish actual squarebob binary linking: cargo build --locked --bin squarebob --quiet, exit 0.
- [ ] Reproduce the actual renderer symptom with raw/AOV/denoised snapshots across progressiveSPP.
- [x] Compare synthetic frozen-input repeated runs and adaptive versus fixed clamp.
- [ ] Repeat those comparisons on an actual scene snapshot.
- [ ] Approve production repair scope after reviewing this report.
- [ ] Implement approved systemic repairs and focused regression tests.
- [ ] Verify actual renderer/native/CPU/WGPU quality before declaring root cause or closure.

## Evidence and audit boundary

Detailed evidence: [bridge report](../oidn-rs/bughunt/squarebob_bridge.md), [OIDN source audit](../oidn-rs/bughunt/pipeline.md), [local OIDN2.5 comparison](../oidn-rs/bughunt/local_pipeline.md), and [OIDN approval plan](../oidn-rs/plan2.md). The local reference is clean OIDN v2.5.0 at `f7ae1bf07b3201aaa8cfe04d71f5243f8e0f2bb7`; original downloaded citations remain pinned v2.4.1. No native weights parity or numerical parity is asserted.

GitNexus MCP became available to the bridge agent; incremental comparison refreshed indexed e662a71 to 5f73509 before querying. The parent's separate CLI query timed out during catalog scanning. After dependency/docs changes and the new example, incremental reanalyze succeeded for five files and graph_status reported fresh at 5f73509, with expected unstaged work. detect_changes(all) reported 11 changed nodes across 6 files, 0 affected processes, medium risk. Incremental communities/processes/routes/tool maps remain unchanged until full analysis; absence of affected processes is not a complete semantic impact proof. Direct source reads validate the statements below; successful graph navigation does not imply complete graph coverage. The docs-write supplemental guide is absent, already recorded in the mandated harness BUG3.md; inline style rules and direct Markdown review are the fallback.

## Checked bridge dataflow

```text
PT normalized HDR Rgba32Float + per-pixel AOV RGB sums/W counts
 -> app manual/periodic/final trigger using current SPP
 -> shared renderer GPU device/queue input copies into Burn tensors
 -> trim row padding; sample-dependent HDR clamp; AOV RGB/max(W,1)
 -> NCHW RGB -> immutable committed model + fresh input tensors
 -> env/physical/automatic input scale -> OIDN preprocess/model/postprocess
 -> HWC RGBA alpha1 -> row padding -> get_resource flush/allocation pin
 -> copy into separate result texture -> poll -> result_view
 -> composite_overlay targets render_view -> display
```

Anchors: `crates/render-3d/src/lib.rs:1141,1153,1159,1217-1222`; `src/app/treemap_view.rs:1491-1532,1579-1606`; `crates/pt-denoise-oidn/src/lib.rs:392-422,457-493,515-518,627,639-676,906-919,979-1012`. The inspected bridge contains no denoised-to-raw feedback. A full raw accumulator/shader review and runtime experiment are separate evidence.

## Confirmed source findings and scoped candidates

| Finding | Evidence | Result and proposed systemic repair |
|---|---|---|
| Adaptive clamp changes highlight input | bridge457-468; render-shared1128-1135 | Default ceiling6 at 128 SPP rises to 10 at 256 SPP. Freeze raw input and vary onlySPP; compare fixed/adaptive clamp before assigning causality. Preserve clamping feature. |
| Final denoise skipped after preview | treemap_view1510-1522,1609-1612 | target 300/interval 128 can retain256SPP preview. Track completedSPP and accumulation identity; final completion must not reuse an any-preview boolean. |
| Denoised display advances in snapshots | treemap_view1531,1608-1612,1209-1215 | RawSPP and denoisedSPP can differ. Capture both and identify which image is displayed. |
| Resolved model metadata lost | bridge548-552,597 | Cache retains bytes while dropping stem, propagating known Small/Fast override defect. Resolver/commit must share validated descriptor/provenance. |
| Exposure scopes differ by camera mode | treemap_view1587-1591; bridge515-518 | Physical supplies explicit scale, Manual uses autoexposure absent env override. P5/P6 autoexposure defects are conditional, not universal. |
| Scale unnecessarily invalidates model cache | bridge562-569,619 | Runtime scale setter exists; remove redundant rebuild after proving immutable-state contract. Validate scale centrally. |
| Poll failure discarded | bridge676,690 | Return meaningful GPU errors through existing error path; no silent successful frame after failed synchronization. |
| Standalone debug path crosses devices | bridge707-711,392-422,953-1012 | Independent Burn buffers are still copied with renderer encoder. Use supported host transfer for that experiment; normal shared-device path is different. |

Bridge is `crates/pt-denoise-oidn/src/lib.rs`; treemap_view is `src/app/treemap_view.rs`; render-shared is `crates/render-shared/src/lib.rs`.

Current buffer race is not confirmed. Pinned CubeCL `get_resource` flushes relevant streams and holds an allocation binding; bridge retains it through external-copy queue submission. Same queue supports compute-before-copy. Historical bridge comments658-675 claiming growing speckle from pool reuse are not a reproduction. See full paired runtime source paths in the bridge report.

OIDN P1 normal padding is reachable in full-AOV default. P2/P3 auxiliary-only primary filters, P4 lightmap, and P11 scalar host output are not reached by this color/RGB bridge. P7 Large overlap requires actual Large topology and multiple tiles; High/noisy-aux usually resolves Base. P9 mutable input-handle caching is not this immutable fresh-tensor path.

## SSH dependency verification

All 11 SSH Git repositories were checked against remote `main` using `git ls-remote --symref URL HEAD refs/heads/main`. Six advanced; five were already current. The table records checked full SHA values from the dependency agent's report; Cargo.lock stores the full resolved hashes.

| Repository | Previous SHA | Current SHA | Current lock source |
|---|---|---|---|
| ffmpeg-rs | f12daaf60358a1edcdbb13303fe5cc271990cb4b |7524123cb6dddb7f3009aa06208f4012673c5958|Cargo.lock:300|
| codec-simd-rs | c79919b3304a18e66393e61527f868a94b95ac03 | unchanged |Cargo.lock:1545|
| exr-rs |1e454eaf9e637efb1b90cf1e270f87e09851dff9|a2c5d7b8a45c12d774fc63f94719431dbe5bd69f|Cargo.lock:2459|
| fscan-rs |c33d4474b5004ec047ecd065f96f6a53addbc97e|5f57af677f902522e52977e07e1acd88661339aa|Cargo.lock:3343|
| gpu-info-rs |583b1b85a344365a8de413c087284d64da95bf7d|43d0078526ee00513e1a4b7b572351fdc4324c1a|Cargo.lock:3746|
| jph-rs |7f7b1518cb927437bf2aa19ad76687aae4cd4e67|e7232855b7774315515f7ffe13f6d8dc67190713|Cargo.lock:4427|
| kvz-rs |42f14e9bd9bea92163f6720c2864a1e6f1c4f5f1|unchanged|Cargo.lock:4472|
| lcms-rs |27d17877d548f54fe7a107159c56118c117fd438|unchanged|Cargo.lock:4611|
| murmur3 |1819dc0c9295f804eb07e4aa273e20d603163f78|unchanged|Cargo.lock:4959|
| oidn-rs |bebbfc5ec26c589121e2b0878ff55a6f669b43a6|unchanged|Cargo.lock:5603|
| oiio-rs |03f0296e8953e6c32edbcd1e8cc5f067a2497c4f|fa4936bcc4fd9f34d9dded823b144189c33d5265|Cargo.lock:8187|

fscan-rs remains an explicit revision pin at `Cargo.toml:179`; remote comparison reported ahead1/behind0 from the old pin. Other SSH main branch declarations were retained. HTTPS Burn/CubeCL/vcv pins were unchanged. Normal Cargo update also removed obsolete Windows registry packages and normalized Windows dependency edges; upstream vfx-lut no longer depends on rayon. Do not describe all lockfile deltas as SSH source changes.

An initial resolution chose Windows 0.56 for gpu-allocator 0.28, causing DX12 COM-type mismatch with wgpu-hal 30/Windows 0.62. The allocator's published range allows 0.53..0.62; normal `cargo update -p gpu-allocator --precise 0.28.0` restored common Windows 0.62.2. Current allocator anchor `Cargo.lock:3730-3741`, iana-time-zone windows-core0.62.2 at4014-4025. No dependency patch, registry-cache edit, or production Rust edit was used.

Dependency agent reported `cargo metadata --locked --offline` and `cargo metadata --locked --no-deps` exit 0 with 880 packages/21 workspace members, and `git diff --check` exit 0. These establish resolution, not image quality.

## Tests and GPU experiment

The bridge unit test `tests::mode_aov_requirements` checks only mode/AOV requirements (`crates/pt-denoise-oidn/src/lib.rs:1034-1040`). Parent reports 1 passed after gpu-allocator was resolved with windows 0.62.2, matching wgpu-hal 30. This does not establish image quality or resource stability.

Diagnostic `examples/oidn_highlight_probe.rs` was run by the parent with `cargo run --locked --example oidn_highlight_probe`, exit 0. Evidence: [stdout](../oidn-rs/bughunt/squarebob_probe.stdout.log), [stderr](../oidn-rs/bughunt/squarebob_probe.stderr.log). Adapter: NVIDIA GeForce RTX 3080 Ti, Vulkan, driver 616.64.

Input: frozen 96×64 HDR checkerboard, bright region RGB=(v,0.8v,0.6v), v alternating 48/8; background 0.25; constant albedo 0.7 and normal+Z with count 1. FullAOV/Balanced, explicit scale 0.02, user clamp 10; env scale and standalone-device overrides prohibited. Bright ROI x20..76/y20..44; population luminance standard deviation, not a ground-truth noise-error metric. Source: `examples/oidn_highlight_probe.rs:135-157,169-185,221-241,247-286`.

| Clamp | current SPP | Highlight mean | Highlight SD | Highlight max | Dark mean |
|---|---:|---:|---:|---:|---:|
| Adaptive |1|2.000045|0.001503|2.005736|0.210797|
| Adaptive |64|3.248755|0.002404|3.255955|0.210939|
| Adaptive |128|5.997941|0.003034|6.010278|0.210957|
| Adaptive |256|8.164577|0.065254|8.366093|0.210212|
| Fixed |1|8.164577|0.065254|8.366093|0.210212|
| Fixed |256|8.164577|0.065254|8.366093|0.210212|

All captured values were finite. Two consecutive adaptive 256 runs had max absolute RGB difference 0; fixed clamp at SPP 1 versus 256 also had difference 0. Adaptive 1 versus 256 differed by 8.448264122 maximum absolute RGB. This demonstrates sample-dependent image change and increased spatial ROI variation on synthetic input. It does not reproduce the user's scene, prove that this variation is erroneous noise, or establish a root cause. The 32-repeat extension also exited 0: fixed-clamp, current SPP 256, same cached bridge, all finite, max absolute RGB difference 0 against the earlier fixed-clamp256 baseline. Evidence: [series stdout](../oidn-rs/bughunt/squarebob_probe_series.stdout.log) and [series stderr](../oidn-rs/bughunt/squarebob_probe_series.stderr.log); loop at examples/oidn_highlight_probe.rs288 onward. Stability is established only for this frozen pattern/device/configuration; this is not stochastic scene proof or a general absence-of-race proof. Workspace compilation also passed: parent ran cargo check --workspace --locked --quiet, exit 0 in 39.958s, empty stdout/stderr. Evidence: [check stdout](../oidn-rs/bughunt/squarebob_workspace.stdout.log), [check stderr](../oidn-rs/bughunt/squarebob_workspace.stderr.log). This confirms compilation, not actual-scene image quality. Actual binary linking also passed: cargo build --locked --bin squarebob --quiet, exit 0 in 28.547s, empty stdout/stderr. Evidence: [build stdout](../oidn-rs/bughunt/squarebob_build.stdout.log), [build stderr](../oidn-rs/bughunt/squarebob_build.stderr.log). The squarebob binary was built, but no actual-scene GUI render or screenshot comparison was performed.

stderr contains a third-party Bandicam Vulkan layer warning (API1.2 versus application1.3); no GPU validation failure was reported. The warning alone does not prove influence on the output.

## Remaining measured verification

1. Frozen HDR/AOV snapshot, fixed model/scale/dimensions/SPP: repeated image differences and hashes.
2. Same snapshot128 versus 256SPP: adaptive on/off and clamp0, using existing effective-clamp trace at bridge475.
3. Actual progressive raw/AOV/denoised snapshots128/256/512/...: separate highlight/detail/flat-region metrics.
4. Color/fullAOV and Base/Fast/High: log resolved stem549 and committed model605.
5. Physical scale versus Manual autoexposure, aligned/nonaligned widths, tile/edge geometry.
6. NaN/+Inf/-Inf stage-specific behavior; propagate GPU errors.
7. Scheduling regression: nonmultiple final targets, exact multiples, manual preview, accumulation resets, and target changes.

## Production scope proposed for approval

Approve source repairs after measured results: completed-SPP/accumulation scheduling, validated model descriptors through one resolver/commit path, shared scale/device error handling, and reachable OIDN parity fixes. Preserve exposure, AOVs, clamping, quality choices, and preview modes. Do not remove unfinished features or introduce competing classifiers/codepaths. Any new mechanism must justify why an existing function cannot accept the needed argument.

The diagnostic and dependency maintenance do not constitute approval for production fixes. Review this report and approve the concrete production scope before implementation.
