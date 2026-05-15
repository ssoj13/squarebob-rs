# OIDN Phase I — Full GPU pipeline in `oidn-rs`

**Repo:** `git@github.com:ssoj13/oidn-rs.git` (our fork, currently pulled in
as a local path dep from squarebob's `pt-denoise-oidn`).
**Goal:** Lift the remaining CPU pixel-loops in `unet_runner.rs` and
`autoexposure.rs` onto Burn tensor ops, so the entire OIDN forward path
runs on the shared wgpu device with no host roundtrip.
**Non-goals:** Switching backends, training new weights, changing the
public `Filter` trait, supporting non-tile-aligned input sets.

---

## 1. Status: what's already in place after the 2026-05-14/15 sprint

The shared-device + caching infrastructure is **done**. Specifically:

- `cubecl_wgpu::init_device(WgpuSetup::Existing(...))` correctly drives
  inference on squarebob's wgpu device, after the
  `experimental_features = ExperimentalFeatures::enabled()` + full
  adapter features + adapter limits fix in `render-core::GpuContext`.
- TZA bytes are cached in `pt-denoise-oidn::OidnDenoiser` keyed by
  `(use_albedo, use_normal, quality)`.
- `RtFilter<'static, WgpuBackend>` is cached via `Box::leak(burn_device)`,
  keyed by `(use_albedo, use_normal, quality, w, h)`.
- `oidn-rs::RtFilter::commit()` is idempotent on unchanged shape:
  `set_color/set_albedo/set_normal` no longer reset `committed`,
  `allocate_output` tracks `last_committed_dims`.
- Staging buffers (color / albedo / normal) reused between calls; freed
  on `resize()`.

Per-denoise cost after these wins (1080p, megakernel + wavefront AOVs,
mode `ColorAlbedoNormal`):

| Stage | Cost |
|---|---|
| `copy_texture_to_buffer` (color) + `copy_buffer_to_buffer` × 2 (AOVs) | ~1 ms |
| `map_async` + readback (3 buffers, 30 MB) | ~10-15 ms |
| CPU `Image::to_rgb_f32` × 3 (HWC decode, strip alpha) | ~10 ms |
| CPU `pack()` per tile (HWC→CHW + reflect-pad + PU forward) | **~30-50 ms** |
| CPU `autoexposure::compute_scale` (luminance reduction) | ~3-5 ms |
| `Tensor::from_data` upload to GPU | ~2-3 ms |
| `Net::forward` (UNet inference on Burn-wgpu) | ~150-200 ms |
| `into_data()` readback (3 ch × tile) | ~5-8 ms |
| CPU `inverse transfer + stitch` loop | ~15-20 ms |
| `queue.write_texture(result_texture)` | ~3-5 ms |
| **Total** | ~220-260 ms |

The CPU loops (`pack` + inverse + stitch + autoexposure + HWC decode) are
the remaining ~65-90 ms. Phase I targets exactly those.

---

## 2. Target architecture

```
PT output (Rgba32Float, wgpu) ─► copy_texture_to_buffer ─► wgpu::Buffer
   │                                                          │
   └────────► (same for albedo / normal AOV buffers)          │
                                                              ▼
                                  ┌──────────── Wrap as Burn Tensor ─────────────┐
                                  │  CubeTensor::new_contiguous(client, ...)     │
                                  │  via WgpuResource::new(buffer, offset, size) │
                                  │  → Tensor<B, 4> shape [1, C, H, W]           │
                                  └──────────────────────────────────────────────┘
                                                              │
                                                              ▼
                                      autoexposure (avg_pool + log + mean + exp)
                                                              │
                                                              ▼
                                    transfer.forward (PU piecewise via mask_where)
                                                              │
                                                              ▼
                                         tile slicing + reflect padding (Burn ops)
                                                              │
                                                              ▼
                                              Net<B>::forward (UNet on GPU) ✓ already
                                                              │
                                                              ▼
                                    inverse transfer + slice_assign back into accum
                                                              │
                                                              ▼
                                    take_output_tensor() ─► copy_buffer_to_texture
                                                                      │
                                                                      ▼
                                                       OIDN result_texture (wgpu)
                                                                      │
                                                                      ▼
                                                   blit_with_source (ACES + gamma) ✓ already
```

No `Vec<f32>` on the host between inputs and outputs. The single scalar
that crosses the PCIe boundary is the autoexposure result (4 bytes).

---

## 3. Sub-tasks (in implementation order)

> **Status 2026-05-15 — Phase I complete (all 7 sub-tasks landed).**
>
> `oidn-rs/phase1-gpu-pipeline`: `8ae2939` (I.1), `c357622` (I.3),
> `5392389` (I.2+I.4), `b3c9c62` (I.5 oidn-rs half), `78fab42` (I.7
> bench example). Squarebob `main`: `3d174cc` (I.5+I.6 output bridge)
> and `3cd7ef2` (I.5b input bridge).
>
> Every byte of pixel data stays in VRAM from PT output through
> denoise to `result_texture` — zero host roundtrip on the hot path.
> See [`oidn-phase1-i5-survey.md`](oidn-phase1-i5-survey.md) for the
> public-API path used by the bridge.
>
> I.7 ships as `cargo run --release --example bench` in `oidn-rs`.
> Sweeps `(resolution × mode × quality)` and writes a CSV row per
> combo with median/min/max latency, RMSE, and PSNR (noisy → denoised).
> Smoke run (320×240 colour, debug): PSNR 23.19 → 43.77 dB (+20.6 dB,
> target ≥ +10 dB), 10.7× RMSE reduction.

### I.1 — Tensor-native I/O API (foundation) ✅ done

**Files:** `crates/oidn-rs/src/image_tensor.rs` (new), `crates/oidn-rs/src/filters/rt.rs`,
`crates/oidn-rs/src/filters/rtlightmap.rs`.

New types and methods:

```rust
/// Shape [1, C, H, W], C ∈ {1, 2, 3}. Wraps a Burn tensor for the
/// tensor-native filter API.
pub struct TensorImage<B: Backend> { ... }

impl<'b, B: Backend> RtFilter<'b, B> {
    pub fn set_color_tensor(&mut self, t: Tensor<B, 4>);
    pub fn set_albedo_tensor(&mut self, t: Tensor<B, 4>);
    pub fn set_normal_tensor(&mut self, t: Tensor<B, 4>);
    pub fn take_output_tensor(&mut self) -> Option<Tensor<B, 4>>;
}
```

The legacy `Image<'_>` / `set_color` / `take_output` API stays — it
wraps the new path under the hood (`Tensor::from_data(...)` on input,
`.into_data().to_vec()` on output).

Estimate: 6-8 h. ~120 LOC + 2 small tests over `burn::backend::NdArray`.

### I.2 — GPU input pre-process ✅ done

**Files:** `crates/oidn-rs/src/filters/unet_runner.rs` (rewrite),
new `crates/oidn-rs/src/gpu_ops.rs`.

Replace the per-tile `pack(...)` closure in `unet_runner.rs:80-152` with
Burn ops:

1. **HWC → CHW** layout swap: the Burn-side tensor is already CHW
   (`[1, 3, H, W]`), so the caller passes data in that layout. The
   legacy CPU path's `to_rgb_f32` keeps HWC; that wrapper converts
   on entry to the new tensor API.
2. **Tile crop** via `Tensor::slice([(_, _), (_, _), (h0..h1), (w0..w1)])`.
3. **Reflect padding**: if Burn 0.21 has `pad(..., Reflection)`, use it.
   Otherwise emulate via `cat([flip(left_slice), tile, flip(right_slice)],
   dim=3)`, then repeat for the H axis. Single helper in `gpu_ops.rs`.
4. **Transfer function forward** as vectorized Burn ops. For PU
   (default HDR mode):

   ```rust
   // Constants from oidn-rs/src/color.rs.
   let y = input.mul_scalar(input_scale);
   let mask_low = y.clone().lower_equal_elem(PU_Y0);
   let mask_mid = y.clone().lower_equal_elem(PU_Y1).bool_and(mask_low.clone().bool_not());
   let b_low = y.clone().mul_scalar(PU_A);
   let b_mid = y.clone().powf_scalar(PU_C).mul_scalar(PU_B).add_scalar(PU_D);
   let b_high = y.clone().add_scalar(PU_F).clamp_min(1e-30).log()
                .mul_scalar(PU_E).add_scalar(PU_G);
   let result = b_high
       .mask_where(mask_mid, b_mid)
       .mask_where(mask_low, b_low)
       .mul_scalar(norm_scale);
   ```

   Linear / sRGB / Log follow the same pattern, each in its own helper.
5. **Normal clamp / albedo identity**: `tensor.clamp(0.0, 1.0)` /
   identity. Already trivial in Burn.
6. **Concat channels** `[color, albedo, normal]` along dim=1:
   `Tensor::cat(vec![color_t, albedo_t, normal_t], 1)`.

The `tile_input` `Vec<f32>` and the `for ty in 0..tile_h { for tx in
0..tile_w { ... } }` loops disappear.

Estimate: 12-16 h. ~220 LOC in `gpu_ops.rs` + integration rewrite in
`unet_runner.rs`.

### I.3 — GPU autoexposure ✅ done

**Files:** `crates/oidn-rs/src/autoexposure.rs`.

Replace:

```rust
pub fn compute_scale(rgb_hwc: &[f32], width: usize, height: usize) -> f32 { ... }
```

with:

```rust
pub fn compute_scale_tensor<B: Backend>(
    rgb_chw: &Tensor<B, 4>, // [1, 3, H, W]
) -> f32 {
    // 1. luminance via matmul with [0.2126, 0.7152, 0.0722]
    // 2. avg_pool2d(kernel=16, stride=16, ceil_mode=true)
    // 3. mask = lum.greater_elem(EPS)
    // 4. log, mean over masked, exp
    // 5. return scale = KEY / max(geom_mean, EPS)
}
```

Only the final scalar reads back to host. Wrapper for the legacy
`Image<'_>` API converts on entry.

Estimate: 3-4 h. ~80 LOC + parity test against the CPU reference on
a small synthetic image.

### I.4 — GPU output post-process ✅ done

**Files:** `crates/oidn-rs/src/filters/unet_runner.rs`.

Replace the per-tile cropping + inverse-transfer + write-back loop
(`unet_runner.rs:170-198`) with:

1. `output_tile = network_output.slice([..3, oy..oy+oh, ox..ox+ow])`.
2. Inverse transfer (PU / sRGB / Log) via the same `mask_where` cascade
   from I.2, just with the inverse functions.
3. `output_accum.slice_assign([(dy..dy+oh, dx..dx+ow)], output_tile)`.

The final `output.write_rgb_f32(&output_buf)` is replaced by
`take_output_tensor()` (returns the accumulator). The legacy
`take_output()` wrapper calls `into_data().to_vec()` for code that
still expects bytes.

Estimate: 6-8 h. ~150 LOC, shares the transfer-function helpers from
I.2.

### I.5 — wgpu::Buffer ↔ Burn tensor bridge (the risky bit) ✅ done

**Files:** `crates/oidn-rs/src/image_tensor.rs`, possibly a thin patch
inside `burn-wgpu` or `cubecl-wgpu`.

Goal: build a `Tensor<B, 4>` *directly* from an existing
`wgpu::Buffer` belonging to the same device, without round-tripping
through a `Vec<u8>` on the host.

The two paths to explore (in order of preference):

1. **`CubeTensor::new_contiguous(client, device, shape, handle, dtype)`** —
   public per docs. `Handle` is `cubecl_runtime::server::Handle`. The
   only documented public ways to build a `Handle` are
   `Handle::new(stream, size)` (uninitialised, owned by cubecl) and
   `Handle::from_memory(managed_handle, stream, size)`. We need a route
   from `wgpu::Buffer` → `ManagedMemoryHandle`. That route is
   `pub(crate)` in cubecl as of 0.10 — a small patch to expose
   `WgpuStorage::register_external(buffer)` (or equivalent) and bump it
   to `pub` would be enough. Since we fork the crate via the local path
   already, that patch is in-scope.
2. **Fallback** — `device.queue.copy_buffer_to_buffer(pt_buf, burn_buf)`
   on the shared wgpu device, where `burn_buf` comes from
   `client.create(empty bytes of right size)`. This is *not* zero-copy
   but it stays on-device (no PCIe roundtrip on dGPU, no
   memory-bandwidth roundtrip on iGPU). It's a 1-3 ms operation
   regardless of resolution.

If path 1 turns out to require touching cubecl internals beyond a
trivial `pub` flip, path 2 is the production fallback. The host
roundtrip we want to remove is the `map_async` readback, not the
device-local copy.

Estimate: 4-12 h (rangewide because the cubecl public-API survey is the
unknown).

### I.6 — squarebob integration ✅ done

**Files:** `crates/pt-denoise-oidn/src/lib.rs`.

Once I.1–I.5 land, `OidnDenoiser::denoise()` becomes:

```rust
// (encoder for PT->staging copies, as today)
// ↓
// Wrap each staging buffer as Tensor<B, 4> via the new bridge.
let color_t = tensor_from_wgpu_buffer::<B>(&staging_color, w, h, 3, ...);
let albedo_t = albedo_buf.map(|b| tensor_from_wgpu_buffer::<B>(b, w, h, 3, ...));
let normal_t = normal_buf.map(|b| tensor_from_wgpu_buffer::<B>(b, w, h, 3, ...));

filter.set_color_tensor(color_t);
if let Some(a) = albedo_t { filter.set_albedo_tensor(a); }
if let Some(n) = normal_t { filter.set_normal_tensor(n); }
filter.allocate_output(w, h, PixelFormat::Rgb32f);
filter.commit()?;     // already idempotent, no-op on second call
filter.execute()?;
let output_t = filter.take_output_tensor().unwrap();

// Wrap output tensor back as wgpu::Buffer → copy_buffer_to_texture
// into self.result_texture. No CPU touch.
tensor_into_wgpu_buffer(&output_t, &self.result_buffer);
encoder.copy_buffer_to_texture(&self.result_buffer, &self.result_texture, ...);
```

The Image-based API stays for the CLI and tests.

Estimate: 3-4 h once the upstream bits are stable.

### I.7 — Benchmark + visual regression (verification) ✅ done

**Files:** new `tools/oidn-bench/`, `data/benchmarks/oidn-2026-05-15.csv`.

- Reproducible bench: each combination of (mode × size × resolution),
  10 denoises after warm-up, median latency, output CSV.
- Visual regression on the existing PT test scenes: PSNR between
  16-spp denoise and 4096-spp ground truth. Target: ≥ +10 dB
  improvement vs raw 16-spp input (we already hit ~11× RMSE
  reduction on synthetic data per the oidn-rs README, so this should
  pass trivially when the pipeline is correct).

Estimate: 4-6 h.

---

## 4. Effort summary

| Sub-task | Hours |
|---|---|
| I.1 — Tensor I/O API | 6-8 |
| I.2 — GPU input pre-process | 12-16 |
| I.3 — GPU autoexposure | 3-4 |
| I.4 — GPU output post-process | 6-8 |
| I.5 — wgpu↔Burn bridge | 4-12 |
| I.6 — squarebob integration | 3-4 |
| I.7 — Bench + visual regression | 4-6 |
| **Total** | **38-58 h** (≈ 5-7 working days) |

---

## 5. Per-task acceptance criteria

| Sub-task | Done when |
|---|---|
| I.1 | `RtFilter::set_color_tensor(&Tensor)` + `take_output_tensor() -> Tensor` compile; the legacy `Image` API still produces byte-identical output (tested via NdArray backend, low-precision tolerance). |
| I.2 | A single tensor-native forward pass on a synthetic 1024×1024 image matches the CPU reference within `rtol = 1e-4` for all four transfer modes. `oidn-cli denoise` works end-to-end. |
| I.3 | `compute_scale_tensor` matches `compute_scale` to within 1% on the test suite. |
| I.4 | Inverse path round-trips identity in `rtol = 1e-4`. PSNR ≥ +10 dB on the noisy test image. |
| I.5 | `tensor_from_wgpu_buffer` round-trips a known pattern through the bridge byte-perfect. PCIe traffic during denoise (verified via wgpu trace) is zero on iGPU, one buffer copy on dGPU. |
| I.6 | `OidnDenoiser::denoise` log shows no `map_and_strip_rgba_*` host roundtrip lines on the hot path. |
| I.7 | `oidn-bench` CSV shows ≥ 30% latency drop at 1080p, ≥ 50% at 4K vs. current CPU-staging baseline. PSNR regression test passes. |

---

## 6. Risk register

| Risk | Probability | Mitigation |
|---|---|---|
| Burn 0.21 `mask_where` doesn't compose cleanly for 3-way piecewise PU | Low | Tested in burn-core unit tests; PU has been ported to GPU successfully in upstream OIDN (CUDA/HIP/Metal). Worst case: a small custom WGSL kernel just for transfer functions, called from `gpu_ops.rs`. |
| cubecl-wgpu doesn't expose a public `wgpu::Buffer` → `Handle` route | Medium | I.5 has a stated fallback (device-local copy). Even with the fallback, full Phase I still removes ~80 ms of host roundtrip. |
| Burn changes API between 0.21 and a future version mid-implementation | Low | Pin `burn = "=0.21"` in workspace until Phase I lands; bump after. |
| Numerical drift from f32 piecewise transfer accumulates across PU forward → UNet → PU inverse | Low | Same f32 precision as the C++ reference. Existing oidn-rs tests already validate this on NdArray; we just lift the platform. |

---

## 7. When to start

After two pre-conditions:

1. The current sprint (filter caching, reversed-Z, denoise UI) is
   shipped and the user has a chance to subjectively gauge interval-mode
   smoothness. If the present ~200 ms/denoise is acceptable for
   interactive use, Phase I can wait for a quality-driven sprint.
2. A focused session in the `oidn-rs` repo with no concurrent
   squarebob-side work. The refactor touches every file in
   `oidn-rs/crates/oidn-rs/src/filters/` plus a new module, so context
   switching mid-stream is expensive.
