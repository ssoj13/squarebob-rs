//! Environment map loading and management
//! Supports HDR/LDR images (PNG/JPG/HDR/EXR) via the image crate

use image::{GenericImageView, ImageFormat, ImageReader};
use log::info;
use render_core::gpu::GpuContext;
use wgpu::util::DeviceExt;

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
            size: wgpu::Extent3d { width: 1, height: 1, depth_or_array_layers: 1 },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::Rgba8Unorm,
            usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
            view_formats: &[],
        });
        ctx.queue.write_texture(
            wgpu::TexelCopyTextureInfo {
                texture: &texture, mip_level: 0,
                origin: wgpu::Origin3d::ZERO, aspect: wgpu::TextureAspect::All,
            },
            &[128u8, 128, 128, 255],
            wgpu::TexelCopyBufferLayout { offset: 0, bytes_per_row: Some(4), rows_per_image: Some(1) },
            wgpu::Extent3d { width: 1, height: 1, depth_or_array_layers: 1 },
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

        let marginal_cdf = ctx.device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("default_env_marginal_cdf"),
            contents: bytemuck::cast_slice(&[1.0f32]),
            usage: wgpu::BufferUsages::STORAGE,
        });
        let conditional_cdf = ctx.device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("default_env_conditional_cdf"),
            contents: bytemuck::cast_slice(&[1.0f32]),
            usage: wgpu::BufferUsages::STORAGE,
        });

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
    /// HDR/EXR are loaded as Rgba16Float to preserve dynamic range
    pub fn load_from_file(&mut self, ctx: &GpuContext, path: &std::path::Path) -> anyhow::Result<()> {
        let ext = path.extension()
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

        // Detect HDR formats by extension
        let is_hdr = ext
            .as_deref()
            .map(|e| matches!(e, "hdr" | "exr"))
            .unwrap_or(false);

        let (format, data, luminance): (wgpu::TextureFormat, Vec<u8>, Vec<f32>) = if is_hdr {
            // HDR: convert to f16 (Rgba16Float) to preserve dynamic range
            let rgba32 = img.to_rgba32f();
            let f16_data: Vec<u8> = rgba32.pixels()
                .flat_map(|p| {
                    p.0.iter().flat_map(|&f| {
                        half::f16::from_f32(f).to_le_bytes()
                    }).collect::<Vec<u8>>()
                })
                .collect();
            let lum: Vec<f32> = rgba32.pixels()
                .map(|p| 0.2126 * p[0] + 0.7152 * p[1] + 0.0722 * p[2])
                .collect();
            (wgpu::TextureFormat::Rgba16Float, f16_data, lum)
        } else {
            // LDR: standard 8-bit
            let rgba8 = img.to_rgba8();
            let lum: Vec<f32> = rgba8.pixels()
                .map(|p| {
                    let r = p[0] as f32 / 255.0;
                    let g = p[1] as f32 / 255.0;
                    let b = p[2] as f32 / 255.0;
                    0.2126 * r + 0.7152 * g + 0.0722 * b
                })
                .collect();
            (wgpu::TextureFormat::Rgba8Unorm, rgba8.into_raw(), lum)
        };

        let texture = ctx.device.create_texture(&wgpu::TextureDescriptor {
            label: Some("Env Map"),
            size: wgpu::Extent3d { width: w, height: h, depth_or_array_layers: 1 },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format,
            usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
            view_formats: &[],
        });

        let bytes_per_pixel = if is_hdr { 8u32 } else { 4u32 };
        ctx.queue.write_texture(
            wgpu::TexelCopyTextureInfo {
                texture: &texture, mip_level: 0,
                origin: wgpu::Origin3d::ZERO, aspect: wgpu::TextureAspect::All,
            },
            &data,
            wgpu::TexelCopyBufferLayout { offset: 0, bytes_per_row: Some(bytes_per_pixel * w), rows_per_image: Some(h) },
            wgpu::Extent3d { width: w, height: h, depth_or_array_layers: 1 },
        );

        self.view = texture.create_view(&Default::default());
        self.texture = texture;
        self.width = w;
        self.height = h;

        let (conditional_cdf_data, marginal_cdf_data) = build_env_cdfs(w, h, &luminance);
        self.conditional_cdf_data = conditional_cdf_data;
        self.marginal_cdf_data = marginal_cdf_data;
        self.conditional_cdf = ctx.device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("env_conditional_cdf"),
            contents: bytemuck::cast_slice(&self.conditional_cdf_data),
            usage: wgpu::BufferUsages::STORAGE,
        });
        self.marginal_cdf = ctx.device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("env_marginal_cdf"),
            contents: bytemuck::cast_slice(&self.marginal_cdf_data),
            usage: wgpu::BufferUsages::STORAGE,
        });

        info!("Loaded env map: {}x{} {:?} from {:?}", w, h, format, path);
        Ok(())
    }
}

#[allow(clippy::needless_range_loop)]
fn build_env_cdfs(width: u32, height: u32, luminance: &[f32]) -> (Vec<f32>, Vec<f32>) {
    let w = width as usize;
    let h = height as usize;
    let mut conditional_cdf = vec![0.0f32; w * h];
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

        if row_sum > 0.0 {
            for x in 0..w {
                conditional_cdf[row_start + x] /= row_sum;
            }
        }
        row_integrals[y] = row_sum;
    }

    let mut marginal_cdf = vec![0.0f32; h];
    let mut total = 0.0f32;
    for y in 0..h {
        total += row_integrals[y];
        marginal_cdf[y] = total;
    }
    if total > 0.0 {
        for y in 0..h {
            marginal_cdf[y] /= total;
        }
    }

    (conditional_cdf, marginal_cdf)
}
