# plan17 — PQ presentation and native numerical follow-up

Updated:2026-10-02. Continues [plan16](plan16.md), [OIDN plan3](../oidn-rs/plan3.md), and the earlier static audit. The user explicitly authorized implementing PQ when missing. **PQ and the old CPU display repair are explicitly authorized; unrelated OIDN/scheduling repairs remain proposals.** Preserve rendering, SDR output, persistence, input, accessibility, screenshots and application lifecycle.

## Implemented source contract — verified scope

The previous eframe window presented SDR. The current source replaces that owner with a generic native host and uses the existing shared `egui-display` presenter, rather than another local PQ encoder.

```text
PT/denoised scene-linear RGB
 -> exposure / OCIO view + look/LUT
 -> selected display color space -> display-reference XYZ D65
 -> Rec.709 display light relative to negotiated reference white
 -> float extended-sRGB transport canvas (GUI + renderer)
 -> egui-display PresentPass decodes canvas to light
 -> selected SDR/PQ/HLG/scRGB signal once -> actual surface
```

Current checked source anchors:

| Contract | Source |
| --- | --- |
| Native owner and app construction | `src/main.rs:6,187`; `src/display_host.rs:102-145,181-268` |
| Float egui canvas and shared presenter | `src/display_host.rs:212-231,256-268,520-584` |
| Renderer float display targets and explicit SDR/raw readback | `crates/render-core/src/lib.rs:5,652-778`; `crates/render-3d/src/targets.rs:50`; `crates/render-3d/src/pt/megakernel/render.rs:48` |
| Actual negotiation before app UI | `src/display_host.rs:309-310,389-455`; `src/app/mod.rs:59-64` |
| Runtime-only actual HDR/white settings and hash | `crates/color-pipeline/src/lib.rs:193-198,258-259,316-317` |
| HDR view decoding and luminance units | `crates/color-pipeline/src/lib.rs:772-808`: output color space -> display-reference XYZ -> Rec.709; HDR units100nits/reference white |
| CPU/baked-LUT output scale | `crates/color-pipeline/src/lib.rs:523-541,818,875-887` |
| HDR view availability follows actual target | `src/app/settings/color.rs:497-529` |
| Existing settings/storage and window/egui persistence | `src/display_host.rs:47-90,125-131,279-302` |
| Window events, accessibility, repaint, autosave/exit | `src/display_host.rs:641-769`; exact line numbers may move during validation fixes |
| Screenshots share presenter, captured as SDR for existing internal API | `src/display_host.rs:587-635` |

The canonical presenter is local/locked `egui-widgets-rs` revision `06acf66506583e2cef07450ac304d8ae0414b59d`, checked independently. `crates/egui-display/src/present.rs:25` defines the float canvas; `:127-149` selects supported format/color-space pairs including HDR10 RGB10A2/Bt2100Pq; `:202-215` implements the shared PQ curve; `:612,766` constructs/draws PresentPass. Squarebob uses the negotiated output state, not merely the requested preference. Unsupported HDR10 falls back visibly to supported SDR; it is not reported as actual PQ output. Shared default levels/limits and monitor state remain owned by egui-display.

HDR display-reference conversion is separate from OIDN HDR radiance preprocessing. The shared presenter encodes PQ exactly once. The renderer's scene/render-texture screenshot path is distinct from the native host's whole-window presentation capture; qualify each diagnostic by which path it measures. Do not pass PQ code values as scene-linear or canvas light. SDR OCIO behavior remains supported. Runtime `output_hdr`/white fields are not user-persisted scene settings. Render-core uses Rgba16Float extended-sRGB display textures; semantic SDR readback clips/converts them to RGBA8, while raw-byte readback retains float channels. A clipped screenshot is not a measurement of HDR headroom or PQ code values.

## Completed and pending work

- [x] Confirm earlier SDR-only output and blocked HDR OCIO path.
- [x] Reuse shared egui-display API and verify its local revision matches lock pin.
- [x] Implement generic native owner and actual-output state propagation.
- [x] Preserve eframe settings keys/egui/window persistence and lifecycle hooks in source.
- [x] Add negotiated HDR/white runtime context and OCIO display-reference conversion.
- [x] Retain supported SDR fallback and explicit unsupported-output explanation.
- [x] Initial `cargo check --workspace --quiet` passed exit0 in4.476s; stdout/stderr empty. Evidence: [stdout](../oidn-rs/bughunt/squarebob_pq_check.stdout.log), [stderr](../oidn-rs/bughunt/squarebob_pq_check.stderr.log). This predates any subsequent validation fixes.
- [x] Validate host persistence/corrupt-storage behavior with focused unit tests:3 passed.
- [x] Validate bounded CPU/GPU OCIO/display-light/extended-sRGB and shader contracts; see final verification below.
- [x] Re-run locked workspace/all-target check and actual binary linking after final source changes.
- [x] Launch PBR/PT GPU/PT CPU windows and verify actual PQ pair, visible controls and OS-reported metadata; multi-monitor/recovery and alternate-output runtime gates remain unverified.
- [x] Verify renderer/full-window captures, timed close and lossless CPU-state restart. Manual clipboard/accessibility and broader lifecycle cases remain unverified.
- [x] Record measured offscreen display-light/PQ values and actual GUI PQ surface pair; physical monitor luminance remains unmeasured.
- [ ] Complete actual-scene progressive capture and noise metrics; PQ authorization does not authorize unrelated source repair.

## Native OIDN measurements and corrected interpretation

See [native runtime report](../oidn-rs/bughunt/native_runtime.md), [Astra numerics](../oidn-rs/bughunt/astra_numerics.md), and reproducible JSON artifacts. All 23 Rust weight sizes/hashes equal pinned native archives. Official isolated OIDN2.5 CPU/CUDA and Rust WGPU produced 90/90 finite successful synthetic outputs. Color-only explicit-scale Rust/nativeCPU max absolute error is0.0003814697265625 in the measured corpus; aligned full-AOV32x32 max0.000213623046875. Unaligned AOV and odd/tiny automatic exposure have much larger differences, consistent with audited normal-padding/exposure contracts. These are CLI host-path tests, not a full Squarebob bridge/scene parity certificate.

The checker spatial-SD rise is not proof of added noise: effective threshold6 removes both levels' contrast before inference, while threshold10 restores it. The fixed-clamp repeated-input results in plan16 remain valid bounded repeatability evidence. The schedule plateaus at256SPP; it cannot alone explain indefinitely growing threshold after256. Native CUDA half-storage/quality accumulation policy differs from checked Burn f32 Direct execution; no default Burn fusion/autotune feature was enabled in Astra's checked graph. Do not prescribe half precision or blame a changing autotuned kernel without matched-stage evidence.

## Actual-scene diagnostic gates

Capture early/late and above256SPP raw normalized color, AOV sums/counts, post-clamp RGB, scale, denoised output and displayed snapshotSPP. Use identical display transform and bright-detail plus dim control ROIs. Separate restored clean detail from residual stochastic error using clean targets and controlled noise/seeds; raw spatial SD is insufficient.

Keep color/AOV/model/scale/context fixed for first-divergence backend comparisons. Record all effective settings and dependency/device pins. Frozen-input replay, GPU ownership/completion boundaries and per-stage finite counts remain separate from PQ window negotiation. No actual user-scene noise cause is established.

## Resumption and approval boundary

Docs owner maintains this plan and architecture diagrams; implementation agents own their assigned source files. Never revert others or change runtime claims merely from comments. Mark checks only after exact command/result or actual GUI evidence is recorded. Authorized PQ/CPU implementation is complete in the verified scope; unrelated production repairs still need concrete report approval.

## Authorized CPU repair and final verification — 2026-10-02

The user explicitly authorized fixing the old CPU display defects and pushing reviewed Squarebob changes to main. This supersedes the approval boundary for CPU/PQ display work only. Unrelated denoiser/scheduling repairs still require approval. Push is pending until a remote commit/result is recorded.

[Astra post-fix review](../oidn-rs/bughunt/astra_pq_review.md) confirms both CPU findings are resolved in inspected source. They existed in historical HEAD, rather than being introduced by PQ integration. The earlier bridge no-feedback claim covered GPU composition only. Shared `Renderer3D::composite_overlay` now accepts optional raw source, options and ColorPipeline (`crates/render-3d/src/lib.rs:1190-1236`); both App callers share it (`src/app/treemap_view.rs:699-708,1193-1205`). CPU physical exposure runs before OCIO, then a separate reusable display scratch is written (`crates/pt-megakernel/src/compute.rs:5383-5495`). PT accumulation and OIDN result textures remain raw. Repeated display refresh cannot compound the view transform. The CPU blit receives exposure1 after preprocessing. [OIIO review](../oidn-rs/bughunt/oiio_cpu_display.md) found no matching library defect in the inspected immutable plain Processor/fresh-result display contracts; no OIIO changes were needed.

| Verification | Observed result |
| --- | --- |
| Color pipeline unit tests |6/6 passed; `../oidn-rs/bughunt/squarebob_pq_color.stdout.log` |
| Native host tests |3/3 passed; `../oidn-rs/bughunt/squarebob_pq_host.stdout.log` |
| Render-core tests |3 passed plus separately executed GPU test1 passed; native-verification logs |
| Extended-sRGB shader and full PresentPass probe | Passed on RTX3080Ti/Vulkan. Gray plus six saturated-primary cases; max primary PQ error0.000542. 203nits code0.580652,1000nits0.751720; `squarebob_pq_signal.*` logs |
| Final locked workspace/all-target check | Exit0,9.381s, empty stdout/stderr; `pq_final_check.*` |
| Actual binary build | Exit0,33.406s, empty stdout/stderr; `squarebob_pq_build.*` |
| CPU raw immutability/order probe | Passed10.422s: external OIDN-like source and PT default source preserve original bytes; repeated outputs equal; exposure2 before OCIO matches processor oracle; `display_blit_cpu_probe.*` |
| Actual GUI | Initial texture-delta guard issue was corrected; actual PBR/PT GPU/PT CPU plus CPU restart passed. See final verification below. |
| Push main | Explicitly authorized and prepared; confirmed remote receipt belongs in OIDN plan3. |

Signal tests measure offscreen encoded output, not physical monitor luminance or full GUI lifecycle. The Context7 consultation used the official winit0.30 changelog. GitNexus requests remained unavailable/timed out; direct source/caller inspection is the documented fallback.

```text
raw PT or raw OIDN result (read-only)
 -> shared composite_overlay
 -> CPU lane: exposure -> OCIO -> reusable separate display scratch
 -> GPU lane: exposure -> OCIO during blit
 -> float extended-sRGB render target / GUI canvas
 -> canonical PresentPass -> negotiated output signal
```

- [x] Complete focused host/color/render-core and GPU signal verification.
- [x] Verify CPU source immutability, repeated refresh and exposure ordering.
- [x] Complete final locked check and binary build.
- [x] Review OCIO/OIIO contracts and Astra CPU/PQ source findings.
- [x] Complete actual PBR/PT GPU/PT CPU window retry and lossless CPU-state restart; broader manual lifecycle gates remain below.
- [x] Prepare explicitly authorized main publication; remote receipt is recorded separately in OIDN plan3.

## Final window and persistence verification — 2026-10-02

This supersedes the preceding pending GUI retry and final-check status. Actual PBR, PT GPU, PT CPU and newly saved CPU-state restart windows completed successfully. Logs confirm actual HDR10(PQ), `Rgb10a2Unorm/Bt2100Pq`; full UI capture `squarebob_pq_ui_controls.png` was inspected by the root reviewer. CPU/GPU controls are visible; all five output modes, reference-white Auto/manual and HLG peak controls exist in the checked host. Auto white is240nits. OS-reported peak603nits/full-frame150nits/headroom2.51/10bits are metadata, not physical luminance measurements. Restart preserved CPU color path, ISO240, f/1, shutter1 and effective scale2.

App state now uses a typed RON payload inside the existing outer storage map, while DisplayPrefs stays a JSON string. `src/app/state.rs:142-149` accepts RON plus valid legacy JSON; `src/app/mod.rs:782-834` saves the complete typed state. `src/app/persistence_tests.rs:5-43` verifies full-state roundtrip including nonfinite dock rectangle sentinels, finite floating-window position/size, topology and CPU/camera settings. Previously corrupted JSON null-rectangle snapshots report decode failure; no geometry is guessed or stripped to migrate them.

Final binary unit suite:34/34 passed in0.13s (`squarebob_pq_bin_tests.stdout.log`). Final locked workspace/all-target check passed in6.313s with empty logs; binary build passed in16.140s with empty logs. Earlier measurements remain historical. Final Clippy exited0 in10.685s, with464 stderr lines of warnings (`pq_final_clippy.stderr.log`); this is not a warning-free result. The official Rust API checklist was consulted for getter naming, validation, meaningful errors and intermediate results; Context7 supplied official winit0.30 documentation.

Manual clipboard/accessibility, multi-monitor transitions, all alternative output modes and physical luminance remain unverified. Actual user-scene progressive noise remains unresolved. Production work and review are complete in the verified scope. Main publication is explicitly authorized; its confirmed commit/remote receipt is recorded separately in OIDN plan3.
