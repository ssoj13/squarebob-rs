# I.5 — wgpu::Buffer ↔ Burn Tensor bridge: API survey

**Outcome:** Path 1 (zero-copy wrap of external `wgpu::Buffer` as a Burn tensor) is **NOT feasible** with the burn 0.21 / cubecl 0.10 public API. Path 2 (device-local `copy_buffer_to_buffer` against Burn-allocated tensors) is **fully feasible with zero crate patches**. Total implementation effort: ~3-6 h.

All citations below are absolute paths inside the local cargo registry cache (`C:/Programs/Ntutil/apps/prog/lang/Rust/cargo/registry/src/index.crates.io-1949cf8c6b5b557f/`).

---

## Path 1 — zero-copy wrap. Verdict: **blocked**.

To wrap an existing `wgpu::Buffer` (held by squarebob) as a Burn `Tensor`, we need to walk this type chain backwards:

```
Tensor<B, 4>
  └─ TensorPrimitive<B>            // burn-tensor: enum Float/QFloat/etc.
       └─ CubeTensor<R>            // burn-cubecl: backing primitive for Wgpu backend
            ├─ ComputeClient<R>    // already obtainable
            ├─ Metadata            // we can construct
            └─ Handle              // cubecl-runtime::server::Handle
                 ├─ ManagedMemoryHandle  // cubecl-runtime
                 ├─ StreamId             // public
                 └─ size                 // public
```

`CubeTensor::new_contiguous(client, device, shape, handle, dtype)` is public (`burn-cubecl-0.21.0/src/tensor/base.rs:150`). `Handle::from_memory(managed_memory_handle, stream_id, size)` is public (`cubecl-runtime-0.10.0/src/server/handle.rs:49`). So far so good — until we need a `ManagedMemoryHandle` that points to an externally-owned `wgpu::Buffer`.

### Where the chain breaks

`ManagedMemoryHandle` (`cubecl-runtime-0.10.0/src/memory_management/memory_pool/handle.rs:7-11`) holds:

```rust
pub struct ManagedMemoryHandle {
    descriptor: Arc<ManagedMemoryDescriptor>,
    handle_count: Arc<()>,
}
```

The `descriptor` is `pub(crate) struct ManagedMemoryDescriptor` (line 39). Its `MemoryLocation` is `pub(crate)` (line 76), constructed via `MemoryLocation::new(...)` which is `pub(crate)` (line 125). The only public way to build a `ManagedMemoryHandle` is `ManagedMemoryHandle::new()` (line 147), which produces a handle with `MemoryLocation::uninit()` — pointing at *nothing*. The descriptor's `update_location` is `pub(crate)` (line 89), so we can't even set it from outside.

And the location itself isn't a pointer — it's an index `(pool: u8, page: u16, slice: u32)` into cubecl's memory-management bookkeeping. Even if we could fabricate a location, there has to be a matching entry in:

1. **`WgpuStorage::memory: HashMap<StorageId, wgpu::Buffer>`** (`cubecl-wgpu-0.10.0/src/compute/storage.rs:14`) — private field. The only insertion path is `WgpuStorage::alloc(size)` (line 93), which allocates a fresh buffer via `device.create_buffer(...)` — no entry point for adopting an external `wgpu::Buffer`.

2. **`MemoryManagement`'s pool/page/slice tracking tables** in `cubecl-runtime-0.10.0/src/memory_management/memory_pool/*.rs` — all `pub(crate)`.

### What it would take

Adding zero-copy support would require coordinated edits to **both** crates:

- `cubecl-wgpu`: `WgpuStorage::register_external(&mut self, buf: wgpu::Buffer, size: u64) -> StorageId` plus a route for `alloc` callers to opt-in.
- `cubecl-runtime`: a `MemoryManagement::reserve_external(storage_id, size) -> ManagedMemoryHandle` API that bypasses pool/page bookkeeping (or registers the external entry in a dedicated "external" pool slot), plus making `ManagedMemoryDescriptor` and `MemoryLocation` constructable with the right semantics.
- The two crates' fork checkouts have to stay in sync with our copy of burn 0.21.

Estimated effort: **1-3 days** of fork work, plus risk that future cubecl bugs become very hard to merge upstream-style.

**Recommendation: don't do path 1.** Path 2 captures most of the same wall-clock win at a fraction of the engineering risk.

---

## Path 2 — device-local copy. Verdict: **feasible, public-API only**.

The win we actually want: stop bouncing pixel bytes through host RAM. We can keep that property *without* zero-copy by issuing a `wgpu::CommandEncoder::copy_buffer_to_buffer` from squarebob's encoder into a Burn-allocated buffer on the same `Arc<wgpu::Device>`. Bytes never leave VRAM. The host roundtrip via `map_async` + `to_rgb_f32` goes away.

### The bridge — fully public

`burn-cubecl-0.21.0/src/tensor/base.rs:19-32`:

```rust
pub struct CubeTensor<R: CubeRuntime> {
    pub client: ComputeClient<R>,
    pub handle: Handle,
    pub meta: Box<Metadata>,
    pub device: R::Device,
    pub dtype: DType,
    pub qparams: Option<QParams>,
}
```

**All fields are `pub`.** Including `handle` and `client`.

`cubecl-runtime-0.10.0/src/client.rs:197-210`:

```rust
pub fn get_resource(
    &self,
    handle: Handle,
) -> Result<
    ManagedResource<<<R::Server as ComputeServer>::Storage as ComputeStorage>::Resource>,
    ServerError,
> {
    let stream_id = self.stream_id();
    let binding = handle.binding();
    self.device
        .submit_blocking(move |state| state.get_resource(binding, stream_id))
        .unwrap()
}
```

`ManagedResource::resource()` is public (`cubecl-runtime-0.10.0/src/storage/base.rs:117`). For `WgpuRuntime`, the underlying `Resource` is `WgpuResource` (`cubecl-wgpu-0.10.0/src/compute/storage.rs:28-39`):

```rust
pub struct WgpuResource {
    pub buffer: wgpu::Buffer,
    pub offset: u64,
    pub size: u64,
}
```

**`pub buffer: wgpu::Buffer`.** That's the wire.

### The end-to-end flow

```rust
// In squarebob, given a Burn Tensor allocated empty:
let burn_tensor: Tensor<Wgpu, 4> = Tensor::zeros([1, 3, h, w], &burn_device);

// Bridge: Tensor → CubeTensor → ComputeClient::get_resource → WgpuResource.
let primitive: TensorPrimitive<Wgpu> = burn_tensor.into_primitive();
let cube: CubeTensor<WgpuRuntime> = primitive.tensor(); // Float variant
let managed = cube.client.get_resource(cube.handle.clone()).unwrap();
let wgpu_buf: &wgpu::Buffer = &managed.resource().buffer;
let dst_offset: u64 = managed.resource().offset;
let dst_size: u64 = managed.resource().size;

// Squarebob's encoder copies into Burn's buffer:
encoder.copy_buffer_to_buffer(
    &pt_color_aov,      // source: squarebob's PT output
    0,                  // src_offset
    wgpu_buf,           // destination: Burn's storage
    dst_offset,
    dst_size,
);

// Hand `burn_tensor` (still the same instance — we recreate it from
// `cube` via TensorPrimitive::Float + Tensor::from_primitive) to
// the OIDN filter's `set_color_tensor`. From here it stays on-device.
```

For output (after `take_output_tensor`):

```rust
let out_tensor: Tensor<Wgpu, 4> = filter.take_output_tensor().unwrap();
let cube_out = out_tensor.into_primitive().tensor();
let managed_out = cube_out.client.get_resource(cube_out.handle.clone()).unwrap();
let src_buf = &managed_out.resource().buffer;

// Squarebob: pipe through PathTraceCompute::blit_with_source so the
// existing ACES + gamma + hover-overlay path keeps working. We need
// a [src_buf → result_texture] copy step:
encoder.copy_buffer_to_texture(
    wgpu::TexelCopyBufferInfo {
        buffer: src_buf,
        layout: wgpu::TexelCopyBufferLayout {
            offset: managed_out.resource().offset,
            bytes_per_row: Some(w as u32 * 4 * std::mem::size_of::<f32>() as u32),
            rows_per_image: Some(h as u32),
        },
    },
    wgpu::TexelCopyTextureInfo { texture: &oidn_result_texture, /* ... */ },
    wgpu::Extent3d { width: w as u32, height: h as u32, depth_or_array_layers: 1 },
);
```

### What about the wgpu::Device identity?

`cubecl-wgpu-0.10.0/src/runtime.rs:232` defines `pub fn init_device(setup: WgpuSetup, options: RuntimeOptions) -> WgpuDevice`. `WgpuSetup::Existing { device, queue, .. }` stores the `Arc<wgpu::Device>` we provide. The buffers it allocates are created via `device.create_buffer(...)` on the same `Arc`. Our `encoder` (created from the same `Arc<wgpu::Device>`) can issue `copy_buffer_to_buffer` against Burn-allocated buffers without further coordination. **Confirmed safe.**

### Constraints to handle in code

1. **Layout pitch.** `MemoryLayoutStrategy::Optimized` (used by `empty_tensor`) may pad the last dim for alignment. For our `[1, 3, H, W]` f32 tensors the row stride could differ from `W * 4` bytes. Two options:
   - Use `MemoryLayoutStrategy::Contiguous` instead (`client.empty(size)` followed by manual shape + stride bookkeeping in a fresh `CubeTensor::new_contiguous`).
   - Or accept the pitched layout and pass `bytes_per_row` to `copy_buffer_to_texture` accordingly.
   The contiguous path is simpler; pick it unless benchmarks force the issue.

2. **Stream synchronisation.** `ComputeClient::get_resource` does `submit_blocking` (line 208), which forces the cubecl-side stream to flush. Our PT-side encoder copy is on the same `wgpu::Queue`, so wgpu serialises them naturally — but be aware that `get_resource` is not free; it inserts a stream barrier. Cache the resource lookup across re-fires when possible (we already cache the `RtFilter` itself; cache the resource pointers next to it).

3. **`Handle::clone` is cheap** (line 35 of handle.rs — just `Arc` clones). Re-getting `get_resource` per denoise call is fine after warm-up.

---

## Implementation plan for I.5 + I.6 (combined)

Both task descriptions overlap — the I.5 "bridge" lands inside `pt-denoise-oidn` (squarebob side), and I.6 is the actual `OidnDenoiser::denoise` rewrite that uses it. There's no clean separation; do them as one commit.

**File: `crates/pt-denoise-oidn/src/lib.rs` (squarebob)**

1. Add a `WgpuTensorBridge` helper module:
   - `pub fn alloc_input_tensor(burn_device: &WgpuDevice, w: usize, h: usize, ch: usize) -> (Tensor<Wgpu, 4>, wgpu::Buffer_handle)` — allocates an empty Burn tensor and caches its `(buffer_arc_clone, offset, size)`.
   - `pub fn alloc_output_tensor(...)` — same for output.
   - Both use `Tensor::zeros` + `into_primitive` + `client.get_resource`. Cache the bridge data on the `OidnDenoiser` struct alongside the existing `RtFilter` cache.

2. Rewrite `OidnDenoiser::denoise`:
   - Pre-allocate (or reuse cached) input tensors.
   - For each PT-side AOV buffer (color/albedo/normal): `encoder.copy_buffer_to_buffer(pt_aov, 0, &burn_buf, offset, size)`.
   - The existing copy from `pt_output` texture into a staging buffer becomes `copy_texture_to_buffer(pt_output, &burn_color_buf)` directly (no intermediate readback).
   - Submit the encoder before running OIDN (or include OIDN's pre-process kernels in the same submit — cubecl will schedule them on the same queue).
   - Call `filter.set_color_tensor(burn_color_t)` etc., then `filter.commit()`, `filter.execute()`.
   - `let out_t = filter.take_output_tensor().unwrap();`
   - Bridge the output tensor's wgpu::Buffer back to the existing `result_texture` via `encoder.copy_buffer_to_texture(...)`.

3. Retire the `map_async` readback path in `denoise_image`. The HWC `Vec<f32>` allocation, the `to_rgb_f32` decode, and the staging buffer pool all go away. Keep them only behind a `#[cfg(test)]` or a feature flag for parity testing.

**File: `crates/oidn-rs/src/filters/unet_runner.rs`**

Currently `run()` takes `Option<&Image<'_>>` and does its own host upload at the top of the function. Add a second entry point that accepts pre-built input tensors directly:

```rust
pub fn run_tensors<B: Backend>(
    net: &Net<B>,
    device: &B::Device,
    plan: &TilePlan,
    color: Option<Tensor<B, 4>>,
    albedo: Option<Tensor<B, 4>>,
    normal: Option<Tensor<B, 4>>,
    /* ...same other args... */
) -> Result<Tensor<B, 4>, OidnError>
```

The body is mostly a copy of the current `run` minus the `to_rgb_f32` and `upload_hwc_as_chw_tensor` calls. The autoexposure switch from `compute_scale` (host) to `compute_scale_tensor` (device) happens here.

Have the legacy `run(&Image<'_>, ...)` wrap `run_tensors` with the existing upload/download bookends — that keeps tests + the CLI working while squarebob takes the fast path.

**Estimated effort (revised against original 4-12 h + 3-4 h = 7-16 h):**

| Step | Hours |
|---|---|
| Add `run_tensors` in oidn-rs | 1.5 |
| Bridge helpers + cache in `pt-denoise-oidn` | 2.0 |
| Rewrite `OidnDenoiser::denoise` to the new path | 2.0 |
| Squarebob-side encoder plumbing (`copy_buffer_to_buffer`, `copy_buffer_to_texture`, fix pitch math if needed) | 1.5 |
| Validation: smoke-test against existing scenes, log compare denoise output stats vs the legacy path | 1.0 |
| **Total** | **8 h** ≈ 1 day |

Path 1's deferred 1-3 days of cubecl fork work disappears.

---

## What this changes about the broader plan

- I.5 stops being "risky" — the cubecl-patch fork branch is no longer on the critical path.
- I.6 collapses into I.5 (single commit covers both — the bridge IS the integration).
- I.7 (bench + visual regression) stays unchanged in shape; numbers will be cleaner because the new path is structurally simpler.

After I.5+I.6 lands, the only remaining host-side work per `OidnDenoiser::denoise` call is:
- Reading scalar parameters into a `TransferState` (microseconds).
- Calling `compute_scale_tensor` (which itself only reads two scalars back, per I.3).
- Issuing the `encoder.submit` (microseconds).

Wall-clock prediction:
- 1080p, mode `ColorAlbedoNormal`, base model: should drop from ~200 ms to ~140-160 ms (~25-30 % saving). The dominant cost is `Net::forward` (UNet inference), which Phase I doesn't touch.
- 4K: bigger absolute savings because the host roundtrip currently amounts to several MB per channel.

The remaining ~150 ms is what a future Phase II "lift U-Net into SPIR-V passthrough custom kernel" would target. That's out of scope here.
