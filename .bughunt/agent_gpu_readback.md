# Agent GPU Readback — GPU context, readback, and PT path audit
Files scanned: 11

## Findings

### [HIGH] Readback and zero-copy PT loops diverged into different renderers
- Loc: crates/render-3d/src/renderer3d/render.rs:203
- Loc: crates/render-3d/src/pt/megakernel/render.rs:413
- Loc: crates/render-3d/src/pt/megakernel/render.rs:485
- Loc: crates/render-3d/src/pt/megakernel/render_no_readback.rs:330
- Loc: crates/render-3d/src/pt/megakernel/render_no_readback.rs:358
- Loc: crates/pt-megakernel/src/compute.rs:1634
- Cat: 8, 12
- Issue: Main eframe path calls 404-line zero-copy implementation. Auto-SPP reads `pt_samples_per_update`, initialized to 1, but never updates it or `pt_last_render_ms`. Legacy readback implementation alone runs feedback controller and timing update. Reverse drift: zero-copy applies `pt_wavefront_tile_size` twice; legacy readback never applies it.
- Why bad: Same frame contract has two state machines. User-visible behavior depends on output transport. Auto-SPP stays at one sample/update on primary zero-copy path. Wavefront tile control ignored on fallback/readback path. Future PT features must land twice. Existing drift proves maintenance failure.
- Impact: GitNexus downstream: `render_path_traced` CRITICAL, 62 symbols / 25 processes / 6 modules; `render_path_traced_no_readback` CRITICAL, 55 symbols / 25 processes / 5 modules. No edit permitted without approval.
- Fix: One PT frame function. Shared init, scene update, camera/history update, feature configuration, adaptive-SPP decision, dispatch, timing, blit, submit. Output policy only controls optional staging-copy encoding and post-submit mapping. Remove duplicate `set_wavefront_tile_size`. Keep zero-copy and CPU-readback public wrappers, both delegating to same frame state machine.

### [MED] Readback layout arithmetic can wrap before widening
- Loc: crates/render-core/src/lib.rs:263
- Loc: crates/render-core/src/lib.rs:268
- Loc: crates/render-core/src/lib.rs:308
- Loc: crates/render-core/src/lib.rs:311
- Loc: crates/render-core/src/lib.rs:316
- Loc: crates/pt-megakernel/src/compute.rs:4935
- Loc: crates/pt-megakernel/src/compute.rs:4939
- Loc: crates/pt-megakernel/src/compute.rs:4994
- Loc: crates/pt-megakernel/src/compute.rs:5001
- Loc: crates/render-3d/src/picking.rs:100
- Loc: crates/render-3d/src/picking.rs:148
- Cat: 5, 8, 12
- Issue: Width, aligned row stride, row offset, pixel count, and buffer size multiply as `u32`; casts happen after arithmetic. RGBA8 helper, Rgba32Float CPU color path, and R32Uint picking repeat same unchecked formula.
- Why bad: Overflow can create undersized staging buffers, invalid copy layouts, wrong capacities, or slice-range panic. Map-failure recovery cannot handle validation/slice panics. Public helper accepts independent texture and dimensions; current callers matching hardware limits does not establish arithmetic safety.
- Impact: GitNexus downstream: `map_readback` CRITICAL, 6 symbols / 11 processes / 3 modules; `ensure_readback` CRITICAL, 3 symbols / 12 processes / 2 modules; `apply_cpu_color_in_place` LOW, 2 symbols / 2 processes. Sister sites inspected: four RGBA8 callers, CPU Rgba32Float path, picking row path.
- Fix: Central checked `TextureReadbackLayout` parameterized by bytes-per-pixel/block geometry. Compute unpadded stride, 256-byte aligned stride, buffer length, row offsets, and output length with `checked_mul`, `checked_add`, and `usize::try_from`. Return typed layout error. Validate mapped length before slicing. Reuse layout in copy and unpack phases for RGBA8, Rgba32Float, and R32Uint.

### [MED] Recoverable readback failures lose error identity; CPU color bypasses central mapper
- Loc: crates/render-core/src/lib.rs:374
- Loc: crates/render-core/src/lib.rs:375
- Loc: crates/render-core/src/lib.rs:323
- Loc: crates/pt-megakernel/src/compute.rs:4972
- Loc: crates/pt-megakernel/src/compute.rs:4980
- Loc: crates/pt-megakernel/src/compute.rs:4984
- Loc: crates/render-3d/src/lib.rs:1136
- Loc: src/app/screenshot.rs:138
- Cat: 8, 9, 14
- Issue: `map_buffer_read` discards `Device::poll` result. `map_readback` logs then returns empty pixels. Screenshot API also uses empty pixels for missing state, zero size, and map failure; final user error becomes generic “Invalid image dimensions.” CPU OCIO path reimplements `map_async + poll + recv + map + unmap` and logs/returns `()` on failure despite central helper.
- Why bad: Failure modes become indistinguishable. UI cannot report retryable device loss versus invalid state/layout. CPU OCIO failure silently leaves intermediate color output. Duplicate mapper can drift from central recovery policy. wgpu 29 `Device::poll` returns `Result<PollStatus, PollError>`; current result is intentionally discarded.
- Impact: GitNexus downstream: `readback_render_texture` CRITICAL, 14 symbols / 13 processes / 4 modules; `map_readback` CRITICAL; `map_buffer_read` LOW. No interface edit permitted without approval.
- Fix: Extend one `ReadbackError` with poll, callback/map, layout overflow, mapped-buffer-too-small, missing target, and zero-size variants. Make readback APIs return `Result<Vec<u8>, ReadbackError>`. Route CPU OCIO through `map_buffer_read`; propagate failure to renderer/app. UI decides retry, skip, or show precise screenshot/color error. Never panic.

### [LOW] Readback staging buffers bypass allocation source of truth
- Loc: crates/render-core/src/lib.rs:211
- Loc: crates/render-core/src/lib.rs:266
- Loc: crates/pt-megakernel/src/compute.rs:4940
- Loc: crates/treemap/src/wgpu.rs:680
- Loc: crates/render-3d/src/lib.rs:1449
- Loc: crates/render-3d/src/pt/megakernel/render.rs:472
- Cat: 4, 8, 11
- Issue: Shared buffer policy mandates `make_buffer`, including readback staging. RGBA8 helper and Rgba32Float CPU path call `device.create_buffer` directly. Per-frame fallback paths allocate new staging buffers and skip `gpu_mem::note_alloc`.
- Why bad: VRAM accounting excludes readback pressure. Legacy fallback and CPU color frames create repeated large transient allocations. Central allocation policy no longer describes actual GPU memory use.
- Impact: GitNexus downstream: `readback_texture` LOW; owning legacy callers include 2D, raster 3D, PT, and screenshots. Their public render/readback interfaces have CRITICAL downstream blast radius.
- Fix: Make checked readback layout own staging allocation through `make_buffer`. Add renderer-owned reusable staging capacity where calls are frame-recurrent; grow on demand, unmap before reuse. Keep one-shot screenshot path explicit. Preserve CPU fallback behavior.

## Dead code
- None confirmed in scope.

## Dedup
- `crates/render-3d/src/pt/megakernel/render.rs:5` and `crates/render-3d/src/pt/megakernel/render_no_readback.rs:5`: duplicate PT frame state machines; already behaviorally divergent.
- `crates/render-core/src/lib.rs:263`, `crates/pt-megakernel/src/compute.rs:4935`, and `crates/render-3d/src/picking.rs:100`: duplicate row-layout arithmetic.
- `crates/render-core/src/lib.rs:361` and `crates/pt-megakernel/src/compute.rs:4972`: duplicate synchronous map workflow.

## Notes
- Current readback textures match 4-byte contract: `crates/treemap/src/wgpu.rs:361` and `crates/render-3d/src/targets.rs:50` use `Rgba8Unorm`. PT CPU color reads its separate `Rgba32Float` output with 16 BPP at `crates/pt-megakernel/src/compute.rs:4934`. No current format mismatch found.
- Device ownership consistent in inspected paths. 2D, 3D, and PT use renderer-owned shared `GpuContext`; no second device/queue creation found.
- Texture/view and staging lifetimes valid in inspected flows. Submission precedes mapping; successful mapper unmaps before local buffer drop. No leak found.
- No tests/builds run. Audit-only instruction honored.
- GitNexus graph fresh at HEAD. `gitnexus-rs` reader rejected database version 42 versus 40; compatible GitNexus MCP used for query/context/impact.
