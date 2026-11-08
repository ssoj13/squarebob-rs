//! Environment map loading and management.
//!
//! Supports HDR/LDR images (PNG/JPG/HDR/EXR) via `image` + `exr-image` (`exr-core` hooks, never
//! crates.io `exr`), delegating sRGB EOTF linearization and spherical `sin(theta)` importance-sampling
//! CDF construction to `render_env` (`EnvCdfs`, `EnvFormat`, `srgb_rgba8_to_linear`).

use image::{DynamicImage, GenericImageView, ImageReader};
use log::info;
use render_core::gpu::GpuContext;
use render_env::{EnvCdfs, EnvFormat, srgb_rgba8_to_linear};

/// Environment map state (texture + sampler)
pub struct EnvMap {
    pub texture: wgpu::Texture,
    pub view: wgpu::TextureView,
    pub sampler: wgpu::Sampler,
    pub marginal_cdf: wgpu::Buffer,
    pub conditional_cdf: wgpu::Buffer,
    pub marginal_cdf_data: Vec<f32>,
    pub conditional_cdf_data: Vec<f32>,
    pub texels_rgb: Vec<[f32; 3]>,
    pub width: u32,
    pub height: u32,
}

impl EnvMap {
    /// Create with a default 1x1 grey placeholder
    pub fn new_default(ctx: &GpuContext) -> Self {
        let texture = ctx.device.create_texture(&wgpu::TextureDescriptor {
            label: Some("Default Env Map"),
            size: wgpu::Extent3d {
                width: 1,
                height: 1,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::Rgba16Float,
            usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
            view_formats: &[],
        });
        ctx.queue.write_texture(
            wgpu::TexelCopyTextureInfo {
                texture: &texture,
                mip_level: 0,
                origin: wgpu::Origin3d::ZERO,
                aspect: wgpu::TextureAspect::All,
            },
            &[0x00, 0x38, 0x00, 0x38, 0x00, 0x38, 0x00, 0x3c],
            wgpu::TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(8),
                rows_per_image: Some(1),
            },
            wgpu::Extent3d {
                width: 1,
                height: 1,
                depth_or_array_layers: 1,
            },
        );

        let view = texture.create_view(&Default::default());
        let sampler = ctx.device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("Env Sampler"),
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            mipmap_filter: wgpu::MipmapFilterMode::Linear,
            address_mode_u: wgpu::AddressMode::Repeat,
            address_mode_v: wgpu::AddressMode::ClampToEdge,
            ..Default::default()
        });

        let marginal_cdf = render_core::gpu::make_buffer_init(
            &ctx.device,
            "default_env_marginal_cdf",
            bytemuck::cast_slice(&[1.0f32]),
            wgpu::BufferUsages::STORAGE,
        );
        let conditional_cdf = render_core::gpu::make_buffer_init(
            &ctx.device,
            "default_env_conditional_cdf",
            bytemuck::cast_slice(&[1.0f32]),
            wgpu::BufferUsages::STORAGE,
        );

        Self {
            texture,
            view,
            sampler,
            marginal_cdf,
            conditional_cdf,
            marginal_cdf_data: vec![1.0],
            conditional_cdf_data: vec![1.0],
            texels_rgb: vec![[0.5, 0.5, 0.5]],
            width: 1,
            height: 1,
        }
    }

    /// Load env map from an image file (PNG, JPG, HDR, EXR)
    /// Decode every supported format into a scene-linear Rgba16Float mip pyramid
    pub fn load_from_file(
        &mut self,
        ctx: &GpuContext,
        path: &std::path::Path,
    ) -> anyhow::Result<()> {
        let img = read_image(path)?;
        let (w, h) = img.dimensions();

        // Environment textures have one contract: scene-linear RGBA16F.
        // HDR decoders already return linear values. LDR decoders return
        // sRGB-encoded values, linearized via `render_env::srgb_rgba8_to_linear`
        // before mip generation, upload, and `render_env::EnvCdfs::build`.
        let is_hdr = EnvFormat::of(path).is_hdr();
        render_core::checked_texture_extent_2d(&ctx.device, "environment texture", w, h)?;
        let pixel_len = render_core::checked_2d_buffer_len("environment texture", w, h)?;

        let mut level0 = Vec::with_capacity(pixel_len);
        let mut texels_rgb = Vec::with_capacity(pixel_len);
        if is_hdr {
            for pixel in img.to_rgba32f().pixels() {
                let rgb = [
                    sanitize_radiance(pixel[0]),
                    sanitize_radiance(pixel[1]),
                    sanitize_radiance(pixel[2]),
                ];
                let alpha = if pixel[3].is_finite() {
                    pixel[3].clamp(0.0, 1.0)
                } else {
                    1.0
                };
                level0.push([rgb[0], rgb[1], rgb[2], alpha]);
                texels_rgb.push(rgb);
            }
        } else {
            for pixel in img.to_rgba8().pixels() {
                let lin = srgb_rgba8_to_linear(pixel.0);
                level0.push(lin);
                texels_rgb.push([lin[0], lin[1], lin[2]]);
            }
        }
        debug_assert_eq!(level0.len(), pixel_len);
        debug_assert_eq!(texels_rgb.len(), pixel_len);

        let pyramid = build_linear_mip_pyramid(w, h, level0);
        let mip_level_count = pyramid.len() as u32;
        let format = wgpu::TextureFormat::Rgba16Float;

        let texture = ctx.device.create_texture(&wgpu::TextureDescriptor {
            label: Some("Env Map"),
            size: wgpu::Extent3d {
                width: w,
                height: h,
                depth_or_array_layers: 1,
            },
            mip_level_count,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format,
            usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
            view_formats: &[],
        });

        let mut mip_bytes = Vec::new();
        for (mip_idx, mip) in pyramid.iter().enumerate() {
            let byte_len =
                render_core::checked_2d_byte_len("environment mip", mip.width, mip.height, 8)?;
            let bytes_per_row = u32::try_from(render_core::checked_2d_buffer_size(
                "environment texture row",
                mip.width,
                1,
                8,
            )?)
            .map_err(|_| anyhow::anyhow!("environment texture row pitch exceeds u32"))?;
            mip_bytes.clear();
            if mip_bytes.capacity() < byte_len {
                mip_bytes.reserve(byte_len - mip_bytes.capacity());
            }
            for &rgba in &mip.texels {
                push_rgba16f(&mut mip_bytes, [rgba[0], rgba[1], rgba[2]], rgba[3]);
            }
            ctx.queue.write_texture(
                wgpu::TexelCopyTextureInfo {
                    texture: &texture,
                    mip_level: mip_idx as u32,
                    origin: wgpu::Origin3d::ZERO,
                    aspect: wgpu::TextureAspect::All,
                },
                &mip_bytes,
                wgpu::TexelCopyBufferLayout {
                    offset: 0,
                    bytes_per_row: Some(bytes_per_row),
                    rows_per_image: Some(mip.height),
                },
                wgpu::Extent3d {
                    width: mip.width,
                    height: mip.height,
                    depth_or_array_layers: 1,
                },
            );
        }

        self.view = texture.create_view(&Default::default());
        self.texture = texture;
        self.width = w;
        self.height = h;

        let (conditional_cdf_data, marginal_cdf_data) = build_env_cdfs(w, h, &texels_rgb)?;
        self.conditional_cdf_data = conditional_cdf_data;
        self.marginal_cdf_data = marginal_cdf_data;
        self.texels_rgb = texels_rgb;
        self.conditional_cdf = render_core::gpu::make_buffer_init(
            &ctx.device,
            "env_conditional_cdf",
            bytemuck::cast_slice(&self.conditional_cdf_data),
            wgpu::BufferUsages::STORAGE,
        );
        self.marginal_cdf = render_core::gpu::make_buffer_init(
            &ctx.device,
            "env_marginal_cdf",
            bytemuck::cast_slice(&self.marginal_cdf_data),
            wgpu::BufferUsages::STORAGE,
        );

        info!(
            "Loaded env map: {}x{} ({} mips) {:?} from {:?}",
            w, h, mip_level_count, format, path
        );
        Ok(())
    }
}

/// Decode an environment image by its extension. EXR goes through exr-core (`exr_image`'s hooks on
/// `ImageReader`), every other format through `image`. Do not force a format with `set_format`:
/// an explicit `ImageFormat::OpenExr` bypasses the hooks and would need crates.io `exr`.
fn read_image(path: &std::path::Path) -> anyhow::Result<DynamicImage> {
    exr_image::register();
    Ok(ImageReader::open(path)?.decode()?)
}

fn sanitize_radiance(value: f32) -> f32 {
    if value.is_finite() {
        value.max(0.0)
    } else {
        0.0
    }
}

fn push_rgba16f(data: &mut Vec<u8>, rgb: [f32; 3], alpha: f32) {
    for value in [rgb[0], rgb[1], rgb[2], alpha] {
        data.extend_from_slice(&half::f16::from_f32(value).to_le_bytes());
    }
}

#[derive(Clone, Debug)]
struct LinearMip {
    width: u32,
    height: u32,
    texels: Vec<[f32; 4]>,
}

#[inline]
fn mip_level_count_for(width: u32, height: u32) -> u32 {
    let max_dim = width.max(height);
    if max_dim <= 1 { 1 } else { max_dim.ilog2() + 1 }
}

/// Downsample `prev` (`pw x ph`) to `wgpu` subresource extent `(nw x nh)` using spherical
/// `sin(theta)` solid-angle row weights, wrapping azimuth columns and clamping polar rows.
fn downsample_solid_angle(
    prev: &[[f32; 4]],
    pw: usize,
    ph: usize,
    nw: usize,
    nh: usize,
) -> Vec<[f32; 4]> {
    let inv_ph = 1.0f32 / (ph as f32);
    let mut out = Vec::with_capacity(nw * nh);
    for y in 0..nh {
        let ys = (y * ph) / nh;
        let ye = (((y + 1) * ph) / nh).max(ys + 1).min(ph);
        let mut wy_sum = 0.0f64;
        for sy in ys..ye {
            let theta = ((sy as f32 + 0.5) * inv_ph) * std::f32::consts::PI;
            wy_sum += f64::from(theta.sin().max(1e-6));
        }
        for x in 0..nw {
            let xs = (x * pw) / nw;
            let xe = (((x + 1) * pw) / nw).max(xs + 1);
            let inv_norm = 1.0 / (wy_sum * ((xe - xs) as f64));
            let mut acc = [0.0f64; 4];
            for sy in ys..ye {
                let theta = ((sy as f32 + 0.5) * inv_ph) * std::f32::consts::PI;
                let wy = f64::from(theta.sin().max(1e-6));
                let row = &prev[sy * pw..(sy + 1) * pw];
                for sx in xs..xe {
                    let px = row[sx % pw];
                    acc[0] += f64::from(px[0]) * wy;
                    acc[1] += f64::from(px[1]) * wy;
                    acc[2] += f64::from(px[2]) * wy;
                    acc[3] += f64::from(px[3]) * wy;
                }
            }
            out.push([
                (acc[0] * inv_norm).clamp(0.0, f64::from(f32::MAX)) as f32,
                (acc[1] * inv_norm).clamp(0.0, f64::from(f32::MAX)) as f32,
                (acc[2] * inv_norm).clamp(0.0, f64::from(f32::MAX)) as f32,
                (acc[3] * inv_norm).clamp(0.0, 1.0) as f32,
            ]);
        }
    }
    out
}

fn build_linear_mip_pyramid(width: u32, height: u32, level0: Vec<[f32; 4]>) -> Vec<LinearMip> {
    let mip_count = mip_level_count_for(width, height);
    let mut levels = Vec::with_capacity(mip_count as usize);
    levels.push(LinearMip {
        width,
        height,
        texels: level0,
    });
    for level in 1..mip_count {
        let nw = (width >> level).max(1);
        let nh = (height >> level).max(1);
        let prev = &levels[(level - 1) as usize];
        let texels = downsample_solid_angle(
            &prev.texels,
            prev.width as usize,
            prev.height as usize,
            nw as usize,
            nh as usize,
        );
        levels.push(LinearMip {
            width: nw,
            height: nh,
            texels,
        });
    }
    levels
}

/// Build row-conditional and marginal importance-sampling CDFs via [`render_env::EnvCdfs::build`],
/// normalizing degenerate zero-energy rows/maps to uniform distributions so shader binary search
/// always receives monotonic `[1/N .. 1.0]` tables.
fn build_env_cdfs(
    width: u32,
    height: u32,
    texels_rgb: &[[f32; 3]],
) -> anyhow::Result<(Vec<f32>, Vec<f32>)> {
    let pixel_count = render_core::checked_2d_buffer_len("environment CDF", width, height)?;
    anyhow::ensure!(
        texels_rgb.len() == pixel_count,
        "environment texel length mismatch: expected {pixel_count}, got {}",
        texels_rgb.len()
    );
    let w =
        usize::try_from(width).map_err(|_| anyhow::anyhow!("environment width exceeds usize"))?;
    let h =
        usize::try_from(height).map_err(|_| anyhow::anyhow!("environment height exceeds usize"))?;

    let mut cdfs = EnvCdfs::build(width, height, texels_rgb);
    for cond_row in cdfs.conditional.chunks_exact_mut(w) {
        if !cond_row
            .last()
            .copied()
            .is_some_and(|end| end > 0.0 && end.is_finite())
        {
            for (x, slot) in cond_row.iter_mut().enumerate() {
                *slot = (x + 1) as f32 / w as f32;
            }
        }
    }
    if !cdfs
        .marginal
        .last()
        .copied()
        .is_some_and(|end| end > 0.0 && end.is_finite())
    {
        for (y, slot) in cdfs.marginal.iter_mut().enumerate() {
            *slot = (y + 1) as f32 / h as f32;
        }
    }

    Ok((cdfs.conditional, cdfs.marginal))
}

#[cfg(test)]
mod tests {
    use super::*;
    use render_env::srgb_to_linear;

    #[test]
    fn srgb_reference_values_are_linearized() {
        assert_eq!(srgb_to_linear(0.0), 0.0);
        assert!((srgb_to_linear(0.04045) - 0.003130805).abs() < 1.0e-7);
        assert!((srgb_to_linear(0.5) - 0.21404114).abs() < 1.0e-7);
        assert_eq!(srgb_to_linear(1.0), 1.0);
    }

    #[test]
    fn black_environment_gets_uniform_valid_cdfs() {
        let (conditional, marginal) = build_env_cdfs(2, 2, &[[0.0; 3]; 4]).unwrap();
        assert_eq!(conditional, vec![0.5, 1.0, 0.5, 1.0]);
        assert_eq!(marginal, vec![0.5, 1.0]);
    }

    #[test]
    fn cdf_rejects_mismatched_luminance() {
        assert!(build_env_cdfs(2, 2, &[[1.0; 3]; 3]).is_err());
    }

    #[test]
    fn npot_mip_pyramid_matches_wgpu_subresource_extents() {
        let npot = build_linear_mip_pyramid(5, 3, vec![[2.0, 1.0, 0.5, 1.0]; 15]);
        assert_eq!(npot.len(), 3);
        assert_eq!((npot[0].width, npot[0].height), (5, 3));
        assert_eq!((npot[1].width, npot[1].height), (2, 1));
        assert_eq!((npot[2].width, npot[2].height), (1, 1));
    }

    /// An EXR environment (any extension casing) decodes through exr-core with radiance above 1
    /// intact — the HDR path `load_from_file` relies on, without crates.io `exr`.
    #[test]
    fn exr_environment_decodes_through_exr_core() {
        use exr_core::attr::Compression;
        use exr_core::{ChannelData, Image};
        use imath_rs::{Box2i, V2i};

        let dir = std::env::temp_dir().join(format!("render-3d-exr-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let window = Box2i { min: V2i { x: 0, y: 0 }, max: V2i { x: 1, y: 0 } };
        let image = Image::new(window)
            .with_channel("R", ChannelData::Float(vec![4.0, 0.25]))
            .with_channel("G", ChannelData::Float(vec![2.0, 0.5]))
            .with_channel("B", ChannelData::Float(vec![1.0, 19.5]));
        for name in ["sky.exr", "SKY.EXR"] {
            let path = dir.join(name);
            image.write(&path, Compression::Zip).unwrap();
            let img = read_image(&path).unwrap();
            assert_eq!(img.dimensions(), (2, 1));
            let px = img.to_rgba32f();
            assert_eq!(px.get_pixel(0, 0).0, [4.0, 2.0, 1.0, 1.0], "{name}");
            assert_eq!(px.get_pixel(1, 0).0, [0.25, 0.5, 19.5, 1.0], "{name}");
        }
        std::fs::remove_dir_all(&dir).unwrap();
    }
}
