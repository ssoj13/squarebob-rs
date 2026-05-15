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
    burn_device: Option<burn_wgpu::WgpuDevice>,

    /// Cached TZA bytes from the last successful model load. Reused across
    /// `denoise()` calls so we don't re-read the 1.8 MB file from disk
    /// on every interval-fire. Key = (use_albedo, use_normal, quality)
    /// since these fully determine which TZA file gets picked.
    cached_model_key: Option<(bool, bool, Quality)>,
    cached_model_bytes: Option<Vec<u8>>,

    /// Reused staging buffers for color readback (padded to 256-byte rows)
    /// and the two AOVs (tight `w*h*16`). Invalidated on resize.
    color_staging: Option<wgpu::Buffer>,
    color_staging_size: u64,
    aov_staging_size: u64,
    albedo_staging: Option<wgpu::Buffer>,
    normal_staging: Option<wgpu::Buffer>,
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
            cached_model_key: None,
            cached_model_bytes: None,
            color_staging: None,
            color_staging_size: 0,
            aov_staging_size: 0,
            albedo_staging: None,
            normal_staging: None,
            burn_device: None,
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
        // Staging dims change with viewport; drop reused buffers.
        self.color_staging = None;
        self.albedo_staging = None;
        self.normal_staging = None;
        self.color_staging_size = 0;
        self.aov_staging_size = 0;
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

        // Lazy init: Burn device + result texture.
        if self.burn_device.is_none() {
            self.burn_device = Some(make_burn_device(ctx)?);
            log::info!("OIDN: Burn-wgpu device initialised on shared wgpu setup");
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

        // Color readback. PT output is Rgba32Float = 16 bytes/pixel. We allocate
        // an aligned staging buffer (256-byte row pitch as required by wgpu's
        // `copy_texture_to_buffer`), then map and tightly repack into f32x4
        // before stripping alpha.
        let bpp = 16u64;
        let unpadded_bpr = (w as u64) * bpp;
        let padded_bpr = (unpadded_bpr + 255) & !255;
        let color_size = padded_bpr * (h as u64);
        if self.color_staging_size != color_size {
            self.color_staging = None;
            self.color_staging_size = color_size;
        }
        let color_staging = self.color_staging.get_or_insert_with(|| {
            ctx.device.create_buffer(&wgpu::BufferDescriptor {
                label: Some("oidn_color_staging"),
                size: color_size,
                usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
                mapped_at_creation: false,
            })
        });

        let mut encoder = encoder;
        encoder.copy_texture_to_buffer(
            wgpu::TexelCopyTextureInfo {
                texture: color_tex,
                mip_level: 0,
                origin: wgpu::Origin3d::ZERO,
                aspect: wgpu::TextureAspect::All,
            },
            wgpu::TexelCopyBufferInfo {
                buffer: color_staging,
                layout: wgpu::TexelCopyBufferLayout {
                    offset: 0,
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

        // AOV buffer readback: source buffers are already vec4<f32>, tightly
        // packed at full_w*full_h*16 bytes — no padding needed. We copy them
        // into mappable staging buffers regardless, because the source buffers
        // are not MAP_READ.
        // Respect downgrade: drop AOV inputs we don't intend to consume so
        // we don't copy 32 MB of normals just to throw them away.
        let albedo_buf = if use_albedo { albedo_buf } else { None };
        let normal_buf = if use_normal { normal_buf } else { None };

        let aov_size = (n as u64) * 16;
        if self.aov_staging_size != aov_size {
            self.albedo_staging = None;
            self.normal_staging = None;
            self.aov_staging_size = aov_size;
        }
        let albedo_staging_ref = if let Some(src) = albedo_buf {
            let buf = self.albedo_staging.get_or_insert_with(|| {
                ctx.device.create_buffer(&wgpu::BufferDescriptor {
                    label: Some("oidn_albedo_staging"),
                    size: aov_size,
                    usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
                    mapped_at_creation: false,
                })
            });
            encoder.copy_buffer_to_buffer(src, 0, buf, 0, aov_size);
            Some(buf)
        } else {
            None
        };
        let normal_staging_ref = if let Some(src) = normal_buf {
            let buf = self.normal_staging.get_or_insert_with(|| {
                ctx.device.create_buffer(&wgpu::BufferDescriptor {
                    label: Some("oidn_normal_staging"),
                    size: aov_size,
                    usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
                    mapped_at_creation: false,
                })
            });
            encoder.copy_buffer_to_buffer(src, 0, buf, 0, aov_size);
            Some(buf)
        } else {
            None
        };

        log::debug!(
            "OIDN: encoder built — color_size={} aov_size={} (albedo={}, normal={})",
            color_size, aov_size,
            albedo_staging_ref.is_some(), normal_staging_ref.is_some(),
        );
        // Re-borrow color_staging by index after the get_or_insert_with above
        // (the binding shadowed earlier) — staging buffers live in `self`
        // now and persist across denoise calls.
        let color_staging_ref: &wgpu::Buffer = self.color_staging.as_ref().unwrap();
        ctx.queue.submit(std::iter::once(encoder.finish()));
        log::trace!("OIDN: copy encoder submitted, mapping color staging");

        // Map everything and pull bytes back to host.
        let color_rgb = map_and_strip_rgba_padded(&ctx.device, color_staging_ref, w, h, padded_bpr)?;
        log::trace!("OIDN: color readback done ({} f32 = {} bytes)", color_rgb.len(), color_rgb.len() * 4);
        let albedo_rgb = albedo_staging_ref
            .map(|b| map_and_strip_rgba_tight(&ctx.device, b, n))
            .transpose()?;
        let normal_rgb = normal_staging_ref
            .map(|b| map_and_strip_rgba_tight(&ctx.device, b, n))
            .transpose()?;

        // Build the filter and run inference.
        let burn_device = self
            .burn_device
            .as_ref()
            .expect("burn_device init guaranteed above");
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

        log::debug!(
            "OIDN: building RtFilter (hdr={}, quality={:?}, input_scale_override={:?}, cached_weights={})",
            hdr, self.quality, user_scale, cached_bytes.is_some()
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
        if let Some(bytes) = cached_bytes {
            builder = builder.weights(bytes);
        }
        let mut filter = builder.build();

        // Sanity-trace the readback so it's obvious whether the input was
        // actually populated or we're feeding OIDN a blank frame.
        let in_stats = stats(&color_rgb);
        log::info!(
            "OIDN input color: min={:.4} max={:.4} mean={:.4} (n={} pixels)",
            in_stats.0, in_stats.1, in_stats.2, n
        );

        let color_img = oidn_rs::Image::from_rgb_f32(&color_rgb, w, h);
        filter.set_color(&color_img);
        if let Some(buf) = albedo_rgb.as_deref() {
            let img = oidn_rs::Image::from_rgb_f32(buf, w, h);
            filter.set_albedo(&img);
        }
        if let Some(buf) = normal_rgb.as_deref() {
            let img = oidn_rs::Image::from_rgb_f32(buf, w, h);
            filter.set_normal(&img);
        }
        filter.allocate_output(w, h, oidn_rs::PixelFormat::Rgb32f);
        log::trace!("OIDN: filter.commit() begin");
        filter.commit().map_err(|e| anyhow::anyhow!("OIDN commit: {e:?}"))?;
        log::debug!(
            "OIDN: filter committed (model={:?})",
            filter.model_key().map(|k| k.filename())
        );
        log::trace!("OIDN: filter.execute() begin");
        filter.execute().map_err(|e| anyhow::anyhow!("OIDN execute: {e:?}"))?;
        log::trace!("OIDN: filter.execute() done");
        let (out_bytes, ow, oh, ofmt) = filter
            .take_output()
            .ok_or_else(|| anyhow::anyhow!("OIDN take_output: empty"))?;
        log::debug!(
            "OIDN: take_output {}×{} fmt={:?} {} bytes (expected {} for Rgb32f)",
            ow, oh, ofmt, out_bytes.len(), n * 12
        );

        // Repack Rgb32f → Rgba16Float. result_texture is Rgba16Float so egui
        // can sample-filter it (Rgba32Float is unfilterable without an opt-in
        // device feature). 8 bytes/pixel; alpha pinned to 1.0.
        let out_f32: &[f32] = bytemuck::cast_slice(&out_bytes);
        debug_assert_eq!(out_f32.len(), n * 3);
        let out_stats = stats(out_f32);
        log::info!(
            "OIDN output color: min={:.4} max={:.4} mean={:.4}",
            out_stats.0, out_stats.1, out_stats.2
        );
        // Safety guard: if the network produced a fully black/NaN result,
        // don't claim "denoised" — let the caller keep showing the raw PT
        // output and surface a warning in the log so it's debuggable.
        if !out_stats.1.is_finite() || out_stats.1 < 1e-6 {
            anyhow::bail!(
                "OIDN produced an empty result (max={:.6}). Keeping raw display.",
                out_stats.1
            );
        }
        // Repack Rgb32f → Rgba32Float (alpha=1). Full f32 — display goes
        // through `blit_with_source` (ACES + gamma) so the texture stays
        // linear HDR right up to the screen.
        let mut rgba: Vec<f32> = Vec::with_capacity(n * 4);
        for px in out_f32.chunks_exact(3) {
            rgba.push(px[0]);
            rgba.push(px[1]);
            rgba.push(px[2]);
            rgba.push(1.0);
        }

        // Upload into the result texture. wgpu handles row padding for textures
        // larger than the unpadded width; our width is whatever the viewport is,
        // and `bytes_per_row` here is the *unpadded* row size — write_texture
        // computes the padded copy internally.
        let result_tex = self.result_texture.as_ref().unwrap();
        ctx.queue.write_texture(
            wgpu::TexelCopyTextureInfo {
                texture: result_tex,
                mip_level: 0,
                origin: wgpu::Origin3d::ZERO,
                aspect: wgpu::TextureAspect::All,
            },
            bytemuck::cast_slice(&rgba),
            wgpu::TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some((w * 16) as u32),
                rows_per_image: Some(self.height),
            },
            wgpu::Extent3d {
                width: self.width,
                height: self.height,
                depth_or_array_layers: 1,
            },
        );

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

/// Map a buffer that was filled from `copy_texture_to_buffer` (256-byte row
/// padding), then strip alpha to produce a tight `Vec<f32>` of length
/// `width * height * 3` in HWC order.
fn map_and_strip_rgba_padded(
    device: &wgpu::Device,
    buf: &wgpu::Buffer,
    width: usize,
    height: usize,
    padded_bpr: u64,
) -> Result<Vec<f32>> {
    let slice = buf.slice(..);
    let (tx, rx) = std::sync::mpsc::channel();
    slice.map_async(wgpu::MapMode::Read, move |r| {
        let _ = tx.send(r);
    });
    let _ = device.poll(wgpu::PollType::wait_indefinitely());
    rx.recv()
        .map_err(|e| anyhow::anyhow!("OIDN map_async channel: {e}"))?
        .map_err(|e| anyhow::anyhow!("OIDN map_async: {e:?}"))?;

    let mapped = slice.get_mapped_range();
    let mut out = Vec::with_capacity(width * height * 3);
    let padded = padded_bpr as usize;
    for y in 0..height {
        let row = &mapped[y * padded..y * padded + width * 16];
        let row_f32: &[f32] = bytemuck::cast_slice(row);
        for px in row_f32.chunks_exact(4) {
            out.push(px[0]);
            out.push(px[1]);
            out.push(px[2]);
        }
    }
    drop(mapped);
    buf.unmap();
    Ok(out)
}

/// Map a tightly-packed `vec4<f32>` storage buffer and strip alpha.
fn map_and_strip_rgba_tight(
    device: &wgpu::Device,
    buf: &wgpu::Buffer,
    n_pixels: usize,
) -> Result<Vec<f32>> {
    let slice = buf.slice(..);
    let (tx, rx) = std::sync::mpsc::channel();
    slice.map_async(wgpu::MapMode::Read, move |r| {
        let _ = tx.send(r);
    });
    let _ = device.poll(wgpu::PollType::wait_indefinitely());
    rx.recv()
        .map_err(|e| anyhow::anyhow!("OIDN map_async channel: {e}"))?
        .map_err(|e| anyhow::anyhow!("OIDN map_async: {e:?}"))?;

    let mapped = slice.get_mapped_range();
    let src: &[f32] = bytemuck::cast_slice(&mapped);
    debug_assert!(src.len() >= n_pixels * 4);
    let mut out = Vec::with_capacity(n_pixels * 3);
    for px in src.chunks_exact(4).take(n_pixels) {
        out.push(px[0]);
        out.push(px[1]);
        out.push(px[2]);
    }
    drop(mapped);
    buf.unmap();
    Ok(out)
}

/// Cheap min/max/mean over a flat HWC f32 slice — used to trace OIDN
/// input/output magnitudes when diagnosing a black-frame display.
fn stats(data: &[f32]) -> (f32, f32, f32) {
    if data.is_empty() {
        return (0.0, 0.0, 0.0);
    }
    let mut min = f32::INFINITY;
    let mut max = f32::NEG_INFINITY;
    let mut sum = 0.0f64;
    for &v in data {
        if v.is_finite() {
            if v < min {
                min = v;
            }
            if v > max {
                max = v;
            }
            sum += v as f64;
        }
    }
    (min, max, (sum / data.len() as f64) as f32)
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
