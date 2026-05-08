//! GPU render targets and bind group management
//! Handles texture creation/recreation on resize

use super::pipelines::BindGroupLayouts;

/// All render target textures and their views
pub struct RenderTargets {
    pub render_texture: wgpu::Texture,
    pub render_view: wgpu::TextureView,
    #[allow(dead_code)] // Must stay alive for depth_view
    pub depth_texture: wgpu::Texture,
    pub depth_view: wgpu::TextureView,
    pub object_id_texture: wgpu::Texture,
    pub object_id_view: wgpu::TextureView,
    pub size: (u32, u32),
}

/// Bind groups that depend on render targets (recreated on resize)
pub struct DynamicBindGroups {
    pub outline: wgpu::BindGroup,
    pub skybox: wgpu::BindGroup,
}

impl RenderTargets {
    /// Create all render targets for given dimensions
    pub fn new(device: &wgpu::Device, width: u32, height: u32) -> Self {
        let size = wgpu::Extent3d {
            width,
            height,
            depth_or_array_layers: 1,
        };

        let depth_texture = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("Depth"),
            size,
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::Depth32Float,
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT,
            view_formats: &[],
        });

        let render_texture = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("Render"),
            size,
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::Rgba8Unorm,
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT
                | wgpu::TextureUsages::COPY_SRC
                | wgpu::TextureUsages::TEXTURE_BINDING, // For egui zero-copy
            view_formats: &[],
        });

        let object_id_texture = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("Object ID"),
            size,
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::R32Uint,
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT
                | wgpu::TextureUsages::TEXTURE_BINDING
                | wgpu::TextureUsages::COPY_SRC,
            view_formats: &[],
        });

        Self {
            render_view: render_texture.create_view(&Default::default()),
            depth_view: depth_texture.create_view(&Default::default()),
            object_id_view: object_id_texture.create_view(&Default::default()),
            render_texture,
            depth_texture,
            object_id_texture,
            size: (width, height),
        }
    }
}

impl DynamicBindGroups {
    /// Create bind groups that reference render targets + shared resources
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        device: &wgpu::Device,
        layouts: &BindGroupLayouts,
        targets: &RenderTargets,
        camera_buffer: &wgpu::Buffer,
        hover_params_buffer: &wgpu::Buffer,
        env_view: &wgpu::TextureView,
        env_sampler: &wgpu::Sampler,
        env_params_buffer: &wgpu::Buffer,
    ) -> Self {
        Self {
            outline: device.create_bind_group(&wgpu::BindGroupDescriptor {
                label: Some("Outline BG"),
                layout: &layouts.outline,
                entries: &[
                    wgpu::BindGroupEntry {
                        binding: 0,
                        resource: wgpu::BindingResource::TextureView(&targets.object_id_view),
                    },
                    wgpu::BindGroupEntry {
                        binding: 1,
                        resource: hover_params_buffer.as_entire_binding(),
                    },
                ],
            }),
            skybox: device.create_bind_group(&wgpu::BindGroupDescriptor {
                label: Some("Skybox BG"),
                layout: &layouts.skybox,
                entries: &[
                    wgpu::BindGroupEntry {
                        binding: 0,
                        resource: camera_buffer.as_entire_binding(),
                    },
                    wgpu::BindGroupEntry {
                        binding: 1,
                        resource: wgpu::BindingResource::TextureView(env_view),
                    },
                    wgpu::BindGroupEntry {
                        binding: 2,
                        resource: wgpu::BindingResource::Sampler(env_sampler),
                    },
                    wgpu::BindGroupEntry {
                        binding: 3,
                        resource: env_params_buffer.as_entire_binding(),
                    },
                ],
            }),
        }
    }
}
