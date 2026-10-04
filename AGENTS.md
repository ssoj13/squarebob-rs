# AGENTS.md

## Bug-hunt operating notes

This is the `squarebob-rs` Rust package and `squarebob` binary (`Cargo.toml:1-13`). Source paths below were checked on 2026-09-23. On 2026-09-23, the first user-authorized `python bootstrap.py b` release build failed in upstream `vfx-io` with `E0599`. After the upstream EXR fix and a local `egui_dock` `TabViewer::id` implementation (`src/app/dock.rs:105-110`), a second release build succeeded. No tests ran; see [plan11.md](plan11.md) for that build sequence and the warnings observed then. The completed systemic repairs are tracked in [plan12.md](plan12.md); [plan13.md](plan13.md) records the subsequent warning and CPU-picking fixes. Read [DIAGRAMS.md](DIAGRAMS.md) for Mermaid dataflows. Neither pass ran tests or a visual comparison.

Primary constraints for future agents:

- Keep owned scan data on the UI side. `DirEntry.rect` uses `Cell`; scanner/cache workers transfer owned trees through channels.
- Use `ScanRoot`'s canonical path and native-path ID for operational identity. Its `display` string preserves user spelling for UI/history (`src/path_key.rs:6-62`). Cache and exclusion validation now use the canonical identity; inspect `plan12.md` for the source review and outstanding verification.
- The standard walker calls `fscan_rs::scan_standard`; its entry callback builds the owned tree and reports progress (`src/scanner.rs:288-421`). It reconstructs omitted parent directories from yielded paths (`src/scanner.rs:302,315-366`). The Windows NTFS adapter calls `fscan_rs::scan_ntfs_tree_with_progress` and converts its tree to owned `DirEntry` nodes (`src/scanner_ntfs.rs:62-150`). `Cargo.toml:179` and `Cargo.lock:3337-3345` pin the private GitHub `fscan-rs` revision `c33d4474b5004ec047ecd065f96f6a53addbc97e`. Its public NTFS tree API preserves typed `ScanFailure` variants (`../fscan-rs/src/ntfs.rs:394-433`).
- A scan generation owns a `ScanSession`, progress receiver, and separate terminal receiver. Replacement cancels and retires the prior session; `poll_scan` discards stale generation/root outcomes (`src/scanner.rs:85-153`; `src/app/scan_orchestration.rs:38-69,456-528`).
- `CacheService` owns ordered cache I/O, generation watermarks, and atomic replacement. Only complete live scans queue a cache store (`src/cache.rs:107-245,893-905`; `src/app/scan_orchestration.rs:237-250`). Flat cache v4 still carries a serialized display-path field for decoding, but runtime validation uses canonical root ID (`src/cache.rs:39-48,523-535,786-797`).
- `render_core::gpu::GpuContext::new` is the wgpu device setup source. `main.rs:145-187` passes its instance/device/queue through the native display host to the app renderers.
- Use central `readback_texture`, `map_readback`, and `map_buffer_read` helpers. They return `Result` for layout, map, poll, channel, mapped-range, and host-allocation failures (`crates/render-core/src/lib.rs:551-805,807-839`).
- Keep native 2D/3D textures on the eframe device. CPU pixels/readback serve 2D CPU, fallback, and screenshot paths (`src/app/treemap_view.rs:20-98`; `src/app/mod.rs:608-647`).
- Trace `#[allow(dead_code)]`, TODO/FIXME, feature gates, and platform stubs before deleting them. Preserve unrelated worktree changes.

## Workspace inventory

`Cargo.toml:15-37` lists 20 members: `squarebob-core`, `pt-core`, `bvh-gpu`, `pt-megakernel`, `pt-wavefront`, `pt-mats`, `render-core`, `render-shared`, `render-3d`, `media-encoder`, `xtask`, `treemap`, `pt-denoise-oidn`, `gpu-mem`, `standard-surface`, `squarebob-widgets`, `pt-material`, `playa-ae`, `egui-colorpicker`, and `color-pipeline`. Earlier file counts and bug-hunt artifact claims were historical snapshots.

## Application dataflow

```text
CLI -> cli::parse_args: Result; main exits 2 on invalid input
    -> GpuContext::new -> shared device -> display_host native owner -> App::new
    -> App::start_scan
         -> ScanRoot(display, canonical path, id)
         -> exclusions::load -> valid policy or visible warning + edit guard
         -> CacheService::Load(generation) -> optional cache preview
         -> scanner::spawn(generation, fscan-rs standard | NTFS MFT)
              -> NTFS unavailable at selection: choose standard backend
              -> NTFS unavailable: terminal warning + standard fallback
              -> NTFS cancelled: Cancelled; fatal error: Failed
              -> Progress + Terminal(Completed | Partial | Cancelled | Failed)
         -> App::poll_scan: reject stale generation/id; install tree
         -> complete scan: CacheService::Store -> atomic cache write
    -> display_root -> App::ui_treemap
         -> 2D CPU -> pixel buffer -> egui texture
         -> 2D GPU -> GpuRenderer2D -> native egui_wgpu texture
         -> 3D raster/PT -> Renderer3D -> native egui_wgpu texture
    -> optional screenshot -> capture_viewport(Result) -> save_png(Result)
         -> success: mark taken/optional exit; failure: retry/dismiss UI
```

Sources: `src/main.rs:19-28,145-185`; `src/app/scan_orchestration.rs:299-363,456-528`; `src/app/treemap_view.rs:20-98`; `src/app/screenshot.rs:12-77`.

## Scan and cache codepath

```text
start_scan
  |-- retire old scan; clear presentation; advance generation
  |-- ScanRoot::from_input -> canonical path + stable id
  |-- exclusions::load; warn and block edits on unreadable policy
  |-- CacheService::load(generation, root)
  `-- scanner::spawn(generation, root, backend)
        |-- fscan-rs standard OR NTFS MFT; fallback only if NTFS unavailable
        |-- scanner::finish_build: sort tree + derive stats
        |-- complete tree: serialize_cache_ref on scan worker
        `-- terminal channel delivers typed ScanOutcome
poll_scan
  |-- poll ordered cache events; gate by generation + root_id
  |-- drain progress and terminal channels; gate by generation + identity
  |-- install_tree -> rebuild_display_tree + invalidate layout/render
  `-- complete outcome -> queue CacheService::store -> atomic_file::write
```

Sources: `src/app/scan_orchestration.rs:38-69,130-250,299-528`; `src/scanner.rs:134-235`; `src/cache.rs:90-245,273-307,893-905`.

## Rendering and readback codepath

```text
App::ui_treemap
  |-- native 2D GPU: render_2d_callback -> render_to_texture -> egui_wgpu
  |-- native 3D: render_3d_callback -> render_to_view -> prepare_scene
  |      |-- empty scene: clear color/object ID; reset picking/UI selection; skip OIDN
  |      |-- raster passes + active object-ID picking; remap selection by path on rebuild
  |      |-- shift-drag marquee: path baseline -> active instance IDs per preview/commit
  |      `-- path tracing + current Object ID for outline + optional OIDN denoise
  `-- legacy: render_treemap -> CPU 2D OR GPU/3D pixel readback -> egui upload
        3D readback also uses prepare_scene + shared pass encoding
GPU pixel readback
  -> TextureReadbackLayout::new (checked geometry)
  -> readback_texture (reusable staging buffer)
  -> map_readback -> map_buffer_read
  -> callback Result/channel -> fallible host allocation -> ReadbackError or packed Vec<u8>
```

Sources: `src/app/treemap_view.rs:20-98,1034-1217,1373-1430`; `src/app/mod.rs:569-659`; `crates/render-3d/src/lib.rs:467-590,1430-1510`; `crates/render-3d/src/renderer3d/render.rs:18-60,112-138`; `crates/render-core/src/lib.rs:551-805,810-839`. The old double-`unwrap` diagram described a previous implementation.

## Media export codepath

```text
Comp::get_frame
  |-- video: encode_sequence_from_comp
  |      -> crop/conditional tonemap
  |      -> VideoEncoder::open/push/finish
  |      -> codec conversion + MovWriter + private partial file + commit
  `-- image sequence: encode_image_sequence
         -> target-depth tonemap (U16 retains float precision)
         -> PNG/TIFF/TGA/EXR/JPEG writer
         -> TIFF selected compression; TGA selected RLE
```

Sources: `crates/media-encoder/src/dialogs/encode/encode.rs:1148-1280,1657-1762,1769-1937`; `crates/media-encoder/src/dialogs/encode/video.rs:66-118`. The TIFF/TGA compression and U16 conversion changes are reviewed in plan12; plan11 retains the original findings. App image-sequence capture currently supplies RGBA8 (`src/app/image_sequence.rs:257-267`), so its U16 output cannot regain lost source precision.

## Current bug-hunt focus

[plan13.md](plan13.md) records the bug-hunt pass at `6562a5280c42195345dee2fef36c59254ee7d894`; it resolved the four `glam` warnings recorded in [plan12.md](plan12.md). The scanner migration now uses a pinned GitHub revision. `cargo check` and the scanner unit tests passed on Windows; large-scan performance and other platforms remain unverified. [plan14.md](plan14.md) records the earlier documentation checkpoint and publication update. [plan15.md](plan15.md) records the typed NTFS outcome repair, the cleanup of incidental lockfile changes, and current CI/runtime verification gates. The current repair passed `cargo metadata --locked --no-deps` and `cargo check --workspace --locked -q` on Windows with empty check stderr; no tests were run for it. [plan12.md](plan12.md) retains the systemic repair and runtime gates; [plan11.md](plan11.md) retains the original evidence and wider workspace audit backlog. Check historical source references against the current worktree before use.

<!-- gitnexus-rs:start -->
# GitNexus-rs — Code Intelligence

This project is indexed by gitnexus-rs as **squarebob-rs** (5697 symbols, 13022 relationships, 300 execution flows). Use the gitnexus-rs MCP tools to understand code, assess impact, and navigate safely.

> Call `graph_status` when freshness matters. Use `reanalyze` (incremental) or `gitnexus-rs analyze` (full). `detect_changes` does **not** re-index.

## Always Do

- **MUST run impact analysis before editing any symbol.** Before modifying a function, class, or method, run `impact({target: "symbolName", direction: "upstream"})` and report the blast radius (direct callers, affected processes, risk level) to the user.
- **MUST run `detect_changes({scope: "all"})` before committing** to verify your changes only affect expected symbols and execution flows.
- **MUST warn the user** if impact analysis returns HIGH or CRITICAL risk before proceeding with edits.
- When exploring unfamiliar code, use `query({query: "concept"})` to find execution flows instead of grepping. It returns process-grouped results ranked by relevance.
- When you need full context on a specific symbol — callers, callees, which execution flows it participates in — use `context({name: "symbolName"})`.
- **Check graph freshness** with `graph_status` before trusting query/impact results on a repo you have been editing.
- **Update the graph** with `reanalyze` (incremental) or `gitnexus-rs analyze` (full). `detect_changes` does **not** re-index.
- **Uncommitted edits** while commit is fresh: `reanalyze` with `scope: "unstaged"`, or run `gitnexus-rs watch` in a terminal.

## Never Do

- NEVER edit a function, class, or method without first running `impact` on it.
- NEVER ignore HIGH or CRITICAL risk warnings from impact analysis.
- NEVER rename symbols with find-and-replace — use `rename`, which understands the call graph (it previews by default).
- NEVER commit changes without running `detect_changes` to check affected scope.

## Resources

| Resource | Use for |
|----------|---------|
| `gitnexus://repo/squarebob-rs/context` | Codebase overview |
| `gitnexus://repo/squarebob-rs/clusters` | All functional areas |
| `gitnexus://repo/squarebob-rs/processes` | All execution flows |
| `gitnexus://repo/squarebob-rs/process/{name}` | Step-by-step execution trace |

## Skills

| Task | Read this skill file |
|------|---------------------|
| Understand architecture / "How does X work?" | `.claude/skills/gitnexus-rs-exploring/SKILL.md` |
| Blast radius / "What breaks if I change X?" | `.claude/skills/gitnexus-rs-impact-analysis/SKILL.md` |
| Trace bugs / "Why is X failing?" | `.claude/skills/gitnexus-rs-debugging/SKILL.md` |
| Rename / extract / split / refactor | `.claude/skills/gitnexus-rs-refactoring/SKILL.md` |
| Tools, resources, schema reference | `.claude/skills/gitnexus-rs-guide/SKILL.md` |
| Index, status, clean, wiki CLI commands | `.claude/skills/gitnexus-rs-cli/SKILL.md` |

<!-- gitnexus-rs:end -->

## OIDN bridge audit and verification — 2026-10-02

Read [plan16.md](plan16.md) for the current progressive-highlight investigation and dependency update. Detailed source evidence is in [the bridge audit](../oidn-rs/bughunt/squarebob_bridge.md); the OIDN parity findings remain in [OIDN plan2](../oidn-rs/plan2.md). The symptom is localized noise in highlights/fine details growing within one progressive run. No source defect is assigned as its measured root cause yet. Production repairs require approval after the report; the diagnostic example and dependency/lockfile maintenance are separate authorized work.

```text
App: current SPP + target + interval/manual/final trigger
      treemap_view.rs1491-1532
                      |
PT output Rgba32Float + active-backend AOV vec4 sums/counts
      render-3d/lib.rs1141,1153,1159
                      |
OidnDenoiser::denoise(ctx, encoder, color_tex, AOVs, current_spp)
      pt-denoise-oidn/lib.rs249-257; encoder consumed
                      |
shared renderer Device/Queue -> tensor allocation -> external input copy
                      |
trim row padding -> HDR luminance clamp dependent on current_spp
AOV RGB/max(W,1) -> NCHW -> fresh tensors into immutable committed model
                      |
env scale > physical-camera scale > internal autoexposure
resolve bytes -> model/tiling -> execute_tensors -> HWC RGBA alpha1
                      |
get_resource flush/pin -> buffer-to-result_texture copy -> device.poll
                      |
result_view -> composite_overlay -> render_view -> display
```

Checked bridge invariants:

- Input and output share the renderer GPU device/queue in the normal path (`crates/pt-denoise-oidn/src/lib.rs:715-727`). Result goes to a separate texture (`:649-656`); display composition targets `render_view` (`crates/render-3d/src/lib.rs:1217-1222`). The inspected bridge contains no output-to-raw feedback.
- AOVs are divided by their per-pixel count before channel conversion (`crates/pt-denoise-oidn/src/lib.rs:906-919`), rather than by global SPP.
- The immutable committed model receives fresh input tensors per pass (`:559-561,627`). Historical comments claiming a buffer race (`:658-675`) are not proof of a present race. Checked pinned CubeCL `get_resource` executes pending streams and pins allocation while the external output copy submits; see the audit for exact dependency paths.
- Public setters are `set_input_clamp`, `set_adaptive_clamp`, `set_nan_protect`, and `set_external_input_scale`. Public output is `result_view()`; `result_texture` is private.
- `OIDN_STANDALONE_DEVICE` creates a different device but direct copies remain on the renderer device (`:707-711,392-422,953-1012`), so it is not a valid direct-copy A/B experiment.

Open source findings:

- App defaults clamp 10/adaptive=true/interval 128 (`crates/render-shared/src/lib.rs:1128-1135`); effective clamp6 at 128 SPP rises to 10 at 256 SPP (`crates/pt-denoise-oidn/src/lib.rs:457-468`). This intentionally changes highlight input, even for a frozen raw snapshot; test it separately from progressive convergence.
- Any periodic success sets `oidn_denoised_this_accumulation=true` (`src/app/treemap_view.rs:1609`), blocking `auto_final` (`:1510-1513`). Target300/interval 128 can leave the 256 SPP preview on screen.
- The bridge drops resolved model stem metadata while keeping bytes (`crates/pt-denoise-oidn/src/lib.rs:548-552,597`); Fast can encounter the audited Small override defect. High/noisy-aux ordinarily falls back to Base; observe the actual stem before blaming Large overlap.
- Physical-camera scale bypasses autoexposure (`src/app/treemap_view.rs:1587-1591`), while Manual without an env override uses it. Do not attribute default Physical-camera noise to the autoexposure defects without verifying settings.

Do not remove clamping, AOVs, exposure, models, or preview modes. Prefer a validated model descriptor through the existing resolver/commit path, one completed-SPP plus accumulation identity for scheduling, and shared scale/device error validation. Record raw-SPP and denoised-SPP separately. Initial frozen-input GPU probe and 32 additional fixed-clamp repeats passed on RTX 3080 Ti/Vulkan: repeated RGB max difference 0, while adaptive-SPP1/256 changed by 8.448264122. Workspace check and actual squarebob binary build both passed with empty stdout/stderr. This establishes only the tested configuration's stability and input-policy effect. Runtime results, complete SSH ref table, and remaining gates are tracked in plan16.

## Native numerical and PQ follow-up — 2026-10-02

Current evidence continues [OIDN plan3](../oidn-rs/plan3.md), [Squarebob plan17](../squarebob-rs/plan17.md), [native runtime](../oidn-rs/bughunt/native_runtime.md) and [Astra numerics](../oidn-rs/bughunt/astra_numerics.md). Earlier audit/no-runtime statements describe their historical pass; later diagnostic testing was explicitly authorized. PQ display production changes have separate explicit authorization; unrelated denoiser/scheduling repairs remain unapproved proposals.

All 23 local Rust archive sizes/SHA256 values now match isolated pinned native archives at28883d1769d5930e13cf7f1676dd852bd81ed9e7. This supersedes the earlier asset-availability limitation without modifying the supplied native checkout. 90/90 synthetic nativeCPU/CUDA/RustWGPU executions passed with finite output. Explicit-scale color-only Rust/nativeCPU max absolute error0.0003814697265625; aligned full-AOV32x32 max0.000213623046875. Odd/tiny autoexposure and unaligned normal-padding comparisons differ substantially. Scope is synthetic CLI host inputs, not all modes or the user's scene.

The checker's adaptive-clamp spatial-SD change restores fixture contrast after clipping, so it is not a noise-only metric. Native CUDA half storage and quality-dependent accumulator policy differ from checked Burn f32 Direct convolution; default fusion/autotune was not enabled in Astra's checked target. The actual progressive highlight-noise cause remains unproven.

The current Squarebob display source uses `src/display_host.rs` plus canonical `egui-display` (`06acf66506583e2cef07450ac304d8ae0414b59d`). Actual negotiated output HDR/white is propagated before app UI/rendering; ColorSettings runtime fields select OCIO display-reference decoding and Rec.709 light relative to reference white. The shared presenter owns final SDR/PQ/HLG/scRGB encoding once. An unsupported requested PQ mode falls back with an explicit actual-state error. Initial workspace check passed, while final color/shader/host/binary/actual-window validation is still in progress; see plan17.

```text
scene-linear renderer / OIDN result
 -> exposure + OCIO selected view/look/LUT
 -> display-reference XYZ D65 -> Rec.709 display light / reference white
 -> float extended-sRGB GUI/renderer canvas
 -> shared egui-display PresentPass
 -> negotiated SDR/PQ/HLG/scRGB surface
```

Anchors in Squarebob: `src/main.rs:187`, `src/display_host.rs:212-231,309-310,389-455,520-584`, `src/app/mod.rs:59-64`, `crates/color-pipeline/src/lib.rs:193-198,772-808,875-887`. Canonical presenter anchors: `egui-widgets-rs/crates/egui-display/src/present.rs:25,127-149,202-215,612,766`. Compilation and float readback do not certify an actual HDR monitor surface or physical luminance.

## CPU/PQ verification closure — 2026-10-02

This closure supersedes earlier pending CPU/PQ test statements and qualifies earlier no-feedback conclusions as GPU-path evidence. Historical pass results and source anchors remain preserved above.

The user explicitly authorized the old CPU display repair and push to main in addition to PQ integration. Unrelated denoiser/scheduling proposals remain unapproved. [Squarebob plan17](../squarebob-rs/plan17.md), [Astra post-fix review](../oidn-rs/bughunt/astra_pq_review.md), and [OIIO CPU review](../oidn-rs/bughunt/oiio_cpu_display.md) record the checked contracts and results.

The historical no-feedback conclusion described the inspected GPU bridge; CPU display had existing raw-buffer feedback/exposure-order and denoised-view defects. The shared composition repair reads raw PT/OIDN, applies CPU exposure before OCIO, and writes a separate reusable display scratch. Source: Squarebob `crates/render-3d/src/lib.rs:1190-1236`, `crates/pt-megakernel/src/compute.rs:5383-5495`, and both callers `src/app/treemap_view.rs:699-708,1193-1205`. No matching defect was found in the scoped OCIO/OIIO library inspection.

Color6/6, host3/3, render-core3 plus GPU1, extended-sRGB/full-PresentPass signal and CPU immutability/order probes passed. The signal probe covered gray plus six saturated-primary cases (max PQ primary error0.000542;203nits0.580652;1000nits0.751720). Final locked workspace/all-target check passed9.381s; actual binary build passed33.406s, both empty logs. CPU probe preserved PT/external raw bytes and repeated output, with exposure2 before OCIO matching the processor oracle. Actual GUI retry and confirmed remote push remain pending; these measurements do not certify physical monitor luminance. Context7 supplied official winit0.30 changelog evidence; unavailable/timed-out GitNexus requests used direct-source fallback.

## Final window and persistence verification — 2026-10-02

This supersedes the preceding pending GUI retry and final-check status. Actual PBR, PT GPU, PT CPU and newly saved CPU-state restart windows completed successfully. Logs confirm actual HDR10(PQ), `Rgb10a2Unorm/Bt2100Pq`; full UI capture `squarebob_pq_ui_controls.png` was inspected by the root reviewer. CPU/GPU controls are visible; all five output modes, reference-white Auto/manual and HLG peak controls exist in the checked host. Auto white is240nits. OS-reported peak603nits/full-frame150nits/headroom2.51/10bits are metadata, not physical luminance measurements. Restart preserved CPU color path, ISO240, f/1, shutter1 and effective scale2.

App state now uses a typed RON payload inside the existing outer storage map, while DisplayPrefs stays a JSON string. `Squarebob src/app/state.rs:142-149` accepts RON plus valid legacy JSON; `Squarebob src/app/mod.rs:782-834` saves the complete typed state. `Squarebob src/app/persistence_tests.rs:5-43` verifies full-state roundtrip including nonfinite dock rectangle sentinels, finite floating-window position/size, topology and CPU/camera settings. Previously corrupted JSON null-rectangle snapshots report decode failure; no geometry is guessed or stripped to migrate them.

Final binary unit suite:34/34 passed in0.13s (`squarebob_pq_bin_tests.stdout.log`). Final locked workspace/all-target check passed in6.313s with empty logs; binary build passed in16.140s with empty logs. Earlier measurements remain historical. Final Clippy exited0 in10.685s, with464 stderr lines of warnings (`pq_final_clippy.stderr.log`); this is not a warning-free result. The official Rust API checklist was consulted for getter naming, validation, meaningful errors and intermediate results; Context7 supplied official winit0.30 documentation.

Manual clipboard/accessibility, multi-monitor transitions, all alternative output modes and physical luminance remain unverified. Actual user-scene progressive noise remains unresolved. Production work and review are complete in the verified scope. Main publication is explicitly authorized; its confirmed commit/remote receipt is recorded separately in OIDN plan3.
