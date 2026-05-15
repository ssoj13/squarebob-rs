//! `pt-denoise-oidn` — Intel OIDN integration for squarebob-rs path-tracer output.
//!
//! Runs the OIDN U-Net on the *same* wgpu device as the renderer (shared via
//! `cubecl_wgpu::init_device(WgpuSetup{...})`). Burn-wgpu allocates its tensors
//! on that shared device, so the only cross-system bridge is the input/output
//! staging on the host: the Image-based API in `oidn-rs` today still expects
//! CPU slices. Phase I in `oidn-rs` lifts this to pure GPU tensors and removes
//! the host roundtrip; the public API here stays unchanged.
//!
//! Pipeline per `denoise()` call:
//! 1. `copy_texture_to_buffer(color_tex)` + `copy_buffer_to_buffer(albedo/normal)`
//!    into mappable staging buffers on the *same* wgpu device.
//! 2. `device.poll(Wait)` + `map_async(Read)` → contiguous `Vec<u8>` per input.
//! 3. Strip alpha (`Rgba32Float` → `Rgb32f` 12-byte stride) into f32 slices.
//! 4. Build a one-shot `RtFilter<WgpuBackend>`, set inputs, commit, execute.
//! 5. `take_output()` → `queue.write_texture(result_texture)`.
//!
//! The denoiser is built lazily on the first `denoise()` call with a
//! non-`Off` mode, so app startup pays no TZA load cost.

#![forbid(unsafe_op_in_unsafe_fn)]

use std::path::PathBuf;
use std::time::Instant;

use anyhow::Result;
use render_core::gpu::GpuContext;

use oidn_rs::filter::Filter;
pub use oidn_rs::Quality;

// ---------- Public API ----------

/// Inputs the denoiser feeds into OIDN. Higher modes pick a richer model:
/// `Color` → `rt_hdr`, `ColorAlbedo` → `rt_hdr_alb`, `ColorAlbedoNormal` →
/// `rt_hdr_alb_nrm`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum OidnMode {
    Off,
    Color,
    ColorAlbedo,
    #[default]
    ColorAlbedoNormal,
}

impl OidnMode {
    /// `(uses_albedo, uses_normal)` — drives which AOV buffers the caller
    /// must supply to [`OidnDenoiser::denoise`].
    pub fn requires_aov(self) -> (bool, bool) {
        match self {
            Self::Off => (false, false),
            Self::Color => (false, false),
            Self::ColorAlbedo => (true, false),
            Self::ColorAlbedoNormal => (true, true),
        }
    }
}

/// Lazy-built OIDN denoiser.
pub struct OidnDenoiser {
    weights_dir: PathBuf,

    mode: OidnMode,
    quality: Quality,
    width: u32,
    height: u32,

    /// Linear `Rgba32Float` result texture, same dims as input. Lazily allocated.
    result_texture: Option<wgpu::Texture>,
    result_view: Option<wgpu::TextureView>,

    /// Last successful execute() wallclock for UI display.
    last_latency_ms: Option<f32>,

    /// Burn device sharing squarebob's wgpu setup. Built on first denoise.
    /// Stored as a `'static` reference (via `Box::leak`) so the cached
    /// `RtFilter` below can carry a `'static` lifetime parameter — a
    /// deliberate single-instance leak of ~16 bytes per app process.
    burn_device_ref: Option<&'static burn_wgpu::WgpuDevice>,

    /// Cached TZA bytes from the last successful model load. Reused across
    /// `denoise()` calls so we don't re-read the 1.8 MB file from disk
    /// on every interval-fire. Key = (use_albedo, use_normal, quality)
    /// since these fully determine which TZA file gets picked.
    cached_model_key: Option<(bool, bool, Quality)>,
    cached_model_bytes: Option<Vec<u8>>,

    /// Cached `RtFilter`. Carries the loaded UNet and tile plan from one
    /// `denoise()` to the next, so the ~30-50 ms `commit()` cost (TZA parse
    /// + UNet weight load + tile-plan compute) is paid once per
    /// mode/quality/dims combination, not every periodic fire.
    cached_filter:
        Option<Box<oidn_rs::RtFilter<'static, burn_wgpu::Wgpu<f32, i32>>>>,
    cached_filter_key: Option<(bool, bool, Quality, u32, u32)>,

}

impl OidnDenoiser {
    pub fn new(_ctx: &GpuContext, width: u32, height: u32, weights_dir: PathBuf) -> Self {
        Self {
            weights_dir,
            mode: OidnMode::default(),
            quality: Quality::Balanced,
            width,
            height,
            result_texture: None,
            result_view: None,
            last_latency_ms: None,
            burn_device_ref: None,
            cached_model_key: None,
            cached_model_bytes: None,
            cached_filter: None,
            cached_filter_key: None,
        }
    }

    pub fn resize(&mut self, _ctx: &GpuContext, width: u32, height: u32) {
        if self.width == width && self.height == height {
            return;
        }
        self.width = width;
        self.height = height;
        self.result_texture = None;
        self.result_view = None;
        // Filter cache is keyed on (mode, quality, w, h) — when dims change
        // the cached filter is invalidated lazily on the next denoise call
        // (see filter_key check there). Burn input tensors are allocated
        // per-call by the I.5b bridge, so there's nothing dimension-tied
        // to drop here aside from the result texture above.
    }

    pub fn set_mode(&mut self, mode: OidnMode) {
        self.mode = mode;
    }

    pub fn set_quality(&mut self, quality: Quality) {
        self.quality = quality;
    }

    pub fn mode(&self) -> OidnMode {
        self.mode
    }

    pub fn quality(&self) -> Quality {
        self.quality
    }

    pub fn result_view(&self) -> Option<&wgpu::TextureView> {
        self.result_view.as_ref()
    }

    pub fn last_latency_ms(&self) -> Option<f32> {
        self.last_latency_ms
    }

    /// Run a single denoise pass. `color_tex` must be the PT accumulator
    /// (Rgba32Float, sample-normalized — the divide-by-frame-count step
    /// has already happened, e.g. in wavefront's `finalize.wgsl`).
    /// `albedo_buf` / `normal_buf` are required when [`OidnMode`] needs them;
    /// see [`OidnMode::requires_aov`]. The supplied `encoder` is consumed —
    /// caller must obtain a fresh one for any subsequent work, since
    /// `denoise` submits its own command stream to the queue.
    pub fn denoise(
        &mut self,
        ctx: &GpuContext,
        encoder: wgpu::CommandEncoder,
        color_tex: &wgpu::Texture,
        albedo_buf: Option<&wgpu::Buffer>,
        normal_buf: Option<&wgpu::Buffer>,
    ) -> Result<()> {
        log::debug!(
            "OIDN denoise() enter: mode={:?} quality={:?} dims={}x{} albedo_in={} normal_in={}",
            self.mode,
            self.quality,
            self.width,
            self.height,
            albedo_buf.is_some(),
            normal_buf.is_some(),
        );
        if matches!(self.mode, OidnMode::Off) {
            log::debug!("OIDN denoise() early-return: mode=Off");
            return Ok(());
        }

        // Graceful downgrade: if the configured mode needs AOVs but they
        // aren't available (e.g. wavefront PT disabled), drop to the richest
        // mode the supplied inputs support. This keeps the default preset
        // (`ColorAlbedoNormal` + wavefront off) working out of the box —
        // it produces a `Color` denoise rather than an error.
        let effective_mode = match self.mode {
            OidnMode::ColorAlbedoNormal => {
                if normal_buf.is_some() && albedo_buf.is_some() {
                    OidnMode::ColorAlbedoNormal
                } else if albedo_buf.is_some() {
                    OidnMode::ColorAlbedo
                } else {
                    OidnMode::Color
                }
            }
            OidnMode::ColorAlbedo => {
                if albedo_buf.is_some() {
                    OidnMode::ColorAlbedo
                } else {
                    OidnMode::Color
                }
            }
            other => other,
        };
        log::debug!(
            "OIDN effective_mode={:?} (use_albedo={} use_normal={})",
            effective_mode,
            matches!(effective_mode, OidnMode::ColorAlbedo | OidnMode::ColorAlbedoNormal),
            matches!(effective_mode, OidnMode::ColorAlbedoNormal),
        );
        if effective_mode != self.mode {
            log::debug!(
                "OIDN: mode {:?} downgraded to {:?} (AOV unavailable)",
                self.mode, effective_mode
            );
        }
        let use_albedo = matches!(
            effective_mode,
            OidnMode::ColorAlbedo | OidnMode::ColorAlbedoNormal
        );
        let use_normal = matches!(effective_mode, OidnMode::ColorAlbedoNormal);

        // Lazy init: Burn device + result texture. The device is built once
        // and leaked to `'static` so the cached `RtFilter` (parameterised
        // over `&'b WgpuDevice`) can survive across denoise calls — saves
        // the UNet rebuild on every periodic fire.
        if self.burn_device_ref.is_none() {
            let dev = make_burn_device(ctx)?;
            let leaked: &'static burn_wgpu::WgpuDevice = Box::leak(Box::new(dev));
            self.burn_device_ref = Some(leaked);
            log::info!("OIDN: Burn-wgpu device initialised on shared wgpu setup (leaked to 'static for filter caching)");
        }
        if self.result_texture.is_none() {
            let (tex, view) = create_result_texture(&ctx.device, self.width, self.height);
            self.result_texture = Some(tex);
            self.result_view = Some(view);
        }

        let started = Instant::now();
        // From here on use `effective_mode` rather than `self.mode` so the
        // model picker and AOV reads stay consistent with the downgrade.
        let w = self.width as usize;
        let h = self.height as usize;
        let n = w * h;

        // I.5b input bridge: copy PT-side data directly into Burn-allocated
        // wgpu::Buffers, then wrap as on-device tensors. No host roundtrip.
        //
        // Width constraint: `w * 16` must be a multiple of 256 (wgpu's
        // `COPY_BYTES_PER_ROW_ALIGNMENT`). Common viewport widths satisfy
        // this. Padded-row fallback is deferred until a non-aligned width
        // shows up in the field.
        let unpadded_bpr = (w as u64) * 16;
        if unpadded_bpr % 256 != 0 {
            anyhow::bail!(
                "OIDN tensor bridge requires width*16 to be 256-byte aligned (got w={w}, bpr={unpadded_bpr}). \
                 Use a width that's a multiple of 16 px, or extend the bridge with a padded-row fallback."
            );
        }

        // Respect downgrade: drop AOV inputs we don't intend to consume so
        // we don't allocate / copy ~30 MB of AOV per side just to throw away.
        let albedo_buf = if use_albedo { albedo_buf } else { None };
        let normal_buf = if use_normal { normal_buf } else { None };

        let burn_device: &'static burn_wgpu::WgpuDevice = self
            .burn_device_ref
            .expect("burn_device init guaranteed above");

        // Allocate one [1, H, W, 4] HWC RGBA tensor per active input and
        // grab the underlying wgpu::Buffer for the encoder copy below.
        use burn::tensor::Tensor;
        type Wgpu = burn_wgpu::Wgpu<f32, i32>;
        let (color_hwc4, color_buf, color_off) = alloc_hwc4_input(burn_device, w, h)?;
        let albedo_bridge = albedo_buf.map(|src| -> Result<_> {
            let (t, buf, off) = alloc_hwc4_input(burn_device, w, h)?;
            Ok((t, buf, off, src))
        }).transpose()?;
        let normal_bridge = normal_buf.map(|src| -> Result<_> {
            let (t, buf, off) = alloc_hwc4_input(burn_device, w, h)?;
            Ok((t, buf, off, src))
        }).transpose()?;

        let mut encoder = encoder;
        // Color: copy_texture_to_buffer with natural HWC RGBA layout.
        encoder.copy_texture_to_buffer(
            wgpu::TexelCopyTextureInfo {
                texture: color_tex,
                mip_level: 0,
                origin: wgpu::Origin3d::ZERO,
                aspect: wgpu::TextureAspect::All,
            },
            wgpu::TexelCopyBufferInfo {
                buffer: &color_buf,
                layout: wgpu::TexelCopyBufferLayout {
                    offset: color_off,
                    bytes_per_row: Some(unpadded_bpr as u32),
                    rows_per_image: Some(self.height),
                },
            },
            wgpu::Extent3d {
                width: self.width,
                height: self.height,
                depth_or_array_layers: 1,
            },
        );
        // AOVs: already vec4<f32> tightly packed, copy_buffer_to_buffer
        // straight into the Burn allocation.
        let aov_size = (n as u64) * 16;
        if let Some((_, ref dst_buf, dst_off, src)) = albedo_bridge {
            encoder.copy_buffer_to_buffer(src, 0, dst_buf, dst_off, aov_size);
        }
        if let Some((_, ref dst_buf, dst_off, src)) = normal_bridge {
            encoder.copy_buffer_to_buffer(src, 0, dst_buf, dst_off, aov_size);
        }
        ctx.queue.submit(std::iter::once(encoder.finish()));
        log::trace!(
            "OIDN: input bridge submitted ({} bytes color + 2×{} AOV)",
            unpadded_bpr * (h as u64), aov_size,
        );

        // Convert HWC RGBA → CHW RGB on the same wgpu device via Burn ops.
        // permute is a strided view; downstream ops in run_tensors (slice,
        // reflect_pad, transfer, cat) handle non-contiguous tensors.
        let color_chw3_t: Tensor<Wgpu, 4> = hwc4_to_chw3(color_hwc4);
        let albedo_chw3_t: Option<Tensor<Wgpu, 4>> =
            albedo_bridge.map(|(t, _, _, _)| hwc4_to_chw3(t));
        let normal_chw3_t: Option<Tensor<Wgpu, 4>> =
            normal_bridge.map(|(t, _, _, _)| hwc4_to_chw3(t));

        // Build the filter and run inference. `burn_device` was acquired
        // above for the input bridge; reuse the same reference here.
        let hdr = matches!(
            self.mode,
            OidnMode::Color | OidnMode::ColorAlbedo | OidnMode::ColorAlbedoNormal
        );

        // Diagnostic override: `OIDN_INPUT_SCALE=1.0` (or any number) skips
        // OIDN's autoexposure and uses the value as a hard input_scale. Use
        // 1.0 to test "no scaling at all" — quickly tells us whether the
        // dark output is autoexposure-driven.
        let user_scale: Option<f32> = std::env::var("OIDN_INPUT_SCALE")
            .ok()
            .and_then(|s| s.parse::<f32>().ok());

        // Cache TZA bytes across denoise calls. Key = (use_albedo, use_normal,
        // quality) since hdr is always true for our pipeline. When the user
        // toggles mode/quality we transparently reload.
        let cache_key = (use_albedo, use_normal, self.quality);
        if self.cached_model_key != Some(cache_key) {
            self.cached_model_bytes = None;
            self.cached_model_key = None;
        }
        let cached_bytes = match self.cached_model_bytes.clone() {
            Some(b) => Some(b),
            None => {
                // Resolve filename via oidn-rs registry, then read it once
                // and cache. Errors leave the cache empty so we fall back
                // to the builder's own fs::read on commit.
                let base_key = oidn_rs::registry::select_rt(
                    true, use_albedo, use_normal,
                    /*hdr*/ true, /*srgb*/ false, /*directional*/ false,
                    /*clean_aux*/ false, self.quality,
                );
                if let Some(key) = base_key {
                    let candidates = oidn_rs::registry::quality_candidates(&key, self.quality);
                    let mut loaded: Option<Vec<u8>> = None;
                    for stem in &candidates {
                        let path = self.weights_dir.join(format!("{stem}.tza"));
                        if let Ok(bytes) = std::fs::read(&path) {
                            log::debug!(
                                "OIDN: cached TZA stem={} ({} bytes)",
                                stem, bytes.len()
                            );
                            loaded = Some(bytes);
                            break;
                        }
                    }
                    if let Some(b) = loaded {
                        self.cached_model_bytes = Some(b.clone());
                        self.cached_model_key = Some(cache_key);
                        Some(b)
                    } else {
                        None
                    }
                } else {
                    None
                }
            }
        };

        // Cache the full filter (UNet + tile plan) by (mode/quality/dims).
        // The filter survives between denoise calls — only the input/output
        // images are reassigned, and `commit()` is now idempotent on
        // unchanged shape, so the heavy UNet build cost is paid once.
        let filter_key = (use_albedo, use_normal, self.quality, w as u32, h as u32);
        if self.cached_filter_key != Some(filter_key) {
            self.cached_filter = None;
        }
        let filter_was_cached = self.cached_filter.is_some();
        let cached_bytes_for_build = cached_bytes.clone();
        let filter = self.cached_filter.get_or_insert_with(|| {
            log::debug!(
                "OIDN: building RtFilter (hdr={}, quality={:?}, input_scale_override={:?}, cached_weights={})",
                hdr, self.quality, user_scale, cached_bytes_for_build.is_some()
            );
            let mut builder = oidn_rs::RtFilter::<burn_wgpu::Wgpu<f32, i32>>::builder(
                burn_device,
                &self.weights_dir,
            )
            .hdr(hdr)
            .quality(self.quality);
            if let Some(s) = user_scale {
                builder = builder.input_scale(Some(s));
            }
            if let Some(bytes) = cached_bytes_for_build {
                builder = builder.weights(bytes);
            }
            Box::new(builder.build())
        });
        self.cached_filter_key = Some(filter_key);
        log::trace!(
            "OIDN: filter cached={} (cache_key={:?})",
            filter_was_cached, self.cached_filter_key
        );

        // Tensor handoff to the filter. All three inputs already live on
        // the shared wgpu device — the bridge above wrote PT pixels
        // directly into each tensor's wgpu::Buffer, no host roundtrip.
        filter.set_color_tensor(color_chw3_t);
        if let Some(t) = albedo_chw3_t { filter.set_albedo_tensor(t); }
        if let Some(t) = normal_chw3_t { filter.set_normal_tensor(t); }
        filter.allocate_output_tensor(w, h);
        log::trace!("OIDN: filter.commit() begin");
        filter.commit().map_err(|e| anyhow::anyhow!("OIDN commit: {e:?}"))?;
        log::debug!(
            "OIDN: filter committed (model={:?})",
            filter.model_key().map(|k| k.filename())
        );
        log::trace!("OIDN: filter.execute() begin");
        filter.execute().map_err(|e| anyhow::anyhow!("OIDN execute: {e:?}"))?;
        log::trace!("OIDN: filter.execute() done");
        let out_chw3: Tensor<Wgpu, 4> = filter
            .take_output_tensor()
            .ok_or_else(|| anyhow::anyhow!("OIDN take_output_tensor: empty"))?;
        log::debug!(
            "OIDN: take_output_tensor shape={:?}",
            out_chw3.dims()
        );

        // Bridge: convert the tensor-native CHW RGB output into a
        // contiguous HWC RGBA buffer (alpha=1) on-device, then copy
        // that buffer directly into `result_texture`. No host bytes;
        // no `queue.write_texture` round-trip.
        let out_hwc4 = chw_rgb_to_hwc_rgba_ones(out_chw3, burn_device);
        let result_tex = self.result_texture.as_ref().unwrap();
        copy_tensor_into_texture(&ctx.device, &ctx.queue, out_hwc4, result_tex, w, h)?;

        let elapsed = started.elapsed().as_secs_f32() * 1000.0;
        self.last_latency_ms = Some(elapsed);
        log::info!(
            "OIDN: denoise {}×{} mode={:?} quality={:?} -> {:.1} ms",
            self.width, self.height, self.mode, self.quality, elapsed
        );

        Ok(())
    }
}

// ---------- Helpers ----------

/// Construct a `burn_wgpu::WgpuDevice` that shares squarebob's wgpu setup.
///
/// Without this bridge OIDN would create its own adapter+device, forcing PCIe
/// roundtrips on every input/output buffer. By feeding our `Instance`/
/// `Adapter`/`Device`/`Queue` to `cubecl_wgpu::init_device`, Burn allocates
/// its tensors on the *same* device.
pub fn make_burn_device(ctx: &GpuContext) -> Result<burn_wgpu::WgpuDevice> {
    // A/B test: when this env var is set, build a standalone Burn device
    // (separate wgpu context) so we can isolate whether the all-zero output
    // we're chasing is a `cubecl_wgpu::init_device(WgpuSetup)` bridge bug.
    // Unset → production path (shared device).
    if std::env::var("OIDN_STANDALONE_DEVICE").is_ok() {
        log::warn!(
            "OIDN_STANDALONE_DEVICE set — using non-shared Burn device (debug only)"
        );
        return Ok(burn_wgpu::WgpuDevice::default());
    }

    let backend = ctx.adapter.get_info().backend;
    let setup = cubecl_wgpu::WgpuSetup {
        instance: (*ctx.instance).clone(),
        adapter: (*ctx.adapter).clone(),
        device: (*ctx.device).clone(),
        queue: (*ctx.queue).clone(),
        backend,
    };
    let device = cubecl_wgpu::init_device(setup, cubecl_wgpu::RuntimeOptions::default());
    Ok(device)
}

/// Resolve the directory holding OIDN `.tza` weights in this order:
/// 1. `$OIDN_WEIGHTS_DIR` (highest priority — runtime override).
/// 2. `<exe_dir>/data/oidn-weights/`.
/// 3. `<cwd>/data/oidn-weights/`.
pub fn resolve_weights_dir() -> Result<PathBuf> {
    if let Ok(p) = std::env::var("OIDN_WEIGHTS_DIR") {
        let pb = PathBuf::from(p);
        if pb.exists() {
            return Ok(pb);
        }
    }
    if let Ok(exe) = std::env::current_exe() {
        if let Some(dir) = exe.parent() {
            let pb = dir.join("data").join("oidn-weights");
            if pb.exists() {
                return Ok(pb);
            }
        }
    }
    let pb = std::path::Path::new("data").join("oidn-weights");
    if pb.exists() {
        return Ok(pb);
    }
    anyhow::bail!(
        "OIDN weights directory not found. Set $OIDN_WEIGHTS_DIR or place \
         weights at data/oidn-weights/ (next to the executable, or under \
         the current working directory)."
    )
}

fn create_result_texture(
    device: &wgpu::Device,
    width: u32,
    height: u32,
) -> (wgpu::Texture, wgpu::TextureView) {
    // `Rgba32Float` so the PT megakernel `blit_with_source` pipeline (which
    // expects non-filterable Float textures via `textureLoad`) can read this
    // directly. Display path: this texture is *not* egui-native; instead the
    // caller pipes it through `PathTraceCompute::blit_with_source` so it
    // goes through the same ACES + gamma tonemap (and the hover/selection
    // pipeline that lives upstream of the blit).
    let tex = device.create_texture(&wgpu::TextureDescriptor {
        label: Some("oidn_result"),
        size: wgpu::Extent3d {
            width,
            height,
            depth_or_array_layers: 1,
        },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format: wgpu::TextureFormat::Rgba32Float,
        usage: wgpu::TextureUsages::COPY_DST
            | wgpu::TextureUsages::TEXTURE_BINDING
            | wgpu::TextureUsages::COPY_SRC,
        view_formats: &[],
    });
    let view = tex.create_view(&wgpu::TextureViewDescriptor::default());
    (tex, view)
}

/// Allocate a `[1, H, W, 4]` HWC RGBA Burn tensor on the shared wgpu
/// device and return `(tensor, wgpu::Buffer clone, offset)` so squarebob
/// can issue `copy_texture_to_buffer` / `copy_buffer_to_buffer` directly
/// into the tensor's backing memory. After the copy is submitted, the
/// tensor can be used by Burn ops as if its bytes were populated by
/// Burn itself — the wgpu queue's FIFO ordering guarantees our writes
/// land before any cubecl-side reads.
fn alloc_hwc4_input(
    device: &burn_wgpu::WgpuDevice,
    w: usize,
    h: usize,
) -> Result<(burn::tensor::Tensor<burn_wgpu::Wgpu<f32, i32>, 4>, wgpu::Buffer, u64)> {
    let t = burn::tensor::Tensor::<burn_wgpu::Wgpu<f32, i32>, 4>::zeros(
        [1, h, w, 4],
        device,
    );
    // Reach the underlying CubeTensor via the public Tensor::into_primitive +
    // TensorPrimitive::Float route. All fields on CubeTensor are pub
    // (burn-cubecl-0.21.0/src/tensor/base.rs).
    let primitive = t.clone().into_primitive();
    let cube = match primitive {
        burn::tensor::TensorPrimitive::Float(c) => c,
        _ => anyhow::bail!("alloc_hwc4_input: expected Float tensor primitive"),
    };
    let managed = cube
        .client
        .get_resource(cube.handle.clone())
        .map_err(|e| anyhow::anyhow!("OIDN get_resource: {e:?}"))?;
    let res = managed.resource();
    Ok((t, res.buffer.clone(), res.offset))
}

/// Convert a `[1, H, W, 4]` HWC RGBA Burn tensor into a `[1, 3, H, W]`
/// CHW RGB view. Slice trims off the alpha channel; permute swaps axes
/// without copying. The result is non-contiguous (view); downstream
/// `run_tensors` ops (`slice`, `reflect_pad_2d`, `cat`, ...) handle
/// strides correctly.
fn hwc4_to_chw3(
    hwc4: burn::tensor::Tensor<burn_wgpu::Wgpu<f32, i32>, 4>,
) -> burn::tensor::Tensor<burn_wgpu::Wgpu<f32, i32>, 4> {
    let dims = hwc4.dims();
    debug_assert_eq!(dims[0], 1);
    debug_assert_eq!(dims[3], 4);
    let h = dims[1];
    let w = dims[2];
    // [1, H, W, 4] -> [1, H, W, 3] (drop alpha)
    let hwc3 = hwc4.slice([0..1, 0..h, 0..w, 0..3]);
    // [1, H, W, 3] -> [1, 3, H, W]
    hwc3.permute([0, 3, 1, 2])
}

/// Convert a `[1, 3, H, W]` CHW RGB Burn tensor into a contiguous
/// `[1, H, W, 4]` HWC RGBA tensor with alpha=1, on the same device.
///
/// Uses `permute` + `cat` so the conversion stays on-device — no host
/// roundtrip. The result is contiguous (cat materialises a new tensor),
/// which is what the subsequent `copy_buffer_to_texture` needs.
fn chw_rgb_to_hwc_rgba_ones(
    chw3: burn::tensor::Tensor<burn_wgpu::Wgpu<f32, i32>, 4>,
    device: &burn_wgpu::WgpuDevice,
) -> burn::tensor::Tensor<burn_wgpu::Wgpu<f32, i32>, 4> {
    let dims = chw3.dims();
    debug_assert_eq!(dims[0], 1);
    debug_assert_eq!(dims[1], 3);
    let h = dims[2];
    let w = dims[3];
    // [1, 3, H, W] → [1, H, W, 3]
    let hwc3 = chw3.permute([0, 2, 3, 1]);
    let ones = burn::tensor::Tensor::<burn_wgpu::Wgpu<f32, i32>, 4>::ones([1, h, w, 1], device);
    burn::tensor::Tensor::cat(vec![hwc3, ones], 3)
}

/// Extract the underlying `wgpu::Buffer` of a Burn tensor (allocated on
/// the shared wgpu device), then issue a single `copy_buffer_to_texture`
/// into `dst`. `dst` must be `Rgba32Float` with the same `(w, h)` shape
/// the tensor was built for.
///
/// Width restriction: `w * 16` must be a multiple of 256 (wgpu's
/// `COPY_BYTES_PER_ROW_ALIGNMENT`). Common viewport widths satisfy this
/// (any multiple of 16 px). Otherwise we'd need an intermediate
/// padded-row buffer; deferred until needed in practice.
fn copy_tensor_into_texture(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    hwc4: burn::tensor::Tensor<burn_wgpu::Wgpu<f32, i32>, 4>,
    dst: &wgpu::Texture,
    w: usize,
    h: usize,
) -> Result<()> {
    let bytes_per_row = (w * 16) as u32;
    if bytes_per_row % 256 != 0 {
        anyhow::bail!(
            "OIDN output texture copy requires width*16 to be 256-byte aligned (got w={w}, bpr={bytes_per_row})"
        );
    }

    // Pull the inner CubeTensor so we can reach its ComputeClient + Handle.
    let primitive = hwc4.into_primitive();
    let cube = match primitive {
        burn::tensor::TensorPrimitive::Float(c) => c,
        _ => anyhow::bail!("OIDN bridge: expected Float tensor primitive"),
    };
    let managed = cube
        .client
        .get_resource(cube.handle.clone())
        .map_err(|e| anyhow::anyhow!("OIDN get_resource: {e:?}"))?;
    let res = managed.resource();

    let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
        label: Some("oidn_tensor_to_result_texture"),
    });
    encoder.copy_buffer_to_texture(
        wgpu::TexelCopyBufferInfo {
            buffer: &res.buffer,
            layout: wgpu::TexelCopyBufferLayout {
                offset: res.offset,
                bytes_per_row: Some(bytes_per_row),
                rows_per_image: Some(h as u32),
            },
        },
        wgpu::TexelCopyTextureInfo {
            texture: dst,
            mip_level: 0,
            origin: wgpu::Origin3d::ZERO,
            aspect: wgpu::TextureAspect::All,
        },
        wgpu::Extent3d {
            width: w as u32,
            height: h as u32,
            depth_or_array_layers: 1,
        },
    );
    queue.submit(std::iter::once(encoder.finish()));
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mode_aov_requirements() {
        assert_eq!(OidnMode::Off.requires_aov(), (false, false));
        assert_eq!(OidnMode::Color.requires_aov(), (false, false));
        assert_eq!(OidnMode::ColorAlbedo.requires_aov(), (true, false));
        assert_eq!(OidnMode::ColorAlbedoNormal.requires_aov(), (true, true));
    }
}
