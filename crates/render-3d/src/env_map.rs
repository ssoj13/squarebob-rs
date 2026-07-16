//! Environment map loading and management
//! Supports HDR/LDR images (PNG/JPG/HDR/EXR) via the image crate

use image::{GenericImageView, ImageFormat, ImageReader};
use log::info;
use render_core::gpu::GpuContext;

/// Environment map state (texture + sampler)
pub struct EnvMap {
    pub texture: wgpu::Texture,
    pub view: wgpu::TextureView,
    pub sampler: wgpu::Sampler,
    pub marginal_cdf: wgpu::Buffer,
    pub conditional_cdf: wgpu::Buffer,
    pub marginal_cdf_data: Vec<f32>,
    pub conditional_cdf_data: Vec<f32>,
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
            width: 1,
            height: 1,
        }
    }

    /// Load env map from an image file (PNG, JPG, HDR, EXR)
    /// Decode every supported format into a scene-linear Rgba16Float texture
    pub fn load_from_file(
        &mut self,
        ctx: &GpuContext,
        path: &std::path::Path,
    ) -> anyhow::Result<()> {
        let ext = path
            .extension()
            .and_then(|e| e.to_str())
            .map(|e| e.to_lowercase());
        let mut reader = ImageReader::open(path)?;
        if let Some(e) = ext.as_deref() {
            match e {
                "hdr" => reader.set_format(ImageFormat::Hdr),
                "exr" => reader.set_format(ImageFormat::OpenExr),
                "png" => reader.set_format(ImageFormat::Png),
                "jpg" | "jpeg" => reader.set_format(ImageFormat::Jpeg),
                _ => {}
            }
        }
        let img = reader.decode()?;
        let (w, h) = img.dimensions();

        // Environment textures have one contract: scene-linear RGBA16F.
        // HDR decoders already return linear values. LDR decoders return
        // sRGB-encoded values, so decode them before upload and before building
        // the importance distribution.
        let is_hdr = ext.as_deref().is_some_and(|e| matches!(e, "hdr" | "exr"));
        let byte_len = render_core::checked_2d_byte_len("environment texture", w, h, 8)?;
        let pixel_len = render_core::checked_2d_buffer_len("environment texture", w, h)?;

        let mut data = Vec::with_capacity(byte_len);
        let mut luminance = Vec::with_capacity(pixel_len);
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
                push_rgba16f(&mut data, rgb, alpha);
                luminance.push(linear_luminance(rgb));
            }
        } else {
            for pixel in img.to_rgba8().pixels() {
                let rgb = [
                    srgb_to_linear(pixel[0] as f32 / 255.0),
                    srgb_to_linear(pixel[1] as f32 / 255.0),
                    srgb_to_linear(pixel[2] as f32 / 255.0),
                ];
                push_rgba16f(&mut data, rgb, pixel[3] as f32 / 255.0);
                luminance.push(linear_luminance(rgb));
            }
        }
        debug_assert_eq!(data.len(), byte_len);
        debug_assert_eq!(luminance.len(), pixel_len);
        let format = wgpu::TextureFormat::Rgba16Float;

        let texture = ctx.device.create_texture(&wgpu::TextureDescriptor {
            label: Some("Env Map"),
            size: wgpu::Extent3d {
                width: w,
                height: h,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format,
            usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
            view_formats: &[],
        });

        let bytes_per_row = u32::try_from(render_core::checked_2d_buffer_size(
            "environment texture row",
            w,
            1,
            8,
        )?)
        .map_err(|_| anyhow::anyhow!("environment texture row pitch exceeds u32"))?;
        ctx.queue.write_texture(
            wgpu::TexelCopyTextureInfo {
                texture: &texture,
                mip_level: 0,
                origin: wgpu::Origin3d::ZERO,
                aspect: wgpu::TextureAspect::All,
            },
            &data,
            wgpu::TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(bytes_per_row),
                rows_per_image: Some(h),
            },
            wgpu::Extent3d {
                width: w,
                height: h,
                depth_or_array_layers: 1,
            },
        );

        self.view = texture.create_view(&Default::default());
        self.texture = texture;
        self.width = w;
        self.height = h;

        let (conditional_cdf_data, marginal_cdf_data) = build_env_cdfs(w, h, &luminance)?;
        self.conditional_cdf_data = conditional_cdf_data;
        self.marginal_cdf_data = marginal_cdf_data;
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

        info!("Loaded env map: {}x{} {:?} from {:?}", w, h, format, path);
        Ok(())
    }
}

fn srgb_to_linear(value: f32) -> f32 {
    if value <= 0.04045 {
        value / 12.92
    } else {
        ((value + 0.055) / 1.055).powf(2.4)
    }
}

fn sanitize_radiance(value: f32) -> f32 {
    if value.is_finite() {
        value.max(0.0)
    } else {
        0.0
    }
}

fn linear_luminance(rgb: [f32; 3]) -> f32 {
    0.2126 * rgb[0] + 0.7152 * rgb[1] + 0.0722 * rgb[2]
}

fn push_rgba16f(data: &mut Vec<u8>, rgb: [f32; 3], alpha: f32) {
    for value in [rgb[0], rgb[1], rgb[2], alpha] {
        data.extend_from_slice(&half::f16::from_f32(value).to_le_bytes());
    }
}

#[allow(clippy::needless_range_loop)]
fn build_env_cdfs(
    width: u32,
    height: u32,
    luminance: &[f32],
) -> anyhow::Result<(Vec<f32>, Vec<f32>)> {
    let pixel_count = render_core::checked_2d_buffer_len("environment CDF", width, height)?;
    anyhow::ensure!(
        luminance.len() == pixel_count,
        "environment luminance length mismatch: expected {pixel_count}, got {}",
        luminance.len()
    );
    let w =
        usize::try_from(width).map_err(|_| anyhow::anyhow!("environment width exceeds usize"))?;
    let h =
        usize::try_from(height).map_err(|_| anyhow::anyhow!("environment height exceeds usize"))?;
    let mut conditional_cdf = vec![0.0f32; pixel_count];
    let mut row_integrals = vec![0.0f32; h];

    for y in 0..h {
        let theta = std::f32::consts::PI * (y as f32 + 0.5) / h as f32;
        let sin_theta = theta.sin().max(1e-6);
        let row_start = y * w;
        let mut row_sum = 0.0f32;

        for x in 0..w {
            let lum = luminance[row_start + x] * sin_theta;
            row_sum += lum;
            conditional_cdf[row_start + x] = row_sum;
        }

        if row_sum > 0.0 && row_sum.is_finite() {
            for x in 0..w {
                conditional_cdf[row_start + x] /= row_sum;
            }
            row_integrals[y] = row_sum;
        } else {
            // A zero-energy row is never selected when another row has
            // energy, but keep its conditional distribution valid. If the
            // entire image is black this becomes the uniform fallback.
            for x in 0..w {
                conditional_cdf[row_start + x] = (x + 1) as f32 / w as f32;
            }
            row_integrals[y] = 0.0;
        }
    }

    let mut marginal_cdf = vec![0.0f32; h];
    let mut total = 0.0f32;
    for y in 0..h {
        total += row_integrals[y];
        marginal_cdf[y] = total;
    }
    if total > 0.0 && total.is_finite() {
        for y in 0..h {
            marginal_cdf[y] /= total;
        }
    } else {
        for y in 0..h {
            marginal_cdf[y] = (y + 1) as f32 / h as f32;
        }
    }

    Ok((conditional_cdf, marginal_cdf))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn srgb_reference_values_are_linearized() {
        assert_eq!(srgb_to_linear(0.0), 0.0);
        assert!((srgb_to_linear(0.04045) - 0.003130805).abs() < 1.0e-7);
        assert!((srgb_to_linear(0.5) - 0.21404114).abs() < 1.0e-7);
        assert_eq!(srgb_to_linear(1.0), 1.0);
    }

    #[test]
    fn black_environment_gets_uniform_valid_cdfs() {
        let (conditional, marginal) = build_env_cdfs(2, 2, &[0.0; 4]).unwrap();
        assert_eq!(conditional, vec![0.5, 1.0, 0.5, 1.0]);
        assert_eq!(marginal, vec![0.5, 1.0]);
    }

    #[test]
    fn cdf_rejects_mismatched_luminance() {
        assert!(build_env_cdfs(2, 2, &[1.0; 3]).is_err());
    }
}
