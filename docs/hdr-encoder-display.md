# HDR, encoder, and display parity with WarpBro

Current design and status, 2026-10-07. The user requested full parity, deduplication and
one source of truth. Reuse actual WarpBro export/color/display behavior through the existing
egui-display export module and thin application adapters. Keep existing formats/codecs and
UI behavior. This document describes the current path and its open gates; it does not certify
unfinished consumer code.

## One shared implementation

The export extraction was published in `3443b6f46a2a72526c9542b1e7d8e528c88142bc`;
current shared policy source is `d4117f50`. Latest changelog `8127947` changes docs only;
the consumed source is byte-identical to d411 and needs no rebuild for that prose update.
[egui-display export.rs](../../egui-widgets-rs/crates/egui-display/src/export.rs) contains
the actual primitives extracted from [WarpBro export.rs](../../warpbro-rs/src/export.rs):
PNG encoding, display-light scaling, HDR measurements, PNG-sequence video, FFmpeg lifecycle
and canonical SDR gamma. Both applications consume this implementation; adapters supply
their frame source and app/job settings.

Use the existing renderer resources and the existing shared display controls. No new raw
GPU target, GPU interop, replacement display pass or Playa NativeEncoder migration belongs
to this design. Native GPU encoding is reused only where the actual WarpBro implementation
already provides the required path; the port must not invent a new transport architecture.
The separate nodes/profiles proposal is not an implemented runtime dependency.

## Source audit and actual signal domain

SquareBob's earlier [image_sequence.rs](../src/app/image_sequence.rs) path captured the
viewport and constructed `Frame::rgba8`. Higher file bit depth could not recover discarded
HDR precision. Earlier production/metadata-library checkpoints and latest ordinary app tests
passed; latest actual production and workspace checks also passed, as scoped below.

The actual typed canvas is **RGBA16Float containing extended-sRGB codes for Rec.709 display
light**. It is a processed display result, not raw PT radiance, scene-linear light or AP1.
Decode its extended-sRGB RGB through the canonical `display_encoded_to_linear` before
display-light export. Preserve values above 1 and preserve alpha independently. Do not
label this canvas ACEScg or infer its signal from its float storage.

Existing PT output is normalized `Rgba32Float` through `Renderer3D::pt_output_texture`;
OIDN produces a separate result. Those resources do not change the canvas's semantic domain.
CPU/GPU OCIO comparisons operate on the same GPU PT result, not a separate CPU integrator.
Export/display processing must not write back to raw PT or OIDN input.

The audited [PBR shader](../crates/render-3d/shaders/cube_pbr.wgsl) and
[skybox](../crates/render-3d/shaders/skybox.wgsl) use `display_encode`. PBR/sky and
wireframe/background do not yet establish complete parity in selected view/look/exposure,
color interpretation or transparent composition. Validate and port actual WarpBro behavior
at these boundaries using existing resources; a successful PNG writer does not fix them.
A 2D RGBA8 source remains SDR: mapping SDR white onto an HDR output creates no extra detail.

## Source-to-file flow and canonical API

```text
frozen export view/color/time and renderer settings
  -> existing SquareBob render/color path for the intended output
  -> typed display canvas: RGBA16Float extended-sRGB Rec.709
  -> thin source adapter: declared signal + float readback + alpha
  -> shared extended-sRGB decode -> linear Rec.709 display light
  -> explicit DisplayLight and frozen output-white scale
  -> shared PNG/HDR primitives OR existing native video adapter
  -> final format packing, measurement and atomic publication
```

The export view must be selected independently of the monitor. If the current canvas was
rendered for another view, its readback alone cannot supply the required export image.
Evaluate the intended output through the existing color/render path. Freeze output color,
selected view/look, white/peak interpretation, exposure, source/camera/renderer settings,
crop/resize, dimensions and exact timing for the job. Do not read changing monitor policy
as an implicit per-frame export setting.

Newest source routes both window and inline hosts through an immutable post-edit
EncodeLaunchRequest/event, preserving the settings selected by that edit. FrameSourceResult
retains capture errors and Cancelled; unexpected extent is rejected instead of silently cropped.
Completion notification is best-effort after an already successful file commit. These changes
have scoped file/source receipts below. Frozen session ownership and independent SDR-view
selection now pass ordinary tests; the ignored GPU freeze test and full native color parity
remain outside those receipts.

The existing shared Display UI uses reference white 80–1000 nits and HLG peak 400–2000
with Auto/manual controls. Auto applies to white/peak, not a sixth output mode. Unsupported
requested output remains visible alongside the actual SDR fallback and its reason.

| Shared API | Actual contract |
| --- | --- |
| `display_encoded_to_linear` | Invert extended-sRGB RGB without clipping; pass alpha through |
| `DisplayLight` | `Relative`, or `Absolute { peak_nits }`; absolute 1.0 means 100 nits |
| `measure_display_peak` | Probe the actual tone-mapped view returning linear Rec.709 display light; validate its positive finite peak |
| `hdr_scale` / `HdrScale` | Convert relative light using explicit SDR white, or absolute light using 100 nits/unit and the measured view peak; HLG relative light uses the shared reference-display rule |
| `PngEncoding` | `Sdr8`, `Hdr10`, `Hlg`; HDR modes write true 16-bit samples |
| `write_png` | Accept linear Rec.709 display light, scale and an SDR-code callback; validate dimensions/finite HDR data, retain alpha, write atomically |
| `HdrLevels::merge` | Aggregate mastering peak and measured PQ clip MaxCLL/MaxFALL across frames, excluding alpha from RGB light measurement |
| `bt1886_code` / `BT1886_GAMMA` | One canonical SDR-video conversion from relative display light, gamma 2.4 |
| `PngVideoOptions` | Finished sequence, frame range, exact positive `fps_num/fps_den`, codec, encoding and quality |
| `PngVideo`, `ffmpeg_args`, `encode_video` | Off/ProRes/HEVC sequence output; checked arguments and color tags, cancellation/error propagation, final atomic commit |

`Sdr8` still PNG needs codes from the intended SDR view even if the monitor shows HDR.
The native SDR Float/U8 video adapter must decode its actual source and use the same
BT.1886 helper once. The shared external-FFmpeg path converts SDR PNG sRGB codes to
BT.1886 video codes before tagging them. The former mismatch of sRGB pixels and SDR-video
tags is not an accepted output contract.

## Real WarpBro paths and retained scope

WarpBro's scene-linear ACEScg/AP1 EXR is a separate raw-scene path. SquareBob must use a
genuine source with declared working primaries to reproduce it; decoded Rec.709 display
canvas values are not scene-linear AP1. Raw EXR parity remains a gate where that source
adapter is not established.

WarpBro writes SDR8 PNG and PQ16/HLG16 PNG with the shared cICP/mDCV metadata and measured
PQ cLLI. Its native HEVC8 paths include CPU and Vulkan. Its HDR Main10 and ProRes 4444 XQ
path uses a PNG sequence followed by external FFmpeg. The shared `PngVideo` path preserves
that actual implementation; it does not certify new native HDR encoding.

Keep SquareBob's existing H.264/HEVC/AV1/ProRes choices and profiles, PNG/TIFF/TGA/EXR/JPEG,
crop/resize, available depth/compression/alpha behavior and cancellation/publication.
Runtime capability failures name the unsupported combination; no silently HDR-tagged 8-bit
output. Reuse existing audio only when a real source supplies it. Codec capability is not
proof that the adapter delivers correct pixels or precision.

Output processing retains float precision until the actual pack boundary. Mastering values
describe the selected output view/scale; measured light levels describe the clip. Neither
arbitrary constants nor container tags replace the pixel transform. Failed/cancelled jobs
must preserve an existing destination and must not publish a partial video as complete.

## Current receipts and open work

### Latest checkpoint — 2026-10-07

The older receipts below remain historical. Current shared OutputKind/view resolver is
extracted from actual WarpBro policy in egui-display::export and published in `d4117f50`.
Fifteen toolbar tests, six policy tests and strict Rust 1.96 release all-target clippy passed;
sole Apps producer review closed without findings. Earlier `21437` stopped on disk-full
regex-syntax IO before tests; the later passing gates supersede that pending status.

FrozenRenderSession is source-complete across nine files plus ColorPipeline. It owns frozen
DirEntry/camera/quality/options, its own ColorPipeline and existing 3D/2D renderer resources
moved out of author fields. It does not mutate authored time/flags. GPU worker failure and
the final readback drain preserve actual capture causes; strict configuration/LUT-bake failure
is an error. Its 3D SDR source is RGBA16F float, avoiding an 8-bit pre-gamma intermediate.
Freeze source was published in `b487bbb`; camera-restore fix
`6c1e4edbcd470aac466b77c245acbdca2023d350` separates resource invalidation from an
authored-camera action. All nine ordinary Frozen session tests and the finish/cancel ×
failed-path camera regressions passed in latest-source `95027`. Sole independent Plan
review closed with one P2, corrected by that commit. Earlier `95027` recorded a 19m16s
build and 56 passed / one failed / three ignored. Final ordinary gate `48040` passed after
the fixture correction: release compile **1m40s**, runtime **57 passed / zero failed /
three ignored in 0.98s**, as recorded in `%TEMP%/squarebob-final-bin-tests.log`.
Main `8ac9d58f6f4245d431ed7f892e34ad1d9c58d511` is verified against the exact remote,
clean/current. Actual production build `21158` passed in **1m59s** and real squarebob.exe
`--help` exited 0. `cargo check --release --locked --workspace --all-targets` session `33869`
passed in **4m55s**, without warnings/errors. These ordinary tests do not execute the ignored GPU
freeze, motion-movie or preset-screenshot tests, or certify zero-copy encoding/full parity.

Camera-slot/preset/toolbar foundation passed 24 UI tests and one GPU test generating seven
screenshots. Visual inspection found narrow overlap, floating-bar and fade-fixture issues;
geometry fixes are source-ready. Existing camera/preset/store/pointer UI tests passed.
The earlier failing test was
`app::viewport_toolbar::tests::narrow_viewport_toolbar_horizontal_scroll_reaches_camera_slots`
with `toolbar text geometry`: painted-label diagnostics at 280px omitted CamClip completely.
The corrected fixture measures natural ordering with both controls visible at 1200px, then
exercises actual 280px scrolling, camera-button visibility and the separate scrollbar row.
The final suite passed; this correction changes no production UI. Failed-label diagnostics
are retained. No new screenshots are accepted. The user instruction skips extra screenshot/demo generation
after relevant tests pass; it is not a mandatory completion blocker. Five mandatory nullable camera
slots use LMB recall / RMB store. Old screenshots in `C:/Temp/bob/ui-20261007` do not close
narrow acceptance. Full EXR metadata, remaining OCIO/proxy/denoise/snapshot/full-toolbar
features and overall performance/native acceptance remain open.

The actual retained WarpBro reference now includes Kvazaar/Vulkan motion (51 frames,
2.125s, 256x256), a fractional clip (32 frames, 66x50), two-frame 16x16 HDR fixtures and a
nine-frame 256x256 Vulkan partial clip. Actual OIDN HDR/all-quality, PNG-video PQ HEVC/PQ
ProRes/HLG HEVC/SDR ProRes tags+CLL and partial exact `24000/1001` gates each passed.
Use those scoped references without claiming the entire WarpBro suite passed. The earlier
Fast one-object discrepancy is now resolved: specialized radiance and all guides are exact
on all four routes, with unchanged tolerance. Exact retained folders are in
[WarpBro HANDOFF](../../warpbro-rs/HANDOFF.md). The twelve SquareBob three-frame 64x64 codec
fixtures remain technical clips, not real application-motion delivery.

SquareBob floating Git refs were audited against actual remotes and explicit fscan moved
`5f57` → `94dd173`. Owned SHA-matched mirror full-force `54164` and subsequent serialized
`94939`/five-file reindex passed; the canonical original DB remains foreign-locked and unfresh.
No global graph freshness is claimed.
The canonical foreign MCP DB was not repaired. Isolated GITNEXUS_HOME `analyze --out`
wrote fresh owned storage, but CLI graph_status/list_repos read stale canonical `.gitnexus`;
impact could not find a new test. Fresh registry/meta does not certify tool-query freshness.
The authoritative owned mirror now has 39 SHA-verified source files and default local
`.gitnexus`: 5968 nodes / 13831 edges, graph_status HEAD/index `efe1448` fresh at 08:12:27,
then test reindex at 08:16. The newest test impact is LOW, zero direct callers / zero flows.
Those are earlier mirror receipts. Latest authoritative pre-8ac mirror is
`C:/projects/projects.rust.cg/.codex-worktrees/squarebob-main-gate-20261007-0407c6ea38691`:
two latest files SHA-verified, standard local DB 5968 nodes / 13831 edges, HEAD/index `6c1`
fresh at 08:21:08. Precommit detect covered nine symbols / two files, LOW / zero flows.
These receipts certify the matched mirror's graph, not the foreign canonical original.
The `--out` loader defect persists; Apps' canonical resolved-storage architecture fix awaits
source/tests. Exact failure is retained in
oh-my-harness BUG3.md, 2026-10-07.
Earlier CRITICAL camera-path impact/two-file LOW detect and exact remote publication remain
historical receipts, not proof of newer-test graph coverage. Shared widgets' full post-push index
passed. Preserve existing `C:/Temp/bob` files; no gratuitous extra videos/renders after tests pass.

WarpBro reference main `cd505435cfafff60102f6038e55fb44d148f8e36` integrates d411 in four
files (lock/app/gpu/ocio); plain all-target passed in 28.01s without warnings.
Latest specialized-radiance gate passed with all four routes/all guides exact and unchanged
tolerance; 15 OCIO tests passed, including invalid-input/look warm-cache cases.
Ordinary oxide `27252` passed 254 tests, zero failed, ten intentionally ignored and four
previously certified native-movie fixtures filtered in 35.80s, without extra movie reruns.
Paired DE-call
measurement still shows about 27–28% regression. The narrow affine attempt failed the
original oracle because the compiler dropped inline-never intent; it was reverted and the
correct published production source is frozen (docs main `d1bc4ba8`). Authorized cuda-oxide
inline-intent propagation work is pending plan/compile in an owned producer checkout/tool
build, without global replacement. No performance acceptance or completed WarpBro claim.
Strict clippy
stopped at an existing fractal-materials eight-argument factory before WarpBro linting.

Shared source receipts: 31 tests passed, including measured cLLI, real FFmpeg/ffprobe checks
of six SDR/PQ/HLG HEVC+ProRes combinations and cancellation. Strict release all-target clippy
`-D warnings` passed. Publication is confirmed; these are not consumer acceptance receipts.

SquareBob production session `73811` passed on Rust **1.96**: 24m49s bootstrap / 24m34s
Cargo. Library session `57163` passed 19 tests in 7m with canonical FFmpeg metadata
checkpoint `8c4`, excluding newest launch/test changes. The producer published
`bt709_limited` and checked nclx after six release tests; producer success is not full
application acceptance.

Later library/native gates passed 21 + four tests. Latest four native integration gates passed
in 2.51s: alpha, six SDR variants, four PQ/HLG HEVC/ProRes variants and unsupported settings.
Twelve retained files are in `C:/Temp/bob/delivery-20261007-0615`, including two ProRes-alpha
clips. These are three-frame technical fixtures, not real application-motion recordings.
Source-error and extent transactional gate `98764` passed three tests with three ignored in
3m20s. Each receipt covers its tested path; it does not certify full application/UI parity.

The user confirmed named Settings preset buttons. Source uses the existing HashMap SSOT,
New plus RMB Save/Rename/Delete and an atomic av-util-core writer. Raw-pointer, persistence
and GPU screenshot tests have not run; its direct dependency was added after the successful
compile closed. Existing preset storage remains the source of truth; no second catalog is
introduced. Full renderer freeze, monitor-independent SDR still-view, performance and UI
acceptance remain open.

Latest requested source adds WarpBro-style viewport toolbar and CamClip/camera slots.
Five nullable slots persist full OrbitCamera/PhysicalCamera/DoF state: primary click recalls,
secondary click copies, and an empty slot does nothing. The shared egui-viewport-toolbar
supplies actual Top geometry. Current controls cover 2D/PBR/PT, physical EV, spp, Settings
gear via OpenSettingsEvent/focus, primary SDR PNG through the existing capture handler and
context Export settings. Mandatory new persistence fields have no old-JSON fallback.
Actual app gate `1177` is compiling; new UI tests and GPU screenshots have not passed.
Full WarpBro OCIO/proxy/denoise/snapshot-format parity remains open. Keep all video artifacts
under `C:/Temp/bob`; real app-motion delivery remains distinct from technical codec fixtures.

The earlier CFR repair passed four existing video tests across six encoder variants, and
the production build passed in 5m57s. Those earlier receipts are historical. WarpBro's actual
oxide native CFR baseline `74654` passed one test (52m45s reported); updated canonical `8c4`
CFR/coded-metadata all-target gate `57365` passed in 2m55s with zero warnings. Full ordinary
oxide session `71257` is active, not passed.

Required continuation, in dependency order:

1. Complete the consumer build and verify adapter signal/layout/alpha against the shared API.
2. Finish frozen output color configuration and a genuine SDR still view independent of
   monitor mode. Verify PBR/sky/wireframe/background and source interpretation through the
   actual WarpBro reference; preserve existing renderer resources.
3. Verify source-to-file pixels, metadata, exact timing and cancellation for retained formats
   and existing CPU/Vulkan/native or external paths. Close genuine raw-scene EXR parity
   separately from display-light files.
4. Validate full UI/persistence and measure the complete capture/color/readback/PNG/encoder
   pipeline. Record queue bounds and cancellation responsiveness under load; do not infer
   performance from a build or an isolated shader/encoder time.

These are open correctness, performance and UI gates, not completed hidden source changes.

## Acceptance and evidence

- [ ] Typed float canvas readback retains extended-sRGB values above 1, correct alpha and
  dimensions. Canonical decode agrees with an oracle; repeated display/export transforms
  leave raw PT/OIDN source unchanged.
- [ ] Frozen output color/view/white settings produce the same file while monitor mode,
  white/peak, preview exposure and UI activity change. SDR still uses the SDR view, not a
  clipped HDR-monitor canvas. PBR/sky and transparent composition match the actual reference.
- [ ] SDR8/PQ16/HLG16 PNG readback verifies sample depth, transfer, primaries, alpha,
  cICP/mDCV and measured PQ cLLI. EXR checks verify genuine scene-linear AP1 source and tags.
- [ ] Actual supported codec/profile artifacts decode correctly. Verify pixels as well as
  `ffprobe` timing, frame count, depth/profile, range and color/metadata. Native CPU/Vulkan
  receipts are scoped separately from external FFmpeg. Keep `30000/1001` exact.
- [ ] Cancellation, source/readback/encode/mux errors and overwrite protection prevent false
  completion or destructive publication. Queues remain bounded and errors reach the UI.
- [ ] All existing display/export controls and complete settings survive strict save/open,
  without old-format aliases or default repair. The shared Display UI already provides
  SDR8/SDR10/PQ/HLG/scRGB, Auto/manual white and HLG peak, requested-versus-actual fallback,
  OS white/peak/full-frame/headroom/depth and HDR-off state; verify wiring, do not duplicate it.
- [x] Correct and rerun the narrow-toolbar measurement fixture: final ordinary suite passed
  57 tests, zero failed, three ignored. Historical narrow/100–150% visual
  defects remain recorded; no new screenshots are accepted. Skip extra screenshot/demo
  generation after relevant tests pass, as requested. Existing Computer Use native-pipe
  failure limits native evidence; GPU kittest/builds do not prove physical monitor luminance.
  Record hardware, actual modes, fixture hashes, commands and tolerances for executed gates.

## Standards and related work

[ITU-R BT.2100-3](https://www.itu.int/rec/R-REC-BT.2100) is the current PQ/HLG reference.
[ITU-T H.273](https://www.itu.int/rec/T-REC-H.273) identifies signal primaries/transfer/matrix.
[FFmpeg AVCodecContext](https://www.ffmpeg.org/doxygen/9.0/structAVCodecContext.html)
documents rational timing, formats, color fields and coded side data. Use them to check
actual signals; setting metadata alone does not establish parity.

The user's separate [nodes/profiles design](../../warpbro-rs/docs/render-profiles.md)
remains a design proposal, not implemented runtime behavior.
