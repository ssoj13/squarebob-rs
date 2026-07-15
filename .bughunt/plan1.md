# Bug Hunt Plan 1

Date: 2026-07-14
Repository: `squarebob-rs`
Branch: `main`
Mode: static audit and remediation planning only

## Decision gate

No source code was changed. No build or test was run during discovery.

This plan touches symbols with HIGH and CRITICAL GitNexus impact. Implementation must not start until the user approves this plan. Approval includes the listed consolidation/removal of duplicate private paths; no unrelated dead-code deletion is authorized.

## Audit coverage

- Scan/cache/UI orchestration: 14 files.
- GPU context, readback, picking, and PT presentation: 11 files.
- GPU BVH/PT shaders, renderer safety, and repository unsafe anchors.
- Media encode lifecycle, frame invariants, TIFF/TGA settings.
- Repository-wide TODO/FIXME, panic/unwrap, unsafe, arithmetic, threading, error-swallowing, and no-caller sweeps.
- GitNexus index: fresh at HEAD `5befc15`; 198 parsed files, 4,351 nodes, 13,237 edges, 300 execution flows.
- Secondary `gitnexus_rs` reader rejected storage v42 with reader v40. Compatible GitNexus MCP supplied context/impact results. Findings inside WGSL remain source-trace verified where the graph cannot model shader statements.

Agent evidence:

- `.bughunt/agent_app_scan_cache.md`
- `.bughunt/agent_gpu_readback.md`
- `.bughunt/agent_gpu_pt_safety.md`

## Executive summary

| ID | Severity | Problem class | GitNexus impact |
|---|---|---|---|
| F01 | CRITICAL | Megakernel variance ABI under-allocation | `create_variance_buffer`: CRITICAL |
| F02 | CRITICAL | Encode cancellation/lifecycle can hang and overlap exports | App output flow: HIGH; worker ownership crosses modules |
| F03 | HIGH | Treemap solid fill relies on unchecked unsafe parallel writes | `treemap::render`: CRITICAL |
| F04 | HIGH | Scan replacement orphans workers | `start_scan`: CRITICAL |
| F05 | HIGH | Cancellation/error/partial-success states are conflated | Scan owners: HIGH |
| F06 | HIGH | NTFS volume parsing can select unrelated local volume | NTFS owners: HIGH |
| F07 | HIGH | Scan presentation state and screenshot readiness are stale | `start_scan`: CRITICAL; `display_root`: HIGH |
| F08 | HIGH | Cache work blocks UI; writes are detached and non-atomic | `poll_scan`: HIGH |
| F09 | HIGH | Scanner failures become zero-byte cacheable success | Scanner owners: HIGH |
| F10 | HIGH | Frame objects accept invalid pixel layout | `Frame::new` upstream API: HIGH |
| F11 | HIGH | ReSTIR reuse estimator/visibility contract is biased | Shader path; PT entrypoints: CRITICAL |
| F12 | HIGH | Fixed BVH stacks silently drop subtrees | Shader path; PT/picking surfaces |
| F13 | HIGH | Russian roulette is biased, NaN-capable, and ignores UI switch | PT entrypoints: CRITICAL |
| F14 | HIGH | Readback and zero-copy PT implementations have drifted | Both entrypoints: CRITICAL |
| F15 | HIGH | Environment/display color contracts conflict | `EnvMap::load_from_file`: CRITICAL |
| F16 | MEDIUM | GPU/readback/count arithmetic is unchecked and duplicated | `map_readback`, `ensure_readback`: CRITICAL |
| F17 | MEDIUM | Readback failures lose identity; mapper is duplicated | `map_readback`: CRITICAL |
| F18 | MEDIUM | Path identity, progress totals, and cache envelope disagree | Owning scan flow: HIGH/CRITICAL |
| F19 | MEDIUM | TIFF compression and TGA RLE controls are ignored | Local writers: LOW |
| F20 | MEDIUM | Material domains, ray offsets, and RNG contracts are fragmented | PT entrypoints: CRITICAL |
| F21 | LOW | Readback staging bypasses allocation/accounting policy | Owning render APIs: CRITICAL |
| F22 | LOW | Two unsafe anchors lack adequate precondition rationale | Local |
| F23 | LOW | Duplicate `path_to_dir` policy | Local |

## Findings

### F01 — CRITICAL — Megakernel variance ABI under-allocation

Evidence:

- `crates/pt-megakernel/src/bvh_traverse.wgsl:127` defines a 32-byte `VarianceData`.
- `crates/pt-megakernel/src/bvh_traverse.wgsl:970` reads per-pixel `count`.
- `crates/pt-megakernel/src/bvh_traverse.wgsl:1492` updates the same element.
- `crates/pt-megakernel/src/compute.rs:3848` allocates 16 bytes per pixel.
- Correct host layout already exists at `crates/pt-megakernel/src/adaptive/pipeline.rs:11`; sizing uses `size_of::<VarianceData>()` at `crates/pt-megakernel/src/adaptive/pipeline.rs:84`.

Result: runtime array capacity covers only half the pixels. Upper-half indices read robust-access zeros and discard writes. Adaptive state becomes position-dependent.

Systemic fix:

- One host-owned variance ABI type.
- Checked `pixel_count * size_of::<VarianceData>()`.
- Explicit binding minimum size.
- One variance/count ownership model shared by megakernel and adaptive passes.
- Remove duplicate tracker only after feature/caller confirmation.

Acceptance:

- Rust/WGSL layout assertion covers size, alignment, and field offsets.
- Odd/even dimensions update every pixel.
- Adaptive convergence test proves all pixels reach the same stopping contract.

### F02 — CRITICAL — Encode cancellation/lifecycle can hang and overlap exports

Evidence:

- `src/app/image_sequence.rs:74` waits for UI-produced frames; exit depends on source-local cancellation.
- `src/app/image_sequence.rs:94` loops indefinitely on timeout.
- `src/app/image_sequence.rs:185` stops servicing requests as soon as dialog state becomes idle.
- Window mode propagates the encoding-to-idle transition at `src/app/image_sequence.rs:129` and `src/app/image_sequence.rs:146`.
- Inline mode polls and renders without the transition cancellation at `src/app/settings/output.rs:43` and `src/app/settings/output.rs:63`.
- Stop detaches the handle, resets to idle, and replaces the cancel token at `crates/media-encoder/src/dialogs/encode/encode_ui.rs:883`–`899`.
- Finished “orphans” are dropped, not joined, at `crates/media-encoder/src/dialogs/encode/encode_ui.rs:903`–`913`.
- Reset may block the UI in `join` at `crates/media-encoder/src/dialogs/encode/encode_ui.rs:920`–`934`.
- Drop joins workers without first cancelling the source at `crates/media-encoder/src/dialogs/encode/encode_ui.rs:1300`–`1313`.

Result: Stop can strand a worker inside `get_frame`; restart can run old/new exports concurrently; shutdown can hang.

Systemic fix:

- One generation-owned `EncodeSession`: `Idle | Running | Cancelling | Finishing`.
- One cancellation authority shared by dialog worker and frame source.
- Poll lifecycle once per app frame, independent of window/section visibility.
- Stop remains non-blocking but new Encode stays disabled until the old handle is joined.
- Generation IDs reject stale progress/frame requests.
- Shutdown cancels pending frame request and worker before join.
- Remove orphan-handle detachment.

Acceptance:

- Stop during pending frame, render, encode, and finalization terminates.
- Collapse/close both output UIs while encoding; lifecycle still drains.
- Immediate restart cannot create two writers for one output.
- App shutdown during pending frame finishes within bounded time.
- Worker panic is joined and surfaced.

### F03 — HIGH — Treemap solid fill is not memory-safe by construction

Evidence:

- Buffer allocation: `crates/treemap/src/lib.rs:461`–`464`.
- Non-overlap is only a `debug_assert!`: `crates/treemap/src/lib.rs:498`–`504`.
- Rayon closures mutate through a pointer derived from `as_ptr()`: `crates/treemap/src/lib.rs:525`–`545`.
- Safe row ownership already exists at `crates/treemap/src/lib.rs:512`–`520`.

Result: release builds do not enforce bounds/disjointness at the unsafe boundary. Layout regression becomes data race or out-of-bounds UB.

Systemic fix: use safe disjoint row partitions for both cushion and solid modes. Keep layout validation as correctness diagnostics, not memory-safety foundation.

Acceptance:

- No unsafe block in solid fill.
- Property tests cover random layouts, no overlap, bounds, and deterministic CPU output.
- Rayon and serial reference outputs match.

### F04 — HIGH — Starting another scan orphans active worker

Evidence:

- Scan entrypoints remain active at `src/app/toolbar.rs:28`, `src/app/toolbar.rs:38`, `src/app/toolbar.rs:47`, and `src/app/toolbar.rs:63`.
- `start_scan` replaces receiver/token at `src/app/scan_orchestration.rs:86` and `src/app/scan_orchestration.rs:120`.
- Standard/NTFS workers ignore disconnected sends at `src/scanner.rs:157` and `src/scanner_ntfs.rs:128`.
- Thread spawn panics at `src/scanner.rs:53` and `src/scanner_ntfs.rs:146`.

Systemic fix: one `ScanSession` owns generation, normalized root, cancel token, receiver, and worker handle. Replacement cancels the active generation, rejects stale messages, and reaps finished handles without blocking UI. Spawn failure is recoverable.

### F05 — HIGH — Scan terminal outcomes are conflated

Evidence:

- NTFS cancellation falls into fallback at `src/scanner_ntfs.rs:121`–`134`.
- Tree building can break and return partial state at `src/scanner_ntfs.rs:864`.
- Size pass cannot propagate cancellation at `src/scanner_ntfs.rs:896`–`929`.
- Partial tree still reaches `Done` at `src/scanner_ntfs.rs:921`–`929`.

Systemic fix: typed `Completed | Cancelled | Partial(ScanDiagnostics) | Failed`. Fallback only for classified backend capability/runtime failures. Cancelled results never install or cache.

### F06 — HIGH — NTFS volume parsing can select unrelated local volume

Evidence: first-letter extraction is duplicated at `src/scanner_ntfs.rs:29`, `src/scanner_ntfs.rs:77`, `src/scanner_ntfs.rs:323`, `src/scanner_ntfs.rs:466`, `src/scanner_ntfs.rs:643`, and `src/scanner_ntfs.rs:773`.

Result: relative `folder` can become `F:`; UNC `\\server\share` can become `S:`.

Systemic fix: one Windows `VolumeTarget` parser over a normalized absolute `ScanRoot`. Accept only supported disk/verbatim-disk forms. Route UNC, unsupported prefixes, mount aliases, and resolution failure to the standard scanner with an explicit reason.

### F07 — HIGH — Presentation state and screenshot readiness are stale across roots

Evidence:

- Invalid path returns without clearing state at `src/app/scan_orchestration.rs:47`.
- Cache miss clears only three fields at `src/app/scan_orchestration.rs:80`–`84`.
- Derived display state remains preferred at `src/app/mod.rs:362`–`364`.
- Cached preview starts screenshot timing at `src/app/scan_orchestration.rs:76`.
- Live completion cannot restart an already-set timer at `src/app/scan_orchestration.rs:189`.
- Screenshot completion/exit consumes this readiness at `src/app/screenshot.rs:21`–`57`.

Systemic fix: atomic generation/root-keyed presentation transition. Cached tree is explicitly `Preview`; live `Completed` alone arms automation unless CLI policy explicitly requests cached capture.

### F08 — HIGH — Cache pipeline blocks UI and races persistence

Evidence:

- Cache load/deserialization and tree derivation run on UI at `src/app/scan_orchestration.rs:60`–`73` and `src/cache.rs:103`–`126`.
- Full serialization runs on UI at `src/app/scan_orchestration.rs:167`–`180`.
- Writers are detached at `src/app/scan_orchestration.rs:171`.
- Final file is truncated in place at `src/cache.rs:89`.
- Corrupt-cache removal failure is ignored at `src/cache.rs:131`.

Systemic fix: single cache service, normalized key, ordered generations, bounded decode, same-directory temp write, flush, atomic replace, and ordered clear tombstone. Background worker owns `DirEntry` only before channel handoff; final tree ownership stays on UI side.

### F09 — HIGH — Scanner failures become zero-byte cacheable success

Evidence:

- Standard walk errors continue at `src/scanner.rs:94` and `src/scanner.rs:137`.
- Metadata failure becomes size zero at `src/scanner.rs:124`–`125`.
- NTFS malformed/depth/metadata paths lose completeness at `src/scanner_ntfs.rs:287`, `src/scanner_ntfs.rs:429`, `src/scanner_ntfs.rs:857`, and `src/scanner_ntfs.rs:904`.
- Both backends can still produce success at `src/scanner.rs:225` and `src/scanner_ntfs.rs:915`.

Systemic fix: shared `ScanDiagnostics` and completeness contract. Root failure is terminal. Child failures are classified and counted. Unknown metadata is never fabricated as zero. Cache envelope records completeness; UI labels partial data.

### F10 — HIGH — Frame objects accept invalid pixel layout

Evidence:

- Infallible unchecked constructors: `crates/media-encoder/src/frame.rs:45`–`69`.
- Unchecked allocation/capacity arithmetic: `crates/media-encoder/src/frame.rs:98`, `crates/media-encoder/src/frame.rs:105`, `crates/media-encoder/src/frame.rs:112`, `crates/media-encoder/src/frame.rs:130`, `crates/media-encoder/src/frame.rs:156`, `crates/media-encoder/src/frame.rs:164`.
- Unchecked row indexing/slicing: `crates/media-encoder/src/frame.rs:238`–`242`.
- App repeats unchecked expected-length arithmetic at `src/app/image_sequence.rs:224`.

Systemic fix: fallible frame constructors backed by one checked `PixelLayout`: dimensions, channel count, element type, exact length, row stride, and checked conversions. Crop/conversion/encoders consume that invariant instead of reconstructing arithmetic.

### F11 — HIGH — ReSTIR estimator and visibility are biased

Evidence:

- Radiance-magnitude target approximation: `crates/pt-megakernel/src/restir/spatial.wgsl:149`–`160`, `crates/pt-megakernel/src/restir/temporal.wgsl:142`–`153`.
- Spatial compatibility omits material/instance: `crates/pt-megakernel/src/restir/spatial.wgsl:58`–`72`.
- Initial samples are shadow-tested at `crates/pt-megakernel/src/restir/initial.wgsl:501`–`510`.
- Relocated reused samples are accumulated without final visibility at `crates/pt-megakernel/src/restir/shade.wgsl:219`–`260`.

Systemic fix: shared receiver-target evaluation with `Li * BSDF * cosine`, proposal/Jacobian terms, material/instance compatibility, and final-receiver visibility.

### F12 — HIGH — Fixed BVH stacks silently discard subtrees

Evidence:

- Main/shadow overflow drops children: `crates/pt-megakernel/src/bvh_traverse.wgsl:467`, `crates/pt-megakernel/src/bvh_traverse.wgsl:505`.
- Picking repeats a 32-entry stack: `crates/pt-megakernel/src/pick.wgsl:57`, `crates/pt-megakernel/src/pick.wgsl:149`.
- ReSTIR visibility repeats it: `crates/pt-megakernel/src/restir/initial.wgsl:147`, `crates/pt-megakernel/src/restir/initial.wgsl:215`.

Systemic fix: stackless/rope traversal, or builder-enforced maximum depth with validation and recoverable fallback. One traversal contract for hit, shadow, pick, and ReSTIR.

### F13 — HIGH — Russian roulette is biased, NaN-capable, and disconnected from settings

Evidence:

- Unclamped continuation probability: `crates/pt-megakernel/src/bvh_traverse.wgsl:1091`.
- Reciprocal weighting can divide by zero or by values above one: `crates/pt-megakernel/src/bvh_traverse.wgsl:1095`.
- Host setting controls only wavefront RR at `crates/pt-megakernel/src/compute.rs:1654`.
- Both render paths call that setter at `crates/render-3d/src/pt/megakernel/render.rs:379` and `crates/render-3d/src/pt/megakernel/render_no_readback.rs:331`.
- Megakernel applies RR unconditionally at `crates/pt-megakernel/src/bvh_traverse.wgsl:1089`.

Systemic fix: one integrator-settings contract for both backends; explicit enable/start bounce; terminate zero/non-finite throughput; clamp continuation probability to a documented interval before reciprocal weighting.

### F14 — HIGH — PT entrypoints have drifted into two renderers

Evidence:

- Zero-copy sets tile size twice at `crates/render-3d/src/pt/megakernel/render_no_readback.rs:330` and `crates/render-3d/src/pt/megakernel/render_no_readback.rs:337`.
- Readback path omits it around `crates/render-3d/src/pt/megakernel/render.rs:378`.
- Readback path adapts SPP at `crates/render-3d/src/pt/megakernel/render.rs:405`–`420`.
- Primary zero-copy path only reads stored SPP at `crates/render-3d/src/pt/megakernel/render_no_readback.rs:358`–`362`.

Systemic fix: one PT frame state machine for initialization, scene/camera/options, adaptive decision, dispatch, timing, blit, and submit. Output policy controls only optional staging copy/map. Public wrappers delegate to the shared path.

### F15 — HIGH — Environment/display color contracts conflict

Evidence:

- LDR environment values feed luminance as linear at `crates/render-3d/src/env_map.rs:143`–`150`.
- Texture uses `Rgba8Unorm` at `crates/render-3d/src/env_map.rs:152`.
- Encoded-domain luminance builds CDF at `crates/render-3d/src/env_map.rs:196`.
- PT manually applies display encoding at `crates/pt-megakernel/src/blit.wgsl:321`–`351`.
- Render target is `Rgba8Unorm` at `crates/render-3d/src/targets.rs:44`–`54`.
- HDR PQ is destroyed by 8-bit SDR output at `crates/pt-megakernel/src/blit.wgsl:329`–`334`.
- OCIO clamps HDR at `crates/pt-megakernel/src/blit.wgsl:301`–`315`.
- SDR uses `1/2.2` instead of sRGB at `crates/pt-megakernel/src/blit.wgsl:350`.

Systemic fix: explicit scene-linear input/intermediate contract, exactly one display-transfer stage, linearized LDR environment importance, real HDR surface negotiation, and shaper-aware OCIO handling.

### F16 — MEDIUM — Size/count arithmetic is unchecked and duplicated

Verified sites:

- `crates/render-core/src/lib.rs:263`–`268`, `crates/render-core/src/lib.rs:308`–`317`
- `crates/render-3d/src/lib.rs:1364`
- `crates/render-3d/src/picking.rs:100`, `crates/render-3d/src/picking.rs:148`
- `crates/pt-megakernel/src/compute.rs:1218`, `crates/pt-megakernel/src/compute.rs:2811`, `crates/pt-megakernel/src/compute.rs:3690`, `crates/pt-megakernel/src/compute.rs:3825`–`3859`, `crates/pt-megakernel/src/compute.rs:3914`, `crates/pt-megakernel/src/compute.rs:4065`
- `crates/pt-megakernel/src/adaptive/pipeline.rs:83`, `crates/pt-megakernel/src/restir/pipeline.rs:232`
- `crates/pt-wavefront/src/wavefront/pipeline.rs:249`, `crates/pt-wavefront/src/wavefront/pipeline.rs:471`
- `crates/bvh-gpu/src/bvh_gpu/mod.rs:628`, `crates/bvh-gpu/src/bvh_gpu/mod.rs:668`, `crates/bvh-gpu/src/bvh_gpu/mod.rs:683`, `crates/bvh-gpu/src/bvh_gpu/mod.rs:689`, `crates/bvh-gpu/src/bvh_gpu/mod.rs:823`, `crates/bvh-gpu/src/bvh_gpu/mod.rs:871`

Systemic fix: checked typed helpers for pixel count, block geometry, row pitch/alignment, byte size, slice offsets, and `usize/u32/u64` conversion. Reuse across RGBA8, Rgba32Float, R32Uint, PT allocations, wavefront, and BVH.

### F17 — MEDIUM — Readback failure identity is lost

Evidence:

- Poll result discarded at `crates/render-core/src/lib.rs:374`.
- Mapper collapses failure to empty pixels at `crates/render-core/src/lib.rs:323`.
- CPU color path duplicates map/poll/receive/unmap at `crates/pt-megakernel/src/compute.rs:4972`–`4984`.
- Screenshot reports generic dimensions after empty result at `src/app/screenshot.rs:138`.

Systemic fix: one `ReadbackError` covering poll, map callback, layout overflow, undersized mapping, missing target, and zero size. Readback returns `Result`; renderer/app chooses retry, skip, or precise user error.

### F18 — MEDIUM — Path identity, progress totals, and cache envelope disagree

Evidence:

- Raw-string path hash: `src/path_key.rs:6`, `src/cache.rs:33`, `src/exclusions.rs:62`.
- Raw-string history dedupe: `src/app/scan_orchestration.rs:54`–`56`.
- NTFS root is included in directory count at `src/scanner_ntfs.rs:830`–`833`.
- NTFS progress fabricates zero bytes/errors at `src/scanner_ntfs.rs:915`–`919`.
- Standard aggregation uses a different contract at `src/scanner.rs:202`.

Systemic fix: central `ScanRoot` value with display path, normalized operational path, and stable OS-aware identity. Cache/exclusions/history/session/envelope consume the same key. One post-order aggregate definition shared by scanners.

### F19 — MEDIUM — TIFF/TGA settings are exposed but ignored

Evidence:

- TIFF compression UI: `crates/media-encoder/src/dialogs/encode/encode_ui.rs:1230`–`1252`.
- TGA RLE UI: `crates/media-encoder/src/dialogs/encode/encode_ui.rs:1254`–`1262`.
- TIFF writer discards compression at `crates/media-encoder/src/dialogs/encode/encode.rs:2464`.
- TGA writer ignores settings at `crates/media-encoder/src/dialogs/encode/encode.rs:2472` and `crates/media-encoder/src/dialogs/encode/encode.rs:2509`.

Systemic fix: explicit encoder APIs. Map TIFF modes through the `tiff` encoder. Use `image::codecs::tga::TgaEncoder`; call `disable_rle` when requested. Keep UI/settings; make output honor them.

### F20 — MEDIUM — Shader domains, ray offsets, and RNG are fragmented

Evidence:

- Unvalidated IOR: `crates/standard-surface/src/params.rs:91`–`96`, consumed at `crates/pt-megakernel/src/bvh_traverse.wgsl:1101`, `crates/pt-megakernel/src/bvh_traverse.wgsl:1423`, `crates/pt-megakernel/src/restir/shade.wgsl:244`.
- Fixed offsets: `crates/pt-megakernel/src/bvh_traverse.wgsl:1126`, `crates/pt-megakernel/src/bvh_traverse.wgsl:1381`, `crates/pt-megakernel/src/bvh_traverse.wgsl:1439`, `crates/pt-megakernel/src/restir/initial.wgsl:502`.
- RNG seeds diverge at `crates/pt-megakernel/src/bvh_traverse.wgsl:974`, `crates/pt-megakernel/src/restir/initial.wgsl:460`, `crates/pt-megakernel/src/restir/temporal.wgsl:99`, `crates/pt-megakernel/src/restir/spatial.wgsl:99`.
- Several RNGs can return 1.0 at `crates/pt-megakernel/src/restir/initial.wgsl:131`, `crates/pt-megakernel/src/restir/temporal.wgsl:69`, `crates/pt-megakernel/src/restir/spatial.wgsl:55`, `crates/pt-megakernel/src/pathguide/sample.wgsl:31`.

Systemic fix: validate material domains once plus defensive shader guards; one scale/ULP-aware ray-offset helper; one shared WGSL RNG keyed by pixel/frame/sample/bounce/dimension with guaranteed `[0,1)`.

### F21 — LOW — Readback staging bypasses allocation policy

Evidence: direct staging allocation at `crates/render-core/src/lib.rs:266` and `crates/pt-megakernel/src/compute.rs:4940`; central allocator exists at `crates/render-core/src/lib.rs:211`.

Fix: checked readback layout owns allocation through `make_buffer`; recurrent paths reuse grow-on-demand staging capacity; one-shot screenshots remain explicit.

### F22 — LOW — Unsafe rationale gaps

- Missing rationale: `crates/xtask/src/env_setup.rs:64`; sister invariant exists at `crates/xtask/src/env_setup.rs:57`.
- Inadequate precondition rationale: `crates/render-core/src/lib.rs:152`–`158`.
- No unsafe blocks remain in scoped `render-3d`, `bvh-gpu`, or `pt-megakernel`.
- Treemap unsafe is F03 and must be removed, not documented.

### F23 — LOW — Duplicate path-to-directory policy

Identical private helpers exist at `src/app/helpers.rs:209`–`215` and `src/app/shell.rs:246`–`252`.

Fix: one canonical helper with explicit behavior for nonexistent paths. Remove duplicate only under this plan approval.

## Implementation sequence

### Phase 0 — Memory and ABI safety

- [ ] F01: unify variance ABI and checked allocation.
- [ ] F03: replace raw parallel writes with safe row ownership.
- [ ] Add ABI/property tests before broader renderer work.

### Phase 1 — Checked layout and recoverable GPU/media errors

- [ ] Build shared checked byte/layout primitives.
- [ ] F16/F17/F21: migrate readback, picking, PT color, wavefront, and BVH counts.
- [ ] F10: make `Frame` constructors/crop/conversions enforce exact layout.
- [ ] Preserve recoverable fallback behavior; no panic/empty-vector sentinel.

### Phase 2 — Scan, root identity, and cache transaction model

- [ ] Introduce `ScanRoot`, `ScanSession`, generation IDs, and typed outcomes.
- [ ] F04/F05/F06/F07/F09/F18: migrate every scanner and UI entrypoint.
- [ ] F08: ordered background cache service with atomic persistence.
- [ ] Preserve owned `DirEntry` handoff; no `Arc<DirEntry>`.

### Phase 3 — Encode session and format truthfulness

- [ ] Introduce generation-owned `EncodeSession`.
- [ ] Move polling/cancellation out of view visibility.
- [ ] Remove orphan detachment; join completed handles and surface panics.
- [ ] F19: honor TIFF/TGA settings through explicit encoders.
- [ ] Route all format writers through validated frame layout.

### Phase 4 — Path-tracing estimator and traversal correctness

- [ ] F12: one bounded-correct traversal contract.
- [ ] F13: shared integrator settings and valid RR math.
- [ ] F11: correct ReSTIR target, compatibility, and final visibility.
- [ ] F20: material/ray/RNG contracts.
- [ ] Add deterministic CPU/reference and shader integration tests.

### Phase 5 — One PT frame state machine

- [ ] F14: extract shared preparation/dispatch/timing/blit path.
- [ ] Keep zero-copy and readback wrappers as output policies.
- [ ] Verify option parity across both wrappers.

### Phase 6 — End-to-end color contract

- [ ] F15: define scene-linear, display-linear, and encoded boundaries.
- [ ] Correct environment decode/CDF.
- [ ] Ensure one transfer function.
- [ ] Gate HDR modes on real HDR output capability; preserve OCIO dynamic range.

### Phase 7 — Small consolidation

- [ ] F22: add precise safety preconditions.
- [ ] F23: consolidate `path_to_dir`.
- [ ] Run no-caller/TODO cross-check again before any deletion.

## Verification matrix

Run after each implementation chunk, not before design:

- `cargo fmt --all -- --check`
- Targeted crate tests for touched crates.
- Workspace `cargo check --workspace --all-targets`.
- Workspace `cargo test --workspace`.
- Windows NTFS tests: drive, verbatim drive, UNC, relative, cancellation in every phase, backend fallback, stale generations.
- Cache tests: same-root generation ordering, clear/write race, truncated input, wrong root, partial result, atomic replace failure.
- Encode tests: stop/restart/shutdown at every lifecycle state; hidden/collapsed UIs; worker panic; output collision.
- Frame/readback property tests: zero, max, overflow, odd row pitch, mapped buffer short by one byte, exact-length validation.
- PT tests: variance coverage, RR energy/reference, deep BVH, pick/shadow parity, ReSTIR occlusion/material boundary, RNG range/determinism.
- Color tests: linear ramps, sRGB reference values, HDR highlight preservation, environment CDF reference.
- CPU treemap serial/parallel equivalence under randomized layouts.
- Final GitNexus reanalysis and `detect_changes(scope: "all")`; inspect every affected process.

## Explicit non-actions

- No `AGENTS.md` or `DIAGRAMS.md` changes.
- No feature-toggle or platform-parity stub deletion.
- No dead-code deletion. Cypher no-caller results contained trait/UI/cfg false positives.
- No new GPU device/context. `render_core::gpu::GpuContext::new` remains device setup source of truth.
- No `Arc<DirEntry>`; main UI retains final scan-tree ownership.
