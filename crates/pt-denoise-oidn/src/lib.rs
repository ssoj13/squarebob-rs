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

use std::path::{Path, PathBuf};
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
    /// Optional filesystem fallback for TZA blobs not baked into the
    /// binary by an `embed-*` feature on `oidn-rs`. `None` means we run
    /// embed-only — fine for the standard HDR modes since `embed-hdr`
    /// covers them all; required if the user wants `clean_aux`,
    /// `lightmap`, or LDR modes.
    weights_dir: Option<PathBuf>,

    mode: OidnMode,
    quality: Quality,
    width: u32,
    height: u32,

    /// Linear `Rgba32Float` result texture, same dims as input. Allocated
    /// up-front in `new()` and replaced on `resize()` so we never observe a
    /// half-initialised denoiser at run-time.
    result_texture: wgpu::Texture,
    result_view: wgpu::TextureView,

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
    /// `denoise()` to the next, so the ~30-50 ms `commit()` cost (TZA parse,
    /// UNet weight load, tile-plan compute) is paid once per
    /// mode/quality/dims combination, not every periodic fire.
    cached_filter:
        Option<Box<oidn_rs::RtFilter<'static, burn_wgpu::Wgpu<f32, i32>>>>,
    cached_filter_key: Option<(bool, bool, Quality, u32, u32)>,

    /// Per-channel HDR clamp applied to the colour input tensor before
    /// it reaches the UNet. `0.0` (or non-finite) disables clamping;
    /// any positive value caps each `f32` colour channel to that value.
    /// Set by callers via [`Self::set_input_clamp`]. Lives only on the
    /// OIDN input path — the underlying PT accumulator is untouched, so
    /// the raw display stays physically correct.
    input_clamp: f32,
}

impl OidnDenoiser {
    pub fn new(
        ctx: &GpuContext,
        width: u32,
        height: u32,
        weights_dir: Option<PathBuf>,
    ) -> Self {
        let (result_texture, result_view) = create_result_texture(&ctx.device, width, height);
        Self {
            weights_dir,
            mode: OidnMode::default(),
            quality: Quality::Balanced,
            width,
            height,
            result_texture,
            result_view,
            last_latency_ms: None,
            burn_device_ref: None,
            cached_model_key: None,
            cached_model_bytes: None,
            cached_filter: None,
            cached_filter_key: None,
            input_clamp: 0.0,
        }
    }

    /// Set the per-channel HDR firefly clamp applied to the colour
    /// input before OIDN. `0.0` or non-finite disables. See the
    /// `input_clamp` field doc for rationale.
    pub fn set_input_clamp(&mut self, max: f32) {
        self.input_clamp = max;
    }

    pub fn resize(&mut self, ctx: &GpuContext, width: u32, height: u32) {
        if self.width == width && self.height == height {
            return;
        }
        self.width = width;
        self.height = height;
        let (tex, view) = create_result_texture(&ctx.device, width, height);
        self.result_texture = tex;
        self.result_view = view;
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

    pub fn result_view(&self) -> &wgpu::TextureView {
        &self.result_view
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

        // Lazy init: Burn device (fallible — `make_burn_device` returns
        // `Result`, so it can't run in `new`). The device is built once and
        // leaked to `'static` so the cached `RtFilter` (parameterised over
        // `&'b WgpuDevice`) can survive across denoise calls — saves the
        // UNet rebuild on every periodic fire. The result texture itself
        // is allocated up-front in `new()`, so there's no separate slot
        // to ensure here.
        let burn_device: &'static burn_wgpu::WgpuDevice = match self.burn_device_ref {
            Some(d) => d,
            None => {
                let dev = make_burn_device(ctx)?;
                let leaked: &'static burn_wgpu::WgpuDevice = Box::leak(Box::new(dev));
                self.burn_device_ref = Some(leaked);
                log::info!("OIDN: Burn-wgpu device initialised on shared wgpu setup (leaked to 'static for filter caching)");
                leaked
            }
        };

        let started = Instant::now();
        // From here on use `effective_mode` rather than `self.mode` so the
        // model picker and AOV reads stay consistent with the downgrade.
        let w = self.width as usize;
        let h = self.height as usize;
        let n = w * h;

        // I.5b input bridge: copy PT-side data directly into Burn-allocated
        // wgpu::Buffers, then wrap as on-device tensors. No host roundtrip.
        //
        // `copy_texture_to_buffer` requires `bytes_per_row` to be a multiple
        // of 256. When `w * 16` already satisfies this, we use the tight
        // path. Otherwise we allocate `[1, h, padded_w, 4]` (rounded up so
        // `padded_w * 16` is 256-aligned), copy with the padded stride, and
        // slice off the trailing padding columns on the Burn side before
        // feeding the filter. The slice is a strided view — downstream
        // ops handle it without an explicit `.contiguous()` call.
        let unpadded_bpr = (w as u64) * 16;
        let padded_bpr = (unpadded_bpr + 255) & !255;
        let padded_w = (padded_bpr / 16) as usize;
        let pad_cols = padded_w - w;

        // Respect downgrade: drop AOV inputs we don't intend to consume so
        // we don't allocate / copy ~30 MB of AOV per side just to throw away.
        let albedo_buf = if use_albedo { albedo_buf } else { None };
        let normal_buf = if use_normal { normal_buf } else { None };

        // Color uses `padded_w` so `copy_texture_to_buffer` is happy.
        // AOVs are already tight `vec4<f32>` buffers — no alignment
        // constraint on `copy_buffer_to_buffer`, so they get the natural
        // [1, h, w, 4] shape.
        use burn::tensor::Tensor;
        type Wgpu = burn_wgpu::Wgpu<f32, i32>;
        let (color_hwc4_padded, color_buf, color_off) =
            alloc_hwc4_input(burn_device, padded_w, h)?;
        let albedo_bridge = albedo_buf.map(|src| -> Result<_> {
            let (t, buf, off) = alloc_hwc4_input(burn_device, w, h)?;
            Ok((t, buf, off, src))
        }).transpose()?;
        let normal_bridge = normal_buf.map(|src| -> Result<_> {
            let (t, buf, off) = alloc_hwc4_input(burn_device, w, h)?;
            Ok((t, buf, off, src))
        }).transpose()?;

        let mut encoder = encoder;
        // Color: copy_texture_to_buffer with (possibly padded) row stride.
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
                    bytes_per_row: Some(padded_bpr as u32),
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
            "OIDN: input bridge submitted (color {} bytes padded@{} + 2×{} AOV)",
            padded_bpr * (h as u64), padded_w, aov_size,
        );

        // Trim padding columns from the colour tensor — yields a view of
        // shape [1, h, w, 4] (strided when `pad_cols > 0`).
        let color_hwc4: Tensor<Wgpu, 4> = if pad_cols == 0 {
            color_hwc4_padded
        } else {
            color_hwc4_padded.slice([0..1, 0..h, 0..w, 0..4])
        };

        // Firefly clamp before OIDN. The PT accumulator can carry rare
        // extreme samples that OIDN's albedo+normal-guided UNet keeps
        // as "high-frequency content", smearing each spike into a halo
        // and producing splotchy noise that grows with samples. Capping
        // each channel here suppresses the spikes without touching the
        // raw PT image displayed when the denoiser is off. `0.0` (or
        // non-finite) means disabled — handy if the caller wants a
        // physically uncapped run.
        let color_hwc4 = if self.input_clamp.is_finite() && self.input_clamp > 0.0 {
            color_hwc4.clamp_max(self.input_clamp)
        } else {
            color_hwc4
        };

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
                // Delegate weight discovery to oidn-rs: tries the embed-hdr
                // blob baked into the binary first, then `weights_dir`
                // (resolved via env / exe-relative / cwd; missing dir is
                // fine — embedded path covers the standard modes alone).
                let base_key = oidn_rs::registry::select_rt(
                    true, use_albedo, use_normal,
                    /*hdr*/ true, /*srgb*/ false, /*directional*/ false,
                    /*clean_aux*/ false, self.quality,
                );
                let fallback_dir = self.weights_dir.as_deref();
                let loaded = base_key.as_ref().and_then(|key| {
                    oidn_rs::weights::resolve(key, self.quality, fallback_dir)
                });
                if let Some((stem, b)) = loaded {
                    log::debug!("OIDN: weights resolved stem={} ({} bytes)", stem, b.len());
                    self.cached_model_bytes = Some(b.clone());
                    self.cached_model_key = Some(cache_key);
                    Some(b)
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
            // weights_dir is only consulted by RtFilter::commit when the
            // builder wasn't given pre-loaded bytes via `.weights(...)`.
            // We always pass cached bytes below (resolved via
            // `oidn_rs::weights::resolve`), so the path here is a
            // formality — empty string keeps the type happy without
            // requiring the dir to exist.
            let weights_dir_placeholder: &Path = self
                .weights_dir
                .as_deref()
                .unwrap_or(Path::new(""));
            let mut builder = oidn_rs::RtFilter::<burn_wgpu::Wgpu<f32, i32>>::builder(
                burn_device,
                weights_dir_placeholder,
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
        // Pad width axis if needed so the final buffer's bytes_per_row
        // is 256-aligned for copy_buffer_to_texture. extent.width = w
        // skips the padding columns when writing to the result texture.
        let out_hwc4_padded: Tensor<Wgpu, 4> = if pad_cols == 0 {
            out_hwc4
        } else {
            let zeros = Tensor::<Wgpu, 4>::zeros([1, h, pad_cols, 4], burn_device);
            Tensor::cat(vec![out_hwc4, zeros], 2)
        };
        copy_tensor_into_texture(&ctx.device, &ctx.queue, out_hwc4_padded, &self.result_texture, w, h)?;

        // Drain all GPU work this pass submitted before returning.
        //
        // Why: CubeCL submits kernels lazily and the wgpu buffer pool
        // recycles buffers as soon as their owning Tensor is dropped.
        // Between successive `denoise()` calls, that recycling can
        // race the previous pass's UNet reads — the buffer leaves the
        // pool, gets a fresh `Tensor::zeros` fill on the next call,
        // and the still-pending UNet kernel can witness the zero
        // overwrite. The visible symptom is speckle that grows with
        // each denoise (each pass eats a partially-corrupted input).
        //
        // The matching upstream change in `oidn-rs::RtFilter::execute`
        // clears the cached input tensors so the caller's buffers
        // can be recycled immediately; this poll closes the GPU-side
        // of the same race by guaranteeing the kernels reading those
        // buffers have completed before we return. Cost ~ a few ms
        // per pass, which is fine because OIDN already runs once
        // every `pt_oidn_interval` samples.
        let _ = ctx.device.poll(wgpu::PollType::wait_indefinitely());

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
/// `(burn tensor view, the underlying wgpu storage buffer, byte offset into it)`.
/// Helper return type for [`alloc_hwc4_input`]; the wgpu side carries enough
/// context for the caller to issue a direct `copy_buffer_to_buffer` into the
/// tensor's backing storage without going through CubeCL.
type HwC4Input = (
    burn::tensor::Tensor<burn_wgpu::Wgpu<f32, i32>, 4>,
    wgpu::Buffer,
    u64,
);

fn alloc_hwc4_input(
    device: &burn_wgpu::WgpuDevice,
    w: usize,
    h: usize,
) -> Result<HwC4Input> {
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
/// into `dst`. `dst` must be `Rgba32Float`.
///
/// The tensor's third axis is treated as the row stride in pixels
/// (`padded_w`); `bytes_per_row = padded_w * 16` must be 256-aligned
/// (wgpu's `COPY_BYTES_PER_ROW_ALIGNMENT`). `extent.width` is the
/// caller-supplied `valid_w` — when the tensor was padded for alignment
/// we copy only the first `valid_w` pixels per row and the trailing
/// padding columns are discarded.
fn copy_tensor_into_texture(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    hwc4: burn::tensor::Tensor<burn_wgpu::Wgpu<f32, i32>, 4>,
    dst: &wgpu::Texture,
    valid_w: usize,
    h: usize,
) -> Result<()> {
    let dims = hwc4.dims();
    debug_assert_eq!(dims[0], 1);
    debug_assert_eq!(dims[1], h);
    debug_assert_eq!(dims[3], 4);
    let padded_w = dims[2];
    debug_assert!(padded_w >= valid_w, "output tensor must contain all valid pixels");
    let bytes_per_row = (padded_w * 16) as u32;
    if !bytes_per_row.is_multiple_of(256) {
        anyhow::bail!(
            "OIDN output buffer bytes_per_row not 256-aligned (padded_w={padded_w}, bpr={bytes_per_row})"
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
            width: valid_w as u32,
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
