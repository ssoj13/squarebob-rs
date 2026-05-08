//! Pipeline creation for all 3D render passes
//! Deduplicated: common configs extracted into helpers

use super::geometry;

// Shader sources
const PBR_SHADER: &str = include_str!("../shaders/cube_pbr.wgsl");
const OBJECT_ID_SHADER: &str = include_str!("../shaders/cube_object_id.wgsl");
const OUTLINE_SHADER: &str = include_str!("../shaders/outline.wgsl");
const SKYBOX_SHADER: &str = include_str!("../shaders/skybox.wgsl");

/// All bind group layouts needed by the renderer
pub struct BindGroupLayouts {
    /// Group 0: Camera + LightRig + Material (PBR shader)
    pub pbr_group0: wgpu::BindGroupLayout,
    /// Group 1: Env map + sampler + params
    pub env: wgpu::BindGroupLayout,
    /// Group 0: Camera only (object_id shader)
    pub object_id_group0: wgpu::BindGroupLayout,
    /// Group 0: ID texture + hover params (outline shader)
    pub outline: wgpu::BindGroupLayout,
    /// Group 0: Camera + env map + sampler + params (skybox shader)
    pub skybox: wgpu::BindGroupLayout,
}

/// All render pipelines
pub struct Pipelines {
    pub pbr: wgpu::RenderPipeline,
    pub pbr_double: wgpu::RenderPipeline,
    pub wireframe: wgpu::RenderPipeline,
    pub transparent: wgpu::RenderPipeline,
    pub object_id: wgpu::RenderPipeline,
    pub object_id_double: wgpu::RenderPipeline,
    pub outline: wgpu::RenderPipeline,
    pub skybox: wgpu::RenderPipeline,
}

// ============================================================================
// Bind group layout helpers
// ============================================================================

/// Uniform buffer binding entry (common pattern)
fn uniform_entry(binding: u32, visibility: wgpu::ShaderStages) -> wgpu::BindGroupLayoutEntry {
    wgpu::BindGroupLayoutEntry {
        binding,
        visibility,
        ty: wgpu::BindingType::Buffer {
            ty: wgpu::BufferBindingType::Uniform,
            has_dynamic_offset: false,
            min_binding_size: None,
        },
        count: None,
    }
}

/// 2D float texture binding entry
fn texture_entry(binding: u32, sample_type: wgpu::TextureSampleType) -> wgpu::BindGroupLayoutEntry {
    wgpu::BindGroupLayoutEntry {
        binding,
        visibility: wgpu::ShaderStages::FRAGMENT,
        ty: wgpu::BindingType::Texture {
            sample_type,
            view_dimension: wgpu::TextureViewDimension::D2,
            multisampled: false,
        },
        count: None,
    }
}

/// Filtering sampler binding entry
fn sampler_entry(binding: u32) -> wgpu::BindGroupLayoutEntry {
    wgpu::BindGroupLayoutEntry {
        binding,
        visibility: wgpu::ShaderStages::FRAGMENT,
        ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
        count: None,
    }
}

impl BindGroupLayouts {
    pub fn new(device: &wgpu::Device) -> Self {
        let vf = wgpu::ShaderStages::VERTEX | wgpu::ShaderStages::FRAGMENT;
        let f = wgpu::ShaderStages::FRAGMENT;
        let v = wgpu::ShaderStages::VERTEX;

        Self {
            pbr_group0: device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
                label: Some("PBR BG0"),
                entries: &[
                    uniform_entry(0, vf), // Camera
                    uniform_entry(1, f),  // LightRig
                    uniform_entry(2, f),  // Material
                ],
            }),
            env: device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
                label: Some("Env BG"),
                entries: &[
                    texture_entry(0, wgpu::TextureSampleType::Float { filterable: true }),
                    sampler_entry(1),
                    uniform_entry(2, f),
                ],
            }),
            object_id_group0: device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
                label: Some("ObjID BG0"),
                entries: &[
                    uniform_entry(0, v), // Camera
                    // Selected IDs - storage buffer (unlimited size)
                    wgpu::BindGroupLayoutEntry {
                        binding: 1,
                        visibility: wgpu::ShaderStages::FRAGMENT,
                        ty: wgpu::BindingType::Buffer {
                            ty: wgpu::BufferBindingType::Storage { read_only: true },
                            has_dynamic_offset: false,
                            min_binding_size: None,
                        },
                        count: None,
                    },
                ],
            }),
            outline: device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
                label: Some("Outline BG"),
                entries: &[
                    texture_entry(0, wgpu::TextureSampleType::Uint),
                    uniform_entry(1, f),
                ],
            }),
            skybox: device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
                label: Some("Skybox BG"),
                entries: &[
                    uniform_entry(0, vf), // Camera
                    texture_entry(1, wgpu::TextureSampleType::Float { filterable: true }),
                    sampler_entry(2),
                    uniform_entry(3, f), // EnvParams
                ],
            }),
        }
    }
}

// ============================================================================
// Pipeline creation
// ============================================================================

/// Common depth stencil config
fn depth_stencil(write: bool, compare: wgpu::CompareFunction) -> wgpu::DepthStencilState {
    wgpu::DepthStencilState {
        format: wgpu::TextureFormat::Depth32Float,
        depth_write_enabled: Some(write),
        depth_compare: Some(compare),
        stencil: wgpu::StencilState::default(),
        bias: wgpu::DepthBiasState::default(),
    }
}

/// Config for creating an instanced cube pipeline variant
struct CubePipelineConfig<'a> {
    label: &'a str,
    layout: &'a wgpu::PipelineLayout,
    shader: &'a wgpu::ShaderModule,
    fs_entry: &'a str,
    format: wgpu::TextureFormat,
    blend: Option<wgpu::BlendState>,
    cull: Option<wgpu::Face>,
    polygon_mode: wgpu::PolygonMode,
    depth_write: bool,
    depth_compare: wgpu::CompareFunction,
}

/// Create an instanced cube pipeline with given config
fn create_cube_pipeline(device: &wgpu::Device, cfg: CubePipelineConfig) -> wgpu::RenderPipeline {
    let vtx_layouts = geometry::cube_vertex_layouts();
    device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
        label: Some(cfg.label),
        layout: Some(cfg.layout),
        vertex: wgpu::VertexState {
            module: cfg.shader,
            entry_point: Some("vs_main"),
            buffers: &vtx_layouts,
            compilation_options: Default::default(),
        },
        fragment: Some(wgpu::FragmentState {
            module: cfg.shader,
            entry_point: Some(cfg.fs_entry),
            targets: &[Some(wgpu::ColorTargetState {
                format: cfg.format,
                blend: cfg.blend,
                write_mask: wgpu::ColorWrites::ALL,
            })],
            compilation_options: Default::default(),
        }),
        primitive: wgpu::PrimitiveState {
            topology: wgpu::PrimitiveTopology::TriangleList,
            front_face: wgpu::FrontFace::Ccw,
            cull_mode: cfg.cull,
            polygon_mode: cfg.polygon_mode,
            ..Default::default()
        },
        depth_stencil: Some(depth_stencil(cfg.depth_write, cfg.depth_compare)),
        multisample: wgpu::MultisampleState::default(),
        multiview_mask: None,
        cache: None,
    })
}

/// Fullscreen post-process pipeline (no vertex buffers, fullscreen triangle)
fn create_fullscreen_pipeline(
    device: &wgpu::Device,
    label: &str,
    layout: &wgpu::PipelineLayout,
    shader: &wgpu::ShaderModule,
    blend: Option<wgpu::BlendState>,
    depth: Option<wgpu::DepthStencilState>,
) -> wgpu::RenderPipeline {
    device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
        label: Some(label),
        layout: Some(layout),
        vertex: wgpu::VertexState {
            module: shader,
            entry_point: Some("vs_main"),
            buffers: &[],
            compilation_options: Default::default(),
        },
        fragment: Some(wgpu::FragmentState {
            module: shader,
            entry_point: Some("fs_main"),
            targets: &[Some(wgpu::ColorTargetState {
                format: wgpu::TextureFormat::Rgba8Unorm,
                blend,
                write_mask: wgpu::ColorWrites::ALL,
            })],
            compilation_options: Default::default(),
        }),
        primitive: wgpu::PrimitiveState {
            topology: wgpu::PrimitiveTopology::TriangleList,
            ..Default::default()
        },
        depth_stencil: depth,
        multisample: wgpu::MultisampleState::default(),
        multiview_mask: None,
        cache: None,
    })
}

impl Pipelines {
    pub fn new(device: &wgpu::Device, layouts: &BindGroupLayouts) -> Self {
        // Shader modules
        let pbr_shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("PBR Shader"),
            source: wgpu::ShaderSource::Wgsl(PBR_SHADER.into()),
        });
        let obj_id_shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("ObjID Shader"),
            source: wgpu::ShaderSource::Wgsl(OBJECT_ID_SHADER.into()),
        });
        let outline_shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("Outline Shader"),
            source: wgpu::ShaderSource::Wgsl(OUTLINE_SHADER.into()),
        });
        let skybox_shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("Skybox Shader"),
            source: wgpu::ShaderSource::Wgsl(SKYBOX_SHADER.into()),
        });

        // Pipeline layouts
        let pbr_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("PBR Layout"),
            bind_group_layouts: &[Some(&layouts.pbr_group0), Some(&layouts.env)],
            immediate_size: 0,
        });
        let obj_id_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("ObjID Layout"),
            bind_group_layouts: &[Some(&layouts.object_id_group0)],
            immediate_size: 0,
        });
        let outline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("Outline Layout"),
            bind_group_layouts: &[Some(&layouts.outline)],
            immediate_size: 0,
        });
        let skybox_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("Skybox Layout"),
            bind_group_layouts: &[Some(&layouts.skybox)],
            immediate_size: 0,
        });

        let rgba8 = wgpu::TextureFormat::Rgba8Unorm;

        Self {
            pbr: create_cube_pipeline(
                device,
                CubePipelineConfig {
                    label: "PBR",
                    layout: &pbr_layout,
                    shader: &pbr_shader,
                    fs_entry: "fs_main",
                    format: rgba8,
                    blend: Some(wgpu::BlendState::REPLACE),
                    cull: None, // TEMP: disabled culling for debugging
                    polygon_mode: wgpu::PolygonMode::Fill,
                    depth_write: true,
                    depth_compare: wgpu::CompareFunction::Less,
                },
            ),
            pbr_double: create_cube_pipeline(
                device,
                CubePipelineConfig {
                    label: "PBR DoubleSided",
                    layout: &pbr_layout,
                    shader: &pbr_shader,
                    fs_entry: "fs_main",
                    format: rgba8,
                    blend: Some(wgpu::BlendState::REPLACE),
                    cull: None,
                    polygon_mode: wgpu::PolygonMode::Fill,
                    depth_write: true,
                    depth_compare: wgpu::CompareFunction::Less,
                },
            ),
            wireframe: create_cube_pipeline(
                device,
                CubePipelineConfig {
                    label: "Wireframe",
                    layout: &pbr_layout,
                    shader: &pbr_shader,
                    fs_entry: "fs_wireframe",
                    format: rgba8,
                    blend: Some(wgpu::BlendState::REPLACE),
                    cull: None,
                    polygon_mode: wgpu::PolygonMode::Line,
                    depth_write: true,
                    depth_compare: wgpu::CompareFunction::Less,
                },
            ),
            transparent: create_cube_pipeline(
                device,
                CubePipelineConfig {
                    label: "Transparent",
                    layout: &pbr_layout,
                    shader: &pbr_shader,
                    fs_entry: "fs_main",
                    format: rgba8,
                    blend: Some(wgpu::BlendState::ALPHA_BLENDING),
                    cull: None,
                    polygon_mode: wgpu::PolygonMode::Fill,
                    depth_write: false,
                    depth_compare: wgpu::CompareFunction::Less,
                },
            ),
            object_id: create_cube_pipeline(
                device,
                CubePipelineConfig {
                    label: "Object ID",
                    layout: &obj_id_layout,
                    shader: &obj_id_shader,
                    fs_entry: "fs_main",
                    format: wgpu::TextureFormat::R32Uint,
                    blend: None,
                    cull: Some(wgpu::Face::Back),
                    polygon_mode: wgpu::PolygonMode::Fill,
                    depth_write: true, // Must write depth for proper occlusion (like alembic-rs)
                    depth_compare: wgpu::CompareFunction::LessEqual,
                },
            ),
            object_id_double: create_cube_pipeline(
                device,
                CubePipelineConfig {
                    label: "Object ID DoubleSided",
                    layout: &obj_id_layout,
                    shader: &obj_id_shader,
                    fs_entry: "fs_main",
                    format: wgpu::TextureFormat::R32Uint,
                    blend: None,
                    cull: None,
                    polygon_mode: wgpu::PolygonMode::Fill,
                    depth_write: true, // Must write depth for proper occlusion
                    depth_compare: wgpu::CompareFunction::LessEqual,
                },
            ),
            outline: create_fullscreen_pipeline(
                device,
                "Outline",
                &outline_layout,
                &outline_shader,
                Some(wgpu::BlendState::ALPHA_BLENDING),
                None,
            ),
            skybox: create_fullscreen_pipeline(
                device,
                "Skybox",
                &skybox_layout,
                &skybox_shader,
                Some(wgpu::BlendState::REPLACE),
                Some(depth_stencil(false, wgpu::CompareFunction::LessEqual)),
            ),
        }
    }
}
