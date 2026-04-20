//! Compute pipeline for instance-based BVH path tracing.
//!
//! Ray-box intersection against cube instances (no triangle expansion).
//! Progressive accumulation, HDR env map, tone-mapped blit.

use wgpu::util::DeviceExt;
use bytemuck::{Pod, Zeroable};

use pt_core::gpu_data::GpuInstanceSceneData;
use bvh_gpu::{GpuBvhBuilder, GpuBvhConfig};
use pt_core::bvh::Instance;
use pt_wavefront::{WavefrontPipeline, WavefrontConfig, WfDims};
use crate::restir::{ReSTIRPipeline, ReSTIRConfig, Reservoir};
use crate::adaptive::{AdaptivePipeline, AdaptiveConfig};
use crate::pathguide::{PathGuidePipeline, PathGuideConfig};

fn bgl_uniform(binding: u32) -> wgpu::BindGroupLayoutEntry {
    wgpu::BindGroupLayoutEntry {
        binding,
        visibility: wgpu::ShaderStages::COMPUTE,
        ty: wgpu::BindingType::Buffer {
            ty: wgpu::BufferBindingType::Uniform,
            has_dynamic_offset: false,
            min_binding_size: None,
        },
        count: None,
    }
}

fn bgl_storage_ro(binding: u32) -> wgpu::BindGroupLayoutEntry {
    wgpu::BindGroupLayoutEntry {
        binding,
        visibility: wgpu::ShaderStages::COMPUTE,
        ty: wgpu::BindingType::Buffer {
            ty: wgpu::BufferBindingType::Storage { read_only: true },
            has_dynamic_offset: false,
            min_binding_size: None,
        },
        count: None,
    }
}

fn bgl_storage_rw(binding: u32) -> wgpu::BindGroupLayoutEntry {
    wgpu::BindGroupLayoutEntry {
        binding,
        visibility: wgpu::ShaderStages::COMPUTE,
        ty: wgpu::BindingType::Buffer {
            ty: wgpu::BufferBindingType::Storage { read_only: false },
            has_dynamic_offset: false,
            min_binding_size: None,
        },
        count: None,
    }
}

/// WGSL source embedded at compile time.
const BVH_TRAVERSE_WGSL: &str = include_str!("bvh_traverse.wgsl");
const BLIT_WGSL: &str = include_str!("blit.wgsl");
const PICK_WGSL: &str = include_str!("pick.wgsl");
const GBUFFER_WGSL: &str = include_str!("wavefront/gbuffer.wgsl");

/// Camera uniform matching the WGSL Camera struct.
#[repr(C)]
#[derive(Debug, Clone, Copy, Pod, Zeroable)]
pub struct PtCameraUniform {
    pub inv_view: [[f32; 4]; 4],    // 64B
    pub inv_proj: [[f32; 4]; 4],    // 64B
    pub position: [f32; 3],         // 12B
    pub _pad0: u32,                 //  4B
    pub frame_count: u32,           //  4B
    pub max_bounces: u32,           //  4B
    pub max_transmission_depth: u32,//  4B
    pub dof_enabled: u32,           //  4B
    pub aperture: f32,              //  4B
    pub focus_distance: f32,        //  4B
    pub _pad1: [u32; 2],            //  8B
    // Slice plane params
    pub slice_enabled: f32,         //  4B
    pub slice_position: f32,        //  4B
    pub slice_invert: f32,          //  4B
    pub _pad2: f32,                 //  4B (align to 16B)
    pub slice_normal: [f32; 3],     // 12B
    pub _pad3: f32,                 //  4B (align to 16B)
    // Spectral options (PT only)
    pub spectral_mode: u32,         //  4B (0=off,1=hero,2=multi)
    pub spectral_samples: u32,      //  4B
    pub spectral_dispersion: u32,   //  4B
    pub _pad4: u32,                 //  4B (align to 16B)
}

const WG_SIZE: u32 = 8;

/// Environment uniform for path tracer.
#[repr(C)]
#[derive(Debug, Clone, Copy, Pod, Zeroable)]
pub struct PtEnvUniform {
    pub params0: [f32; 4], // intensity, rotation, enabled, use_importance_sampling
    pub params1: [f32; 4], // env_width, env_height, global_opacity, time
}

impl Default for PtEnvUniform {
    fn default() -> Self {
        Self {
            params0: [1.0, 0.0, 0.0, 0.0],  // intensity, rotation, enabled, use_importance_sampling
            params1: [1.0, 1.0, 1.0, 0.0],  // env_width, env_height, global_opacity, time
        }
    }
}

/// Ray pick params for GPU picking.
#[repr(C)]
#[derive(Debug, Clone, Copy, Pod, Zeroable)]
pub struct PickParams {
    pub origin: [f32; 3],
    pub _pad0: f32,
    pub dir: [f32; 3],
    pub _pad1: f32,
}

/// Ray pick result (object_id + t).
/// Matches WGSL layout: vec3<f32> has alignment 16, so struct is 48 bytes.
#[repr(C)]
#[derive(Debug, Clone, Copy, Pod, Zeroable)]
pub struct PickResult {
    pub object_id: u32,       // offset 0
    pub hit: u32,             // offset 4
    pub _pad0: u32,           // offset 8
    pub _pad1: u32,           // offset 12
    pub t: f32,               // offset 16
    pub _align_pad: [u32; 3], // offset 20-31 (padding to align vec3 to 32)
    pub _pad2: [f32; 3],      // offset 32-43 (vec3<f32> aligned to 16)
    pub _final_pad: u32,      // offset 44-47 (struct alignment to 16)
}

#[repr(C)]
#[derive(Debug, Clone, Copy, Pod, Zeroable)]
struct GBufferParams {
    width: u32,
    height: u32,
    _pad0: [u32; 2],
    prev_view_proj: [[f32; 4]; 4],
    curr_view_proj: [[f32; 4]; 4],
}

#[repr(C)]
#[derive(Debug, Clone, Copy, Pod, Zeroable)]
struct RestirInitialParams {
    width: u32,
    height: u32,
    frame_count: u32,
    num_candidates: u32,
}

#[repr(C)]
#[derive(Debug, Clone, Copy, Pod, Zeroable)]
struct RestirTemporalParams {
    width: u32,
    height: u32,
    frame_count: u32,
    m_max: u32,
    depth_threshold: f32,
    _pad: [f32; 3],
    _pad2: [f32; 4],
}

#[repr(C)]
#[derive(Debug, Clone, Copy, Pod, Zeroable)]
struct RestirSpatialParams {
    width: u32,
    height: u32,
    frame_count: u32,
    num_neighbors: u32,
    radius: f32,
    normal_threshold: f32,
    depth_threshold: f32,
    _pad: f32,
}

#[repr(C)]
#[derive(Debug, Clone, Copy, Pod, Zeroable)]
struct RestirShadeParams {
    width: u32,
    height: u32,
    frame_count: u32,
    _pad: u32,
    camera_pos: [f32; 3],
    _pad2: f32,
}

#[repr(C)]
#[derive(Debug, Clone, Copy, Pod, Zeroable)]
struct PathGuideUpdateParams {
    scene_min: [f32; 4],
    scene_max: [f32; 4],
    params0: [u32; 4], // resolution, sample_count
    params1: [f32; 4], // decay
}

#[repr(C)]
#[derive(Debug, Clone, Copy, Pod, Zeroable)]
struct PathGuideSampleParams {
    scene_min: [f32; 4],
    scene_max: [f32; 4],
    params0: [u32; 4], // resolution, frame_count
    params1: [f32; 4], // guide_weight
}

/// Path trace compute pipeline state.
pub struct PathTraceCompute {
    pipeline: wgpu::ComputePipeline,
    bind_group_layout: wgpu::BindGroupLayout,
    bind_group: Option<wgpu::BindGroup>,

    // Pick pipeline
    pick_pipeline: wgpu::ComputePipeline,
    pick_bind_group_layout: wgpu::BindGroupLayout,
    pick_bind_group: Option<wgpu::BindGroup>,

    // Storage buffers
    nodes_buffer: Option<wgpu::Buffer>,
    instances_buffer: Option<wgpu::Buffer>,
    materials_buffer: Option<wgpu::Buffer>,

    // Pick buffers
    pick_params_buffer: wgpu::Buffer,
    pick_result_buffer: wgpu::Buffer,
    pick_readback_buffer: wgpu::Buffer,

    // Camera uniform
    camera_buffer: wgpu::Buffer,

    // Output texture (rgba32float storage)
    output_texture: wgpu::Texture,
    output_view: wgpu::TextureView,

    // Accumulation buffer (vec4<f32> per pixel)
    accum_buffer: wgpu::Buffer,
    // Variance buffer (M2 for Welford's algorithm, vec4<f32> per pixel)
    variance_buffer: wgpu::Buffer,

    // Environment map
    #[allow(dead_code)]
    env_texture: wgpu::Texture,
    env_view: wgpu::TextureView,
    env_sampler: wgpu::Sampler,
    env_uniform_buffer: wgpu::Buffer,
    env_intensity: f32,
    env_rotation: f32,
    env_enabled: f32,
    env_use_importance_sampling: f32,
    env_width: u32,
    env_height: u32,

    // Environment importance sampling CDFs
    env_marginal_cdf: wgpu::Buffer,
    env_conditional_cdf: wgpu::Buffer,

    // Dimensions
    width: u32,
    height: u32,

    // Progressive frame counter
    pub frame_count: u32,
    pub max_samples: u32,

    /// Last camera position for change detection (resets accumulation on move)
    pub last_camera_pos: Option<[f32; 3]>,
    /// Last view-projection matrix for change detection
    pub last_view_proj: Option<[[f32; 4]; 4]>,
    /// Last slice plane params for change detection (enabled, axis, position, invert)
    pub last_slice_params: Option<(bool, [f32; 3], f32, bool)>,

    // Spectral settings (forwarded to wavefront shade params)
    spectral_mode: u32,
    spectral_samples: u32,
    spectral_dispersion: u32,

    scene_ready: bool,

    // Blit pipeline
    blit_pipeline: wgpu::RenderPipeline,
    blit_bind_group_layout: wgpu::BindGroupLayout,
    blit_bind_group: Option<wgpu::BindGroup>,
    blit_sampler: wgpu::Sampler,

    // GPU BVH builder with refit support for animation
    bvh_builder: GpuBvhBuilder,
    bvh_config: GpuBvhConfig,
    sorted_indices: Vec<u32>,

    // Wavefront path tracing (optional)
    wavefront: Option<WavefrontPipeline>,
    wavefront_config: WavefrontConfig,
    wavefront_bind_groups: Option<WavefrontBindGroups>,
    wavefront_rr_enabled: bool,

    // ReSTIR (optional) - infrastructure ready, integration in progress
    #[allow(dead_code)]
    restir: Option<ReSTIRPipeline>,
    #[allow(dead_code)]
    restir_config: ReSTIRConfig,
    gbuffer_pipeline: Option<wgpu::ComputePipeline>,
    gbuffer_bgl: Option<wgpu::BindGroupLayout>,
    restir_bind_groups: Option<ReSTIRBindGroups>,

    // Adaptive sampling (optional)
    adaptive: Option<AdaptivePipeline>,
    adaptive_config: AdaptiveConfig,
    adaptive_bind_groups: Option<AdaptiveBindGroups>,
    sample_map_fallback: wgpu::Buffer,

    // Path guiding (optional)
    pathguide: Option<PathGuidePipeline>,
    pathguide_config: PathGuideConfig,
    pathguide_bind_groups: Option<PathGuideBindGroups>,
    last_pathguide_enabled: bool,
    last_pathguide_svo_resolution: u32,
    guide_buffer: wgpu::Buffer,
    scene_bounds: Option<([f32; 3], [f32; 3])>,
    history_dirty: bool,
}

/// Wavefront bind groups for one ping-pong state.
struct WavefrontBindGroupSet {
    raygen_bg: wgpu::BindGroup,
    intersect_bg: wgpu::BindGroup,
    shade_bg: wgpu::BindGroup,
}

/// Wavefront bind groups (two sets for ping-pong).
struct WavefrontBindGroups {
    /// Set A: rays_a -> hits -> rays_b
    set_a: WavefrontBindGroupSet,
    /// Set B: rays_b -> hits -> rays_a  
    set_b: WavefrontBindGroupSet,
    /// Count swap bind group (shared)
    count_swap_bg: wgpu::BindGroup,
    /// Finalize bind group (same for both)
    finalize_bg: wgpu::BindGroup,
    shade_params_buf: wgpu::Buffer,
    finalize_params_buf: wgpu::Buffer,
    /// Current ping-pong state (0 = use set_a, 1 = use set_b)
    cur_set: u32,
}

/// Adaptive sampling bind groups.
struct AdaptiveBindGroups {
    variance_bg: wgpu::BindGroup,
    allocate_bg: wgpu::BindGroup,
    variance_params_buf: wgpu::Buffer,
    allocate_params_buf: wgpu::Buffer,
}

struct ReSTIRBindGroups {
    gbuffer_bg_a: wgpu::BindGroup,
    gbuffer_bg_b: wgpu::BindGroup,
    gbuffer_params_buf: wgpu::Buffer,
    initial_bg: wgpu::BindGroup,
    temporal_bg: wgpu::BindGroup,
    spatial_bg: wgpu::BindGroup,
    shade_bg_cur_a: wgpu::BindGroup,
    shade_bg_cur_b: wgpu::BindGroup,
    shade_bg_prev_a: wgpu::BindGroup,
    shade_bg_prev_b: wgpu::BindGroup,
    initial_params_buf: wgpu::Buffer,
    temporal_params_buf: wgpu::Buffer,
    spatial_params_buf: wgpu::Buffer,
    shade_params_buf: wgpu::Buffer,
    prev_depth_buf: wgpu::Buffer,
}

struct PathGuideBindGroups {
    update_bg: wgpu::BindGroup,
    sample_bg: wgpu::BindGroup,
    update_params_buf: wgpu::Buffer,
    sample_params_buf: wgpu::Buffer,
}

impl PathTraceCompute {
    /// Create a new path trace compute pipeline.
    /// Bind group layout: 0=nodes, 1=instances, 2=camera, 3=output, 4=accum, 5=materials, 6=env_tex, 7=env_sampler, 8=env_params, 9=env_marginal_cdf, 10=env_conditional_cdf, 11=sample_map
    pub fn new(
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        width: u32,
        height: u32,
        surface_format: wgpu::TextureFormat,
    ) -> Self {
        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("bvh_traverse_shader"),
            source: wgpu::ShaderSource::Wgsl(BVH_TRAVERSE_WGSL.into()),
        });

        let bind_group_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("pt_bind_group_layout"),
            entries: &[
                // @binding(0) BVH nodes
                wgpu::BindGroupLayoutEntry {
                    binding: 0,
                    visibility: wgpu::ShaderStages::COMPUTE,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Storage { read_only: true },
                        has_dynamic_offset: false,
                        min_binding_size: None,
                    },
                    count: None,
                },
                // @binding(1) Instances
                wgpu::BindGroupLayoutEntry {
                    binding: 1,
                    visibility: wgpu::ShaderStages::COMPUTE,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Storage { read_only: true },
                        has_dynamic_offset: false,
                        min_binding_size: None,
                    },
                    count: None,
                },
                // @binding(2) Camera uniform
                wgpu::BindGroupLayoutEntry {
                    binding: 2,
                    visibility: wgpu::ShaderStages::COMPUTE,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Uniform,
                        has_dynamic_offset: false,
                        min_binding_size: None,
                    },
                    count: None,
                },
                // @binding(3) Output storage texture
                wgpu::BindGroupLayoutEntry {
                    binding: 3,
                    visibility: wgpu::ShaderStages::COMPUTE,
                    ty: wgpu::BindingType::StorageTexture {
                        access: wgpu::StorageTextureAccess::WriteOnly,
                        format: wgpu::TextureFormat::Rgba32Float,
                        view_dimension: wgpu::TextureViewDimension::D2,
                    },
                    count: None,
                },
                // @binding(4) Accumulation buffer
                wgpu::BindGroupLayoutEntry {
                    binding: 4,
                    visibility: wgpu::ShaderStages::COMPUTE,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Storage { read_only: false },
                        has_dynamic_offset: false,
                        min_binding_size: None,
                    },
                    count: None,
                },
                // @binding(5) Materials
                wgpu::BindGroupLayoutEntry {
                    binding: 5,
                    visibility: wgpu::ShaderStages::COMPUTE,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Storage { read_only: true },
                        has_dynamic_offset: false,
                        min_binding_size: None,
                    },
                    count: None,
                },
                // @binding(6) Environment map texture
                wgpu::BindGroupLayoutEntry {
                    binding: 6,
                    visibility: wgpu::ShaderStages::COMPUTE,
                    ty: wgpu::BindingType::Texture {
                        sample_type: wgpu::TextureSampleType::Float { filterable: true },
                        view_dimension: wgpu::TextureViewDimension::D2,
                        multisampled: false,
                    },
                    count: None,
                },
                // @binding(7) Environment sampler
                wgpu::BindGroupLayoutEntry {
                    binding: 7,
                    visibility: wgpu::ShaderStages::COMPUTE,
                    ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
                    count: None,
                },
                // @binding(8) Environment params uniform
                wgpu::BindGroupLayoutEntry {
                    binding: 8,
                    visibility: wgpu::ShaderStages::COMPUTE,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Uniform,
                        has_dynamic_offset: false,
                        min_binding_size: None,
                    },
                    count: None,
                },
                // @binding(9) Environment marginal CDF
                wgpu::BindGroupLayoutEntry {
                    binding: 9,
                    visibility: wgpu::ShaderStages::COMPUTE,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Storage { read_only: true },
                        has_dynamic_offset: false,
                        min_binding_size: None,
                    },
                    count: None,
                },
                // @binding(10) Environment conditional CDF
                wgpu::BindGroupLayoutEntry {
                    binding: 10,
                    visibility: wgpu::ShaderStages::COMPUTE,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Storage { read_only: true },
                        has_dynamic_offset: false,
                        min_binding_size: None,
                    },
                    count: None,
                },
                // @binding(11) Adaptive sample map (per-pixel spp limit)
                wgpu::BindGroupLayoutEntry {
                    binding: 11,
                    visibility: wgpu::ShaderStages::COMPUTE,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Storage { read_only: true },
                        has_dynamic_offset: false,
                        min_binding_size: None,
                    },
                    count: None,
                },
            ],
        });

        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("pt_pipeline_layout"),
            bind_group_layouts: &[&bind_group_layout],
            push_constant_ranges: &[],
        });

        let pipeline = device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
            label: Some("pt_compute_pipeline"),
            layout: Some(&pipeline_layout),
            module: &shader,
            entry_point: Some("main"),
            compilation_options: Default::default(),
            cache: None,
        });

        // Pick pipeline (single ray)
        let pick_shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("pt_pick_shader"),
            source: wgpu::ShaderSource::Wgsl(PICK_WGSL.into()),
        });
        let pick_bind_group_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("pt_pick_bgl"),
            entries: &[
                wgpu::BindGroupLayoutEntry {
                    binding: 0,
                    visibility: wgpu::ShaderStages::COMPUTE,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Storage { read_only: true },
                        has_dynamic_offset: false,
                        min_binding_size: None,
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 1,
                    visibility: wgpu::ShaderStages::COMPUTE,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Storage { read_only: true },
                        has_dynamic_offset: false,
                        min_binding_size: None,
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 2,
                    visibility: wgpu::ShaderStages::COMPUTE,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Uniform,
                        has_dynamic_offset: false,
                        min_binding_size: None,
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 3,
                    visibility: wgpu::ShaderStages::COMPUTE,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Storage { read_only: false },
                        has_dynamic_offset: false,
                        min_binding_size: None,
                    },
                    count: None,
                },
            ],
        });
        let pick_pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("pt_pick_pl"),
            bind_group_layouts: &[&pick_bind_group_layout],
            push_constant_ranges: &[],
        });
        let pick_pipeline = device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
            label: Some("pt_pick_pipeline"),
            layout: Some(&pick_pipeline_layout),
            module: &pick_shader,
            entry_point: Some("main"),
            compilation_options: Default::default(),
            cache: None,
        });

        let camera_buffer = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("pt_camera_buffer"),
            size: std::mem::size_of::<PtCameraUniform>() as u64,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });

        let pick_params_buffer = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("pt_pick_params"),
            size: std::mem::size_of::<PickParams>() as u64,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let pick_result_buffer = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("pt_pick_result"),
            size: std::mem::size_of::<PickResult>() as u64,
            usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_SRC,
            mapped_at_creation: false,
        });
        let pick_readback_buffer = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("pt_pick_readback"),
            size: std::mem::size_of::<PickResult>() as u64,
            usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
            mapped_at_creation: false,
        });

        let (output_texture, output_view) = Self::create_output(device, width, height);
        let accum_buffer = Self::create_accum_buffer(device, width, height);
        let variance_buffer = Self::create_variance_buffer(device, width, height);

        // Blit pipeline
        let blit_shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("pt_blit_shader"),
            source: wgpu::ShaderSource::Wgsl(BLIT_WGSL.into()),
        });

        let blit_bind_group_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("pt_blit_bgl"),
            entries: &[
                wgpu::BindGroupLayoutEntry {
                    binding: 0,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Texture {
                        multisampled: false,
                        view_dimension: wgpu::TextureViewDimension::D2,
                        sample_type: wgpu::TextureSampleType::Float { filterable: false },
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 1,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::NonFiltering),
                    count: None,
                },
            ],
        });

        let blit_pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("pt_blit_pl"),
            bind_group_layouts: &[&blit_bind_group_layout],
            push_constant_ranges: &[],
        });

        let blit_pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("pt_blit_pipeline"),
            layout: Some(&blit_pipeline_layout),
            vertex: wgpu::VertexState {
                module: &blit_shader,
                entry_point: Some("vs_main"),
                compilation_options: Default::default(),
                buffers: &[],
            },
            fragment: Some(wgpu::FragmentState {
                module: &blit_shader,
                entry_point: Some("fs_main"),
                compilation_options: Default::default(),
                targets: &[Some(wgpu::ColorTargetState {
                    format: surface_format,
                    blend: Some(wgpu::BlendState::REPLACE),
                    write_mask: wgpu::ColorWrites::ALL,
                })],
            }),
            primitive: wgpu::PrimitiveState::default(),
            depth_stencil: None,
            multisample: wgpu::MultisampleState::default(),
            multiview: None,
            cache: None,
        });

        let blit_sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("pt_blit_sampler"),
            mag_filter: wgpu::FilterMode::Nearest,
            min_filter: wgpu::FilterMode::Nearest,
            ..Default::default()
        });

        let fallback_samples = vec![u32::MAX; (width * height).max(1) as usize];
        let sample_map_fallback = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("pt_sample_map_fallback"),
            contents: bytemuck::cast_slice(&fallback_samples),
            usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_DST,
        });

        let guide_buffer = Self::create_guide_buffer(device, width, height);

        let blit_bind_group = Some(device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("pt_blit_bg"),
            layout: &blit_bind_group_layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: wgpu::BindingResource::TextureView(&output_view),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: wgpu::BindingResource::Sampler(&blit_sampler),
                },
            ],
        }));

        // Default 1x1 black environment texture
        let (env_texture, env_view) = Self::create_default_env_texture(device, queue);
        let env_sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("pt_env_sampler"),
            address_mode_u: wgpu::AddressMode::Repeat,
            address_mode_v: wgpu::AddressMode::ClampToEdge,
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            ..Default::default()
        });
        let env_uniform_buffer = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("pt_env_uniform"),
            contents: bytemuck::bytes_of(&PtEnvUniform::default()),
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
        });

        let env_marginal_cdf = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("pt_env_marginal_cdf"),
            contents: bytemuck::cast_slice(&[1.0f32]),
            usage: wgpu::BufferUsages::STORAGE,
        });
        let env_conditional_cdf = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("pt_env_conditional_cdf"),
            contents: bytemuck::cast_slice(&[1.0f32]),
            usage: wgpu::BufferUsages::STORAGE,
        });

        Self {
            pipeline,
            bind_group_layout,
            bind_group: None,
            pick_pipeline,
            pick_bind_group_layout,
            pick_bind_group: None,
            nodes_buffer: None,
            instances_buffer: None,
            materials_buffer: None,
            pick_params_buffer,
            pick_result_buffer,
            pick_readback_buffer,
            max_samples: 512,
            last_camera_pos: None,
            last_view_proj: None,
            last_slice_params: None,
            camera_buffer,
            output_texture,
            output_view,
            accum_buffer,
            variance_buffer,
            env_texture,
            env_view,
            env_sampler,
            env_uniform_buffer,
            env_intensity: 1.0,
            env_rotation: 0.0,
            env_enabled: 0.0,
            env_use_importance_sampling: 0.0,
            env_width: 1,
            env_height: 1,
            env_marginal_cdf,
            env_conditional_cdf,
            width,
            height,
            frame_count: 0,
            scene_ready: false,
            blit_pipeline,
            blit_bind_group_layout,
            blit_bind_group,
            blit_sampler,
            bvh_builder: GpuBvhBuilder::new(device),
            bvh_config: GpuBvhConfig::default(),
            sorted_indices: Vec::new(),
            wavefront: None,
            wavefront_config: WavefrontConfig::default(),
            wavefront_bind_groups: None,
            wavefront_rr_enabled: true,
            restir: None,
            restir_config: ReSTIRConfig::default(),
            gbuffer_pipeline: None,
            gbuffer_bgl: None,
            restir_bind_groups: None,
            adaptive: None,
            adaptive_config: AdaptiveConfig::default(),
            adaptive_bind_groups: None,
            sample_map_fallback,
            pathguide: None,
            pathguide_config: PathGuideConfig::default(),
            pathguide_bind_groups: None,
            last_pathguide_enabled: false,
            last_pathguide_svo_resolution: PathGuideConfig::default().svo_resolution,
            guide_buffer,
            scene_bounds: None,
            spectral_mode: 0,
            spectral_samples: 1,
            spectral_dispersion: 0,
            history_dirty: false,
        }
    }

    /// Enable/disable ReSTIR (infrastructure ready, integration pending).
    #[allow(dead_code)]
    pub fn set_restir_enabled(&mut self, device: &wgpu::Device, di: bool, gi: bool) {
        let prev_di = self.restir_config.di_enabled;
        let prev_gi = self.restir_config.gi_enabled;
        let mut needs_rebuild = prev_di != di || prev_gi != gi;
        if (di || gi) && self.restir.is_none() {
            log::info!("ReSTIR: create pipeline (di={}, gi={})", di, gi);
            self.restir = Some(ReSTIRPipeline::new(device, self.width, self.height));
            needs_rebuild = true;
        }
        self.restir_config.di_enabled = di;
        self.restir_config.gi_enabled = gi;
        log::debug!("ReSTIR: enabled di={}, gi={}", di, gi);
        if needs_rebuild {
            self.rebuild_restir_bind_groups(device);
        }
    }

    /// Update ReSTIR temporal/spatial settings.
    #[allow(dead_code)]
    pub fn set_restir_options(&mut self, temporal: bool, spatial: bool, m_max: u32) {
        self.restir_config.temporal = temporal;
        self.restir_config.spatial = spatial;
        self.restir_config.m_max = m_max;
        log::debug!(
            "ReSTIR: options temporal={}, spatial={}, m_max={}",
            temporal,
            spatial,
            m_max
        );
    }

    /// Enable/disable path guiding.
    pub fn set_pathguide_enabled(&mut self, device: &wgpu::Device, enabled: bool) {
        let mut needs_rebuild = self.last_pathguide_enabled != enabled
            || self.last_pathguide_svo_resolution != self.pathguide_config.svo_resolution;
        self.pathguide_config.enabled = enabled;
        if enabled && self.pathguide.is_none() {
            log::info!(
                "PathGuide: create pipeline (svo_resolution={})",
                self.pathguide_config.svo_resolution
            );
            self.pathguide = Some(PathGuidePipeline::new(device, self.pathguide_config.svo_resolution));
            needs_rebuild = true;
        }
        log::debug!("PathGuide: enabled={}", enabled);
        if needs_rebuild {
            self.rebuild_pathguide_bind_groups(device);
        }
        self.last_pathguide_enabled = enabled;
        self.last_pathguide_svo_resolution = self.pathguide_config.svo_resolution;
    }

    /// Enable/disable adaptive sampling.
    #[allow(dead_code)]
    pub fn set_adaptive_enabled(&mut self, device: &wgpu::Device, queue: &wgpu::Queue, enabled: bool) {
        if enabled && self.adaptive.is_none() {
            log::info!("Adaptive: create pipeline");
            self.adaptive = Some(AdaptivePipeline::new(device, self.width, self.height));
            self.adaptive_config.enabled = true;
            self.rebuild_adaptive_bind_groups(device);
            self.rebuild_wavefront_bind_groups(device);
        } else if !enabled {
            self.adaptive_config.enabled = false;
            self.rebuild_wavefront_bind_groups(device);
        }
        self.fill_sample_map(queue);
        log::debug!("Adaptive: enabled={}", enabled);
    }

    /// Update adaptive sampling configuration.
    pub fn set_adaptive_config(
        &mut self,
        queue: &wgpu::Queue,
        min_spp: u32,
        max_spp: u32,
        variance_threshold: f32,
        update_interval: u32,
    ) {
        let min_spp = min_spp.max(1);
        let mut max_spp = max_spp.max(1);
        if max_spp < min_spp {
            max_spp = min_spp;
        }
        self.adaptive_config.min_spp = min_spp;
        self.adaptive_config.max_spp = max_spp;
        self.adaptive_config.variance_threshold = variance_threshold.max(1e-6);
        self.adaptive_config.update_interval = update_interval.max(1);
        self.fill_sample_map(queue);
    }

    fn fill_sample_map(&mut self, queue: &wgpu::Queue) {
        let sample_map = if let Some(ad) = &self.adaptive {
            ad.sample_map()
        } else {
            &self.sample_map_fallback
        };
        let n = (self.width * self.height).max(1) as usize;
        let fill_value = if self.adaptive_config.enabled {
            self.adaptive_config.max_spp.max(1)
        } else {
            u32::MAX
        };
        let data = vec![fill_value; n];
        queue.write_buffer(sample_map, 0, bytemuck::cast_slice(&data));
    }

    /// Enable/disable wavefront path tracing.
    pub fn set_wavefront_enabled(&mut self, device: &wgpu::Device, enabled: bool) {
        let prev_enabled = self.wavefront_config.enabled;
        if enabled && self.wavefront.is_none() {
            log::info!("Wavefront PT enabled");
            self.wavefront = Some(WavefrontPipeline::new(device, self.width, self.height));
            self.wavefront_config.enabled = true;
            self.rebuild_wavefront_bind_groups(device);
        } else if !enabled {
            self.wavefront_config.enabled = false;
        }
        if prev_enabled != enabled {
            self.frame_count = 0;
            self.history_dirty = true;
        }
        log::debug!("Wavefront: enabled={}", enabled);
    }

    pub fn set_wavefront_tile_size(&mut self, size: u32) {
        self.wavefront_config.tile_size = size;
    }

    pub fn set_wavefront_rr_enabled(&mut self, enabled: bool) {
        self.wavefront_rr_enabled = enabled;
    }

    pub fn set_spectral_options(&mut self, mode: u32, samples: u32, dispersion: u32) {
        self.spectral_mode = mode;
        self.spectral_samples = samples.max(1);
        self.spectral_dispersion = dispersion;
    }

    /// Check if wavefront PT is enabled.
    #[allow(dead_code)]
    pub fn is_wavefront_enabled(&self) -> bool {
        self.wavefront_config.enabled && self.wavefront.is_some()
    }

    /// Rebuild wavefront bind groups after scene upload.
    /// Creates two sets of bind groups for ping-pong ray buffer swapping.
    fn rebuild_wavefront_bind_groups(&mut self, device: &wgpu::Device) {
        let Some(wf) = &self.wavefront else { return; };
        let (Some(nodes), Some(instances), Some(materials)) =
            (&self.nodes_buffer, &self.instances_buffer, &self.materials_buffer) else {
            self.wavefront_bind_groups = None;
            return;
        };
        log::debug!(
            "Wavefront: rebuild bind groups (scene_ready={}, size={}x{})",
            self.scene_ready,
            self.width,
            self.height
        );

        let (raygen_bgl, intersect_bgl, shade_bgl) = wf.bgls();
        let (ray_a, ray_b) = wf.ray_bufs_raw();  // Get both buffers directly
        let count_buf = wf.count_buf();
        let hit_buf = wf.hit_buf();
        let dims_buf = wf.dims_buf();

        #[repr(C)]
        #[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
        struct ShadeParams {
            width: u32,
            height: u32,
            max_bounces: u32,
            frame_count: u32,
            time: f32,
            guide_weight: f32,
            guide_warmup: u32,
            guide_enabled: u32,
            guide_product: u32,
            rr_enabled: u32,
            spectral_mode: u32,
            spectral_samples: u32,
            spectral_dispersion: u32,
            _pad: u32,
        }
        let shade_params = ShadeParams {
            width: self.width,
            height: self.height,
            max_bounces: 8,
            frame_count: self.frame_count,
            time: 0.0,
            guide_weight: self.pathguide_config.guide_weight,
            guide_warmup: self.pathguide_config.warmup_frames,
            guide_enabled: if self.pathguide_config.enabled { 1 } else { 0 },
            guide_product: if self.pathguide_config.product_sampling { 1 } else { 0 },
            rr_enabled: if self.wavefront_rr_enabled { 1 } else { 0 },
            spectral_mode: self.spectral_mode,
            spectral_samples: self.spectral_samples,
            spectral_dispersion: self.spectral_dispersion,
            _pad: 0,
        };
        let shade_params_buf = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("wf_shade_params"),
            contents: bytemuck::bytes_of(&shade_params),
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
        });

        // Helper to create one set of bind groups
        let guide = &self.guide_buffer;
        let create_set = |label_suffix: &str, ray_in: &wgpu::Buffer, ray_out: &wgpu::Buffer| -> WavefrontBindGroupSet {
            // Raygen: writes to ray_in, count_in
            let sample_map = if let Some(ad) = &self.adaptive {
                ad.sample_map()
            } else {
                &self.sample_map_fallback
            };
            let raygen_bg = device.create_bind_group(&wgpu::BindGroupDescriptor {
                label: Some(&format!("wf_raygen_bg_{}", label_suffix)),
                layout: raygen_bgl,
                entries: &[
                    wgpu::BindGroupEntry { binding: 0, resource: self.camera_buffer.as_entire_binding() },
                    wgpu::BindGroupEntry { binding: 1, resource: dims_buf.as_entire_binding() },
                    wgpu::BindGroupEntry { binding: 2, resource: ray_in.as_entire_binding() },
                    wgpu::BindGroupEntry { binding: 3, resource: count_buf.as_entire_binding() },
                    wgpu::BindGroupEntry { binding: 4, resource: sample_map.as_entire_binding() },
                    wgpu::BindGroupEntry { binding: 5, resource: self.accum_buffer.as_entire_binding() },
                ],
            });

            // Intersect: reads ray_in, writes hits
            let intersect_bg = device.create_bind_group(&wgpu::BindGroupDescriptor {
                label: Some(&format!("wf_intersect_bg_{}", label_suffix)),
                layout: intersect_bgl,
                entries: &[
                    wgpu::BindGroupEntry { binding: 0, resource: nodes.as_entire_binding() },
                    wgpu::BindGroupEntry { binding: 1, resource: instances.as_entire_binding() },
                    wgpu::BindGroupEntry { binding: 2, resource: ray_in.as_entire_binding() },
                    wgpu::BindGroupEntry { binding: 3, resource: hit_buf.as_entire_binding() },
                    wgpu::BindGroupEntry { binding: 4, resource: count_buf.as_entire_binding() },
                ],
            });

            // Shade: reads ray_in+hits, writes ray_out+accum, samples env
            let shade_bg = device.create_bind_group(&wgpu::BindGroupDescriptor {
                label: Some(&format!("wf_shade_bg_{}", label_suffix)),
                layout: shade_bgl,
                entries: &[
                    wgpu::BindGroupEntry { binding: 0, resource: instances.as_entire_binding() },
                    wgpu::BindGroupEntry { binding: 1, resource: materials.as_entire_binding() },
                    wgpu::BindGroupEntry { binding: 2, resource: ray_in.as_entire_binding() },
                    wgpu::BindGroupEntry { binding: 3, resource: hit_buf.as_entire_binding() },
                    wgpu::BindGroupEntry { binding: 4, resource: ray_out.as_entire_binding() },
                    wgpu::BindGroupEntry { binding: 5, resource: self.accum_buffer.as_entire_binding() },
                    wgpu::BindGroupEntry { binding: 6, resource: count_buf.as_entire_binding() },
                    wgpu::BindGroupEntry { binding: 7, resource: shade_params_buf.as_entire_binding() },
                    wgpu::BindGroupEntry { binding: 8, resource: wgpu::BindingResource::TextureView(&self.env_view) },
                    wgpu::BindGroupEntry { binding: 9, resource: wgpu::BindingResource::Sampler(&self.env_sampler) },
                    wgpu::BindGroupEntry { binding: 10, resource: self.env_uniform_buffer.as_entire_binding() },
                    wgpu::BindGroupEntry { binding: 11, resource: guide.as_entire_binding() },
                ],
            });

            WavefrontBindGroupSet { raygen_bg, intersect_bg, shade_bg }
        };

        // Set A: ray_a -> hits -> ray_b
        let set_a = create_set("a", ray_a, ray_b);
        // Set B: ray_b -> hits -> ray_a
        let set_b = create_set("b", ray_b, ray_a);

        // Count swap: count_out -> count_in (shared)
        let count_swap_bg = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("wf_count_swap_bg"),
            layout: wf.count_swap_bgl(),
            entries: &[
                wgpu::BindGroupEntry { binding: 0, resource: count_buf.as_entire_binding() },
            ],
        });

        // Finalize: accum -> output texture (same for both sets)
        let w = self.width.max(1);
        let h = self.height.max(1);
        let finalize_params_buf = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("wf_finalize_params"),
            contents: bytemuck::bytes_of(&[w, h, self.frame_count, 0u32]),
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
        });
        let finalize_bg = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("wf_finalize_bg"),
            layout: wf.finalize_bgl(),
            entries: &[
                wgpu::BindGroupEntry { binding: 0, resource: self.accum_buffer.as_entire_binding() },
                wgpu::BindGroupEntry { binding: 1, resource: wgpu::BindingResource::TextureView(&self.output_view) },
                wgpu::BindGroupEntry { binding: 2, resource: finalize_params_buf.as_entire_binding() },
            ],
        });

        self.wavefront_bind_groups = Some(WavefrontBindGroups {
            set_a,
            set_b,
            count_swap_bg,
            finalize_bg,
            shade_params_buf,
            finalize_params_buf,
            cur_set: 0,
        });
        log::debug!("Wavefront: bind groups ready");
    }

    /// Rebuild adaptive sampling bind groups.
    fn rebuild_adaptive_bind_groups(&mut self, device: &wgpu::Device) {
        let Some(ad) = &self.adaptive else {
            self.adaptive_bind_groups = None;
            return;
        };
        log::debug!("Adaptive: rebuild bind groups (size={}x{})", self.width, self.height);

        #[repr(C)]
        #[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
        struct VarianceParams {
            width: u32,
            height: u32,
            _pad: [u32; 2],
        }
        #[repr(C)]
        #[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
        struct AllocateParams {
            width: u32,
            height: u32,
            min_spp: u32,
            max_spp: u32,
            variance_threshold: f32,
            _pad: [f32; 3],
            _pad2: [f32; 4],
        }

        let variance_params = VarianceParams { width: self.width, height: self.height, _pad: [0; 2] };
        let variance_params_buf = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("adaptive_variance_params"),
            contents: bytemuck::bytes_of(&variance_params),
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
        });

        let allocate_params = AllocateParams {
            width: self.width,
            height: self.height,
            min_spp: self.adaptive_config.min_spp,
            max_spp: self.adaptive_config.max_spp,
            variance_threshold: self.adaptive_config.variance_threshold,
            _pad: [0.0; 3],
            _pad2: [0.0; 4],
        };
        let allocate_params_buf = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("adaptive_allocate_params"),
            contents: bytemuck::bytes_of(&allocate_params),
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
        });

        let (variance_bgl, allocate_bgl) = ad.bgls();

        let variance_bg = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("adaptive_variance_bg"),
            layout: variance_bgl,
            entries: &[
                wgpu::BindGroupEntry { binding: 0, resource: self.accum_buffer.as_entire_binding() },
                wgpu::BindGroupEntry { binding: 1, resource: ad.variance_buffer().as_entire_binding() },
                wgpu::BindGroupEntry { binding: 2, resource: variance_params_buf.as_entire_binding() },
            ],
        });

        let allocate_bg = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("adaptive_allocate_bg"),
            layout: allocate_bgl,
            entries: &[
                wgpu::BindGroupEntry { binding: 0, resource: ad.variance_buffer().as_entire_binding() },
                wgpu::BindGroupEntry { binding: 1, resource: ad.sample_map().as_entire_binding() },
                wgpu::BindGroupEntry { binding: 2, resource: allocate_params_buf.as_entire_binding() },
            ],
        });

        self.adaptive_bind_groups = Some(AdaptiveBindGroups {
            variance_bg,
            allocate_bg,
            variance_params_buf,
            allocate_params_buf,
        });
    }

    fn rebuild_restir_bind_groups(&mut self, device: &wgpu::Device) {
        let Some(rs) = &self.restir else {
            self.restir_bind_groups = None;
            return;
        };
        let Some(wf) = &self.wavefront else {
            self.restir_bind_groups = None;
            return;
        };
        let (Some(instances_buf), Some(materials_buf)) = (&self.instances_buffer, &self.materials_buffer) else {
            self.restir_bind_groups = None;
            return;
        };
        let sample_map = if let Some(ad) = &self.adaptive {
            ad.sample_map()
        } else {
            &self.sample_map_fallback
        };
        log::debug!("ReSTIR: rebuild bind groups (size={}x{})", self.width, self.height);

        let hit_buf = wf.hit_buf();
        let (ray_a, ray_b) = wf.ray_bufs_raw();

        // Ensure gbuffer pipeline/layout
        if self.gbuffer_pipeline.is_none() || self.gbuffer_bgl.is_none() {
            let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
                label: Some("wf_gbuffer_shader"),
                source: wgpu::ShaderSource::Wgsl(GBUFFER_WGSL.into()),
            });
            let bgl = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
                label: Some("wf_gbuffer_bgl"),
                entries: &[
                    bgl_storage_ro(0),   // rays
                    bgl_storage_ro(1),   // hits
                    bgl_storage_rw(2),   // depth
                    bgl_storage_rw(3),   // normal
                    bgl_storage_rw(4),   // motion
                    bgl_uniform(5),      // params
                ],
            });
            let pl = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
                label: Some("wf_gbuffer_pl"),
                bind_group_layouts: &[&bgl],
                push_constant_ranges: &[],
            });
            let pipeline = device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
                label: Some("wf_gbuffer_pipeline"),
                layout: Some(&pl),
                module: &shader,
                entry_point: Some("main"),
                compilation_options: Default::default(),
                cache: None,
            });
            self.gbuffer_pipeline = Some(pipeline);
            self.gbuffer_bgl = Some(bgl);
        }

        let gbuffer_params = GBufferParams {
            width: self.width,
            height: self.height,
            _pad0: [0; 2],
            prev_view_proj: self.last_view_proj.unwrap_or([[0.0; 4]; 4]),
            curr_view_proj: self.last_view_proj.unwrap_or([[0.0; 4]; 4]),
        };
        let gbuffer_params_buf = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("wf_gbuffer_params"),
            contents: bytemuck::bytes_of(&gbuffer_params),
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
        });

        let gbuffer_bgl = self.gbuffer_bgl.as_ref().unwrap();
        let gbuffer_bg_a = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("wf_gbuffer_bg_a"),
            layout: gbuffer_bgl,
            entries: &[
                wgpu::BindGroupEntry { binding: 0, resource: ray_a.as_entire_binding() },
                wgpu::BindGroupEntry { binding: 1, resource: hit_buf.as_entire_binding() },
                wgpu::BindGroupEntry { binding: 2, resource: rs.depth_buffer().as_entire_binding() },
                wgpu::BindGroupEntry { binding: 3, resource: rs.normal_buffer().as_entire_binding() },
                wgpu::BindGroupEntry { binding: 4, resource: rs.motion_buffer().as_entire_binding() },
                wgpu::BindGroupEntry { binding: 5, resource: gbuffer_params_buf.as_entire_binding() },
            ],
        });
        let gbuffer_bg_b = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("wf_gbuffer_bg_b"),
            layout: gbuffer_bgl,
            entries: &[
                wgpu::BindGroupEntry { binding: 0, resource: ray_b.as_entire_binding() },
                wgpu::BindGroupEntry { binding: 1, resource: hit_buf.as_entire_binding() },
                wgpu::BindGroupEntry { binding: 2, resource: rs.depth_buffer().as_entire_binding() },
                wgpu::BindGroupEntry { binding: 3, resource: rs.normal_buffer().as_entire_binding() },
                wgpu::BindGroupEntry { binding: 4, resource: rs.motion_buffer().as_entire_binding() },
                wgpu::BindGroupEntry { binding: 5, resource: gbuffer_params_buf.as_entire_binding() },
            ],
        });

        let (initial_pl, temporal_pl, spatial_pl, shade_pl) = rs.pipelines();
        let (initial_bgl, temporal_bgl, spatial_bgl, shade_bgl) = rs.bgls();

        let initial_params = RestirInitialParams {
            width: self.width,
            height: self.height,
            frame_count: self.frame_count,
            num_candidates: self.restir_config.initial_candidates,
        };
        let initial_params_buf = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("restir_initial_params"),
            contents: bytemuck::bytes_of(&initial_params),
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
        });

        let temporal_params = RestirTemporalParams {
            width: self.width,
            height: self.height,
            frame_count: self.frame_count,
            m_max: self.restir_config.m_max,
            depth_threshold: 0.1,
            _pad: [0.0; 3],
            _pad2: [0.0; 4],
        };
        let temporal_params_buf = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("restir_temporal_params"),
            contents: bytemuck::bytes_of(&temporal_params),
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
        });

        let spatial_params = RestirSpatialParams {
            width: self.width,
            height: self.height,
            frame_count: self.frame_count,
            num_neighbors: self.restir_config.spatial_neighbors,
            radius: self.restir_config.spatial_radius,
            normal_threshold: 0.5,
            depth_threshold: 0.1,
            _pad: 0.0,
        };
        let spatial_params_buf = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("restir_spatial_params"),
            contents: bytemuck::bytes_of(&spatial_params),
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
        });

        let cam_pos = self.last_camera_pos.unwrap_or([0.0, 0.0, 0.0]);
        let shade_params = RestirShadeParams {
            width: self.width,
            height: self.height,
            frame_count: self.frame_count,
            _pad: 0,
            camera_pos: cam_pos,
            _pad2: 0.0,
        };
        let shade_params_buf = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("restir_shade_params"),
            contents: bytemuck::bytes_of(&shade_params),
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
        });

        let (cur_res, prev_res) = rs.reservoirs();
        let prev_depth_buf = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("restir_prev_depth"),
            size: (self.width * self.height).max(1) as u64 * 4,
            usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::COPY_SRC,
            mapped_at_creation: false,
        });

        let initial_bg = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("restir_initial_bg"),
            layout: initial_bgl,
            entries: &[
                wgpu::BindGroupEntry { binding: 0, resource: hit_buf.as_entire_binding() },
                wgpu::BindGroupEntry { binding: 1, resource: cur_res.as_entire_binding() },
                wgpu::BindGroupEntry { binding: 2, resource: initial_params_buf.as_entire_binding() },
                wgpu::BindGroupEntry { binding: 3, resource: wgpu::BindingResource::TextureView(&self.env_view) },
                wgpu::BindGroupEntry { binding: 4, resource: wgpu::BindingResource::Sampler(&self.env_sampler) },
                wgpu::BindGroupEntry { binding: 5, resource: self.env_uniform_buffer.as_entire_binding() },
                wgpu::BindGroupEntry { binding: 6, resource: self.env_marginal_cdf.as_entire_binding() },
                wgpu::BindGroupEntry { binding: 7, resource: self.env_conditional_cdf.as_entire_binding() },
            ],
        });

        let temporal_bg = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("restir_temporal_bg"),
            layout: temporal_bgl,
            entries: &[
                wgpu::BindGroupEntry { binding: 0, resource: prev_res.as_entire_binding() },
                wgpu::BindGroupEntry { binding: 1, resource: cur_res.as_entire_binding() },
                wgpu::BindGroupEntry { binding: 2, resource: rs.motion_buffer().as_entire_binding() },
                wgpu::BindGroupEntry { binding: 3, resource: prev_depth_buf.as_entire_binding() },
                wgpu::BindGroupEntry { binding: 4, resource: rs.depth_buffer().as_entire_binding() },
                wgpu::BindGroupEntry { binding: 5, resource: temporal_params_buf.as_entire_binding() },
            ],
        });

        let spatial_bg = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("restir_spatial_bg"),
            layout: spatial_bgl,
            entries: &[
                wgpu::BindGroupEntry { binding: 0, resource: cur_res.as_entire_binding() },
                wgpu::BindGroupEntry { binding: 1, resource: prev_res.as_entire_binding() },
                wgpu::BindGroupEntry { binding: 2, resource: rs.depth_buffer().as_entire_binding() },
                wgpu::BindGroupEntry { binding: 3, resource: rs.normal_buffer().as_entire_binding() },
                wgpu::BindGroupEntry { binding: 4, resource: spatial_params_buf.as_entire_binding() },
            ],
        });

        let build_shade_bg = |label: &str, res: &wgpu::Buffer, rays: &wgpu::Buffer| {
            device.create_bind_group(&wgpu::BindGroupDescriptor {
                label: Some(label),
                layout: shade_bgl,
                entries: &[
                    wgpu::BindGroupEntry { binding: 0, resource: res.as_entire_binding() },
                    wgpu::BindGroupEntry { binding: 1, resource: hit_buf.as_entire_binding() },
                    wgpu::BindGroupEntry { binding: 2, resource: self.accum_buffer.as_entire_binding() },
                    wgpu::BindGroupEntry { binding: 3, resource: shade_params_buf.as_entire_binding() },
                    wgpu::BindGroupEntry { binding: 4, resource: instances_buf.as_entire_binding() },
                    wgpu::BindGroupEntry { binding: 5, resource: materials_buf.as_entire_binding() },
                    wgpu::BindGroupEntry { binding: 6, resource: sample_map.as_entire_binding() },
                    wgpu::BindGroupEntry { binding: 7, resource: rays.as_entire_binding() },
                    wgpu::BindGroupEntry { binding: 8, resource: wgpu::BindingResource::TextureView(&self.env_view) },
                    wgpu::BindGroupEntry { binding: 9, resource: wgpu::BindingResource::Sampler(&self.env_sampler) },
                    wgpu::BindGroupEntry { binding: 10, resource: self.env_uniform_buffer.as_entire_binding() },
                ],
            })
        };

        let shade_bg_cur_a = build_shade_bg("restir_shade_bg_cur_a", cur_res, ray_a);
        let shade_bg_cur_b = build_shade_bg("restir_shade_bg_cur_b", cur_res, ray_b);
        let shade_bg_prev_a = build_shade_bg("restir_shade_bg_prev_a", prev_res, ray_a);
        let shade_bg_prev_b = build_shade_bg("restir_shade_bg_prev_b", prev_res, ray_b);

        let _ = (initial_pl, temporal_pl, spatial_pl, shade_pl);

        self.restir_bind_groups = Some(ReSTIRBindGroups {
            gbuffer_bg_a,
            gbuffer_bg_b,
            gbuffer_params_buf,
            initial_bg,
            temporal_bg,
            spatial_bg,
            shade_bg_cur_a,
            shade_bg_cur_b,
            shade_bg_prev_a,
            shade_bg_prev_b,
            initial_params_buf,
            temporal_params_buf,
            spatial_params_buf,
            shade_params_buf,
            prev_depth_buf,
        });
    }

    fn rebuild_pathguide_bind_groups(&mut self, device: &wgpu::Device) {
        if !self.pathguide_config.enabled {
            self.pathguide_bind_groups = None;
            return;
        }
        let Some(pg) = &self.pathguide else {
            self.pathguide_bind_groups = None;
            return;
        };
        log::debug!(
            "PathGuide: rebuild bind groups (size={}x{}, res={})",
            self.width,
            self.height,
            self.pathguide_config.svo_resolution
        );

        let (update_bgl, sample_bgl) = pg.bgls();

        let update_params = PathGuideUpdateParams {
            scene_min: {
                let v = self.scene_bounds.map(|b| b.0).unwrap_or([0.0; 3]);
                [v[0], v[1], v[2], 0.0]
            },
            scene_max: {
                let v = self.scene_bounds.map(|b| b.1).unwrap_or([1.0; 3]);
                [v[0], v[1], v[2], 0.0]
            },
            params0: [self.pathguide_config.svo_resolution, (self.width * self.height).max(1), 0, 0],
            params1: [0.95, 0.0, 0.0, 0.0],
        };
        let update_params_buf = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("pathguide_update_params"),
            contents: bytemuck::bytes_of(&update_params),
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
        });

        let sample_params = PathGuideSampleParams {
            scene_min: {
                let v = self.scene_bounds.map(|b| b.0).unwrap_or([0.0; 3]);
                [v[0], v[1], v[2], 0.0]
            },
            scene_max: {
                let v = self.scene_bounds.map(|b| b.1).unwrap_or([1.0; 3]);
                [v[0], v[1], v[2], 0.0]
            },
            params0: [self.pathguide_config.svo_resolution, self.frame_count, 0, 0],
            params1: [self.pathguide_config.guide_weight, 0.0, 0.0, 0.0],
        };
        let sample_params_buf = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("pathguide_sample_params"),
            contents: bytemuck::bytes_of(&sample_params),
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
        });

        let guide = &self.guide_buffer;
        let update_bg = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("pathguide_update_bg"),
            layout: update_bgl,
            entries: &[
                wgpu::BindGroupEntry { binding: 0, resource: pg.svo_buffer().as_entire_binding() },
                wgpu::BindGroupEntry { binding: 1, resource: guide.as_entire_binding() },
                wgpu::BindGroupEntry { binding: 2, resource: update_params_buf.as_entire_binding() },
            ],
        });
        let sample_bg = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("pathguide_sample_bg"),
            layout: sample_bgl,
            entries: &[
                wgpu::BindGroupEntry { binding: 0, resource: pg.svo_buffer().as_entire_binding() },
                wgpu::BindGroupEntry { binding: 1, resource: guide.as_entire_binding() },
                wgpu::BindGroupEntry { binding: 2, resource: sample_params_buf.as_entire_binding() },
            ],
        });

        self.pathguide_bind_groups = Some(PathGuideBindGroups {
            update_bg,
            sample_bg,
            update_params_buf,
            sample_params_buf,
        });
    }

    /// Dispatch wavefront path tracer passes.
    /// Returns false if not ready.
    pub fn dispatch_wavefront(&mut self, device: &wgpu::Device, encoder: &mut wgpu::CommandEncoder, queue: &wgpu::Queue, max_bounces: u32, time: f32) -> bool {
        // Early checks
        if self.wavefront.is_none() || self.wavefront_bind_groups.is_none() { return false; }
        if !self.scene_ready { return false; }
        if self.frame_count >= self.max_samples { return true; }

        let wf_start = std::time::Instant::now();
        // Clear accum buffer on first frame
        if self.frame_count == 0 {
            encoder.clear_buffer(&self.accum_buffer, 0, None);
            if self.history_dirty {
                // Clear ReSTIR history on jump to avoid stale temporal reuse
                if let (Some(rs), Some(restir_bgs)) = (&self.restir, &self.restir_bind_groups) {
                    let res_size = (self.width * self.height).max(1) as u64 * Reservoir::SIZE as u64;
                    let (cur_res, prev_res) = rs.reservoirs();
                    encoder.clear_buffer(cur_res, 0, Some(res_size));
                    encoder.clear_buffer(prev_res, 0, Some(res_size));
                    let depth_size = (self.width * self.height).max(1) as u64 * 4;
                    encoder.clear_buffer(&restir_bgs.prev_depth_buf, 0, Some(depth_size));
                }
                // Clear path guiding state on jump
                if let Some(pg) = &self.pathguide {
                    encoder.clear_buffer(pg.svo_buffer(), 0, None);
                }
                Self::clear_guide_buffer(encoder, &self.guide_buffer);
                self.history_dirty = false;
            }
        }

        self.frame_count += 1;

        let full_w = self.width.max(1);
        let full_h = self.height.max(1);
        let tile_size = self.wavefront_config.tile_size;
        let mut use_tiling = tile_size > 0 && (full_w > tile_size || full_h > tile_size);
        let tile_capacity_w = if use_tiling { tile_size.min(full_w).max(1) } else { full_w };
        let tile_capacity_h = if use_tiling { tile_size.min(full_h).max(1) } else { full_h };

        if let Some(wf) = &mut self.wavefront {
            let (wf_w, wf_h) = wf.dimensions();
            if wf_w != tile_capacity_w || wf_h != tile_capacity_h {
                wf.resize(device, tile_capacity_w, tile_capacity_h);
                self.rebuild_wavefront_bind_groups(device);
            }
        }

        let wf = self.wavefront.as_ref().unwrap();
        let (wf_w, wf_h) = wf.dimensions();

        // CRITICAL: If wavefront dimensions were clamped below tile capacity, 
        // we MUST enable tiling to avoid out-of-bounds ray buffer access.
        // This can happen when storage buffer limits are exceeded.
        if (wf_w < tile_capacity_w || wf_h < tile_capacity_h)
            && !use_tiling {
                log::warn!(
                    "WF dispatch: forcing tiling due to buffer clamping (requested {}x{}, got {}x{})",
                    tile_capacity_w, tile_capacity_h, wf_w, wf_h
                );
                use_tiling = true;
            }

        // Get current bind group set (will swap between bounces)
        let bgs = self.wavefront_bind_groups.as_ref().unwrap();
        let cur_set = bgs.cur_set;

        // Update shade params with current frame and time
        #[repr(C)]
        #[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
        struct ShadeParams {
            width: u32,
            height: u32,
            max_bounces: u32,
            frame_count: u32,
            time: f32,
            guide_weight: f32,
            guide_warmup: u32,
            guide_enabled: u32,
            guide_product: u32,
            rr_enabled: u32,
            spectral_mode: u32,
            spectral_samples: u32,
            spectral_dispersion: u32,
            _pad: u32,
        }
        let guide_enabled = if use_tiling { 0 } else if self.pathguide_config.enabled { 1 } else { 0 };
        let guide_product = if use_tiling { 0 } else if self.pathguide_config.product_sampling { 1 } else { 0 };
        let params = ShadeParams {
            width: full_w,
            height: full_h,
            max_bounces,
            frame_count: self.frame_count,
            time,
            guide_weight: self.pathguide_config.guide_weight,
            guide_warmup: self.pathguide_config.warmup_frames,
            guide_enabled,
            guide_product,
            rr_enabled: if self.wavefront_rr_enabled { 1 } else { 0 },
            spectral_mode: self.spectral_mode,
            spectral_samples: self.spectral_samples,
            spectral_dispersion: self.spectral_dispersion,
            _pad: 0,
        };
        queue.write_buffer(&bgs.shade_params_buf, 0, bytemuck::bytes_of(&params));

        let (raygen_pl, intersect_pl, shade_pl) = wf.pipelines();

        let mut restir_enabled = (self.restir_config.di_enabled || self.restir_config.gi_enabled)
            && self.restir.is_some()
            && self.restir_bind_groups.is_some()
            && self.gbuffer_pipeline.is_some();
        let mut pathguide_enabled = self.pathguide_config.enabled;
        let mut adaptive_enabled = self.adaptive_config.enabled;
        if use_tiling {
            if restir_enabled {
                log::warn!("WF tiling: ReSTIR disabled (tile_size={})", tile_size);
            }
            if pathguide_enabled {
                log::warn!("WF tiling: Path Guide disabled (tile_size={})", tile_size);
            }
            if adaptive_enabled {
                log::warn!("WF tiling: Adaptive Sampling disabled (tile_size={})", tile_size);
            }
            restir_enabled = false;
            pathguide_enabled = false;
            adaptive_enabled = false;
        }
        log::debug!(
            "WF dispatch: frame {}/{}, full={}x{}, wf_buf={}x{}, tile={}, bounces={}, restir={}, pathguide={}, adaptive={}",
            self.frame_count,
            self.max_samples,
            full_w,
            full_h,
            wf_w,
            wf_h,
            tile_size,
            max_bounces,
            restir_enabled,
            pathguide_enabled,
            adaptive_enabled
        );

        // Get the starting bind group set
        let start_set = if cur_set == 0 { &bgs.set_a } else { &bgs.set_b };

        let count_buf = wf.count_buf();
        let mut tile_y = 0u32;
        // Use actual wavefront dimensions for step size to respect buffer limits
        let step_y = if use_tiling { wf_h.min(tile_capacity_h) } else { full_h };
        let step_x = if use_tiling { wf_w.min(tile_capacity_w) } else { full_w };
        while tile_y < full_h {
            let mut tile_x = 0u32;
            // Clamp tile dimensions to actual wavefront buffer size
            let tile_h = (full_h - tile_y).min(wf_h);
            while tile_x < full_w {
                let tile_w = (full_w - tile_x).min(wf_w);
                let tile_pixels = tile_w * tile_h;

                let dims = WfDims {
                    full_width: full_w,
                    full_height: full_h,
                    tile_width: tile_w,
                    tile_height: tile_h,
                    tile_x,
                    tile_y,
                    _pad: [0, 0],
                };
                wf.write_dims(queue, &dims);

                // Path guiding: sample guided directions from previous SVO
                if pathguide_enabled && !restir_enabled {
                    if let (Some(pg), Some(pg_bgs)) = (&self.pathguide, &self.pathguide_bind_groups) {
                        let sample_params = PathGuideSampleParams {
                            scene_min: {
                                let v = self.scene_bounds.map(|b| b.0).unwrap_or([0.0; 3]);
                                [v[0], v[1], v[2], 0.0]
                            },
                            scene_max: {
                                let v = self.scene_bounds.map(|b| b.1).unwrap_or([1.0; 3]);
                                [v[0], v[1], v[2], 0.0]
                            },
                            params0: [self.pathguide_config.svo_resolution, self.frame_count, 0, 0],
                            params1: [self.pathguide_config.guide_weight, 0.0, 0.0, 0.0],
                        };
                        queue.write_buffer(&pg_bgs.sample_params_buf, 0, bytemuck::bytes_of(&sample_params));
                        let (_, sample_pl) = pg.pipelines();
                        let wg = tile_pixels.max(1).div_ceil(64);
                        let mut pass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor {
                            label: Some("pathguide_sample_pass"),
                            timestamp_writes: None,
                        });
                        pass.set_pipeline(sample_pl);
                        pass.set_bind_group(0, &pg_bgs.sample_bg, &[]);
                        pass.dispatch_workgroups(wg, 1, 1);
                        log::trace!("WF pathguide sample: wg={}", wg);
                    }
                }

                // Initialize counts buffer (count_in + count_out) before raygen
                queue.write_buffer(count_buf, 0, bytemuck::bytes_of(&[tile_pixels, 0u32, 0u32, 0u32]));

                // Pass 1: Generate camera rays (always uses the current set's raygen)
                {
                    let mut pass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor {
                        label: Some("wf_raygen_pass"),
                        timestamp_writes: None,
                    });
                    pass.set_pipeline(raygen_pl);
                    pass.set_bind_group(0, &start_set.raygen_bg, &[]);
                    let wg_x = tile_w.div_ceil(8);
                    let wg_y = tile_h.div_ceil(8);
                    pass.dispatch_workgroups(wg_x, wg_y, 1);
                    log::trace!("WF raygen: wg=({}, {})", wg_x, wg_y);
                }

                if restir_enabled {
                    let rs = self.restir.as_ref().unwrap();
                    let restir_bgs = self.restir_bind_groups.as_ref().unwrap();
                    let gbuffer_pl = self.gbuffer_pipeline.as_ref().unwrap();

                    let initial_params = RestirInitialParams {
                        width: tile_w,
                        height: tile_h,
                        frame_count: self.frame_count,
                        num_candidates: self.restir_config.initial_candidates,
                    };
                    queue.write_buffer(&restir_bgs.initial_params_buf, 0, bytemuck::bytes_of(&initial_params));

                    let temporal_params = RestirTemporalParams {
                        width: tile_w,
                        height: tile_h,
                        frame_count: self.frame_count,
                        m_max: self.restir_config.m_max,
                        depth_threshold: 0.1,
                        _pad: [0.0; 3],
                        _pad2: [0.0; 4],
                    };
                    queue.write_buffer(&restir_bgs.temporal_params_buf, 0, bytemuck::bytes_of(&temporal_params));

                    let spatial_params = RestirSpatialParams {
                        width: tile_w,
                        height: tile_h,
                        frame_count: self.frame_count,
                        num_neighbors: self.restir_config.spatial_neighbors,
                        radius: self.restir_config.spatial_radius,
                        normal_threshold: 0.5,
                        depth_threshold: 0.1,
                        _pad: 0.0,
                    };
                    queue.write_buffer(&restir_bgs.spatial_params_buf, 0, bytemuck::bytes_of(&spatial_params));

                    let cam_pos = self.last_camera_pos.unwrap_or([0.0, 0.0, 0.0]);
                    let shade_params = RestirShadeParams {
                        width: tile_w,
                        height: tile_h,
                        frame_count: self.frame_count,
                        _pad: 0,
                        camera_pos: cam_pos,
                        _pad2: 0.0,
                    };
                    queue.write_buffer(&restir_bgs.shade_params_buf, 0, bytemuck::bytes_of(&shade_params));

                    // Pass 2: Intersect (primary rays)
                    {
                        let mut pass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor {
                            label: Some("wf_intersect_pass"),
                            timestamp_writes: None,
                        });
                        pass.set_pipeline(intersect_pl);
                        pass.set_bind_group(0, &start_set.intersect_bg, &[]);
                        let wg = tile_pixels.div_ceil(64);
                        pass.dispatch_workgroups(wg, 1, 1);
                        log::trace!("WF intersect (primary): wg={}", wg);
                    }

                    // Pass 3: G-buffer (depth/normal/motion)
                    {
                        let gbuffer_bg = if cur_set == 0 { &restir_bgs.gbuffer_bg_a } else { &restir_bgs.gbuffer_bg_b };
                        let mut pass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor {
                            label: Some("wf_gbuffer_pass"),
                            timestamp_writes: None,
                        });
                        pass.set_pipeline(gbuffer_pl);
                        pass.set_bind_group(0, gbuffer_bg, &[]);
                        let wg_x = tile_w.div_ceil(8);
                        let wg_y = tile_h.div_ceil(8);
                        pass.dispatch_workgroups(wg_x, wg_y, 1);
                        log::trace!("WF gbuffer: wg=({}, {})", wg_x, wg_y);
                    }

                    let (initial_pl, temporal_pl, spatial_pl, shade_pl) = rs.pipelines();
                    let wg_x = tile_w.div_ceil(8);
                    let wg_y = tile_h.div_ceil(8);

                    // Pass 4: ReSTIR initial
                    {
                        let mut pass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor {
                            label: Some("restir_initial_pass"),
                            timestamp_writes: None,
                        });
                        pass.set_pipeline(initial_pl);
                        pass.set_bind_group(0, &restir_bgs.initial_bg, &[]);
                        pass.dispatch_workgroups(wg_x, wg_y, 1);
                        log::trace!("ReSTIR initial: wg=({}, {})", wg_x, wg_y);
                    }

                    // Pass 5: Temporal reuse
                    if self.restir_config.temporal && self.frame_count > 1 {
                        let mut tpass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor {
                            label: Some("restir_temporal_pass"),
                            timestamp_writes: None,
                        });
                        tpass.set_pipeline(temporal_pl);
                        tpass.set_bind_group(0, &restir_bgs.temporal_bg, &[]);
                        tpass.dispatch_workgroups(wg_x, wg_y, 1);
                        log::trace!("ReSTIR temporal: wg=({}, {})", wg_x, wg_y);
                    }

                    // Pass 6: Spatial reuse
                    if self.restir_config.spatial {
                        let mut spass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor {
                            label: Some("restir_spatial_pass"),
                            timestamp_writes: None,
                        });
                        spass.set_pipeline(spatial_pl);
                        spass.set_bind_group(0, &restir_bgs.spatial_bg, &[]);
                        spass.dispatch_workgroups(wg_x, wg_y, 1);
                        log::trace!("ReSTIR spatial: wg=({}, {})", wg_x, wg_y);
                    }

                    // Pass 7: Final shading
                    {
                        let shade_bg = match (self.restir_config.spatial, cur_set) {
                            (true, 0) => &restir_bgs.shade_bg_prev_a,
                            (true, _) => &restir_bgs.shade_bg_prev_b,
                            (false, 0) => &restir_bgs.shade_bg_cur_a,
                            (false, _) => &restir_bgs.shade_bg_cur_b,
                        };
                        let mut spass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor {
                            label: Some("restir_shade_pass"),
                            timestamp_writes: None,
                        });
                        spass.set_pipeline(shade_pl);
                        spass.set_bind_group(0, shade_bg, &[]);
                        spass.dispatch_workgroups(wg_x, wg_y, 1);
                        log::trace!("ReSTIR shade: wg=({}, {})", wg_x, wg_y);
                    }

                    // If spatial is disabled, keep temporal history by copying current to prev
                    if self.restir_config.temporal && !self.restir_config.spatial {
                        let (cur_res, prev_res) = rs.reservoirs();
                        let res_size = (tile_w * tile_h).max(1) as u64 * Reservoir::SIZE as u64;
                        encoder.copy_buffer_to_buffer(cur_res, 0, prev_res, 0, res_size);
                    }

                    // Update previous depth for temporal reprojection
                    let depth_size = (tile_w * tile_h).max(1) as u64 * 4;
                    encoder.copy_buffer_to_buffer(rs.depth_buffer(), 0, &restir_bgs.prev_depth_buf, 0, depth_size);
                } else {
                    // Bounce loop with ping-pong
                    let mut use_set_a = cur_set == 0;
                    for _bounce in 0..max_bounces {
                        let current_set = if use_set_a { &bgs.set_a } else { &bgs.set_b };

                        // Pass 2: Intersect (reads from current ray buffer)
                        {
                            let mut pass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor {
                                label: Some("wf_intersect_pass"),
                                timestamp_writes: None,
                            });
                            pass.set_pipeline(intersect_pl);
                            pass.set_bind_group(0, &current_set.intersect_bg, &[]);
                            let wg = tile_pixels.div_ceil(64);
                            pass.dispatch_workgroups(wg, 1, 1);
                            log::trace!("WF intersect: wg={}", wg);
                        }

                        // Pass 3: Shade (reads current rays+hits, writes to other ray buffer)
                        {
                            let mut pass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor {
                                label: Some("wf_shade_pass"),
                                timestamp_writes: None,
                            });
                            pass.set_pipeline(shade_pl);
                            pass.set_bind_group(0, &current_set.shade_bg, &[]);
                            let wg = tile_pixels.div_ceil(64);
                            pass.dispatch_workgroups(wg, 1, 1);
                            log::trace!("WF shade: wg={}", wg);
                        }

                        // Swap count_in/count_out for next bounce
                        {
                            let mut pass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor {
                                label: Some("wf_count_swap_pass"),
                                timestamp_writes: None,
                            });
                            pass.set_pipeline(wf.count_swap_pipeline());
                            pass.set_bind_group(0, &bgs.count_swap_bg, &[]);
                            pass.dispatch_workgroups(1, 1, 1);
                            log::trace!("WF count swap");
                        }

                        // Swap for next bounce
                        use_set_a = !use_set_a;
                    }

                    // Path guiding: update SVO with latest samples
                    if pathguide_enabled {
                        if let (Some(pg), Some(pg_bgs)) = (&self.pathguide, &self.pathguide_bind_groups) {
                            let update_params = PathGuideUpdateParams {
                                scene_min: {
                                    let v = self.scene_bounds.map(|b| b.0).unwrap_or([0.0; 3]);
                                    [v[0], v[1], v[2], 0.0]
                                },
                                scene_max: {
                                    let v = self.scene_bounds.map(|b| b.1).unwrap_or([1.0; 3]);
                                    [v[0], v[1], v[2], 0.0]
                                },
                                params0: [self.pathguide_config.svo_resolution, tile_pixels.max(1), 0, 0],
                                params1: [0.95, 0.0, 0.0, 0.0],
                            };
                            queue.write_buffer(&pg_bgs.update_params_buf, 0, bytemuck::bytes_of(&update_params));
                            let (update_pl, _) = pg.pipelines();
                            let wg = tile_pixels.max(1).div_ceil(64);
                            let mut pass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor {
                                label: Some("pathguide_update_pass"),
                                timestamp_writes: None,
                            });
                            pass.set_pipeline(update_pl);
                            pass.set_bind_group(0, &pg_bgs.update_bg, &[]);
                            pass.dispatch_workgroups(wg, 1, 1);
                            log::trace!("WF pathguide update: wg={}", wg);
                        }
                    }
                }

                if !use_tiling { break; }
                tile_x = tile_x.saturating_add(step_x);
            }

            if !use_tiling { break; }
            tile_y = tile_y.saturating_add(step_y);
        }

        // Pass 4: Finalize - copy accum to output texture
        {
            let bgs = self.wavefront_bind_groups.as_ref().unwrap();

            // Update finalize params with current frame count
            queue.write_buffer(&bgs.finalize_params_buf, 0, bytemuck::bytes_of(&[full_w, full_h, self.frame_count, 0u32]));

            let mut pass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor {
                label: Some("wf_finalize_pass"),
                timestamp_writes: None,
            });
            pass.set_pipeline(wf.finalize_pipeline());
            pass.set_bind_group(0, &bgs.finalize_bg, &[]);
            let wg_x = full_w.div_ceil(8);
            let wg_y = full_h.div_ceil(8);
            pass.dispatch_workgroups(wg_x, wg_y, 1);
            log::trace!("WF finalize: wg=({}, {})", wg_x, wg_y);
        }

        // Adaptive sampling update (variance + allocation)
        if adaptive_enabled {
            if let (Some(ad), Some(ad_bgs)) = (&self.adaptive, &self.adaptive_bind_groups) {
                #[repr(C)]
                #[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
                struct VarianceParams {
                    width: u32,
                    height: u32,
                    _pad: [u32; 2],
                }
                #[repr(C)]
                #[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
                struct AllocateParams {
                    width: u32,
                    height: u32,
                    min_spp: u32,
                    max_spp: u32,
                    variance_threshold: f32,
                    _pad: [f32; 3],
                    _pad2: [f32; 4],
                }
                let variance_params = VarianceParams { width: full_w, height: full_h, _pad: [0; 2] };
                queue.write_buffer(&ad_bgs.variance_params_buf, 0, bytemuck::bytes_of(&variance_params));
                let allocate_params = AllocateParams {
                    width: full_w,
                    height: full_h,
                    min_spp: self.adaptive_config.min_spp,
                    max_spp: self.adaptive_config.max_spp,
                    variance_threshold: self.adaptive_config.variance_threshold,
                    _pad: [0.0; 3],
                    _pad2: [0.0; 4],
                };
                queue.write_buffer(&ad_bgs.allocate_params_buf, 0, bytemuck::bytes_of(&allocate_params));

                if self.frame_count.is_multiple_of(self.adaptive_config.update_interval) {
                    let (variance_pl, allocate_pl) = ad.pipelines();
                    {
                        let mut pass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor {
                            label: Some("adaptive_variance_pass"),
                            timestamp_writes: None,
                        });
                        pass.set_pipeline(variance_pl);
                        pass.set_bind_group(0, &ad_bgs.variance_bg, &[]);
                        let wg_x = full_w.div_ceil(8);
                        let wg_y = full_h.div_ceil(8);
                        pass.dispatch_workgroups(wg_x, wg_y, 1);
                    }
                    {
                        let mut pass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor {
                            label: Some("adaptive_allocate_pass"),
                            timestamp_writes: None,
                        });
                        pass.set_pipeline(allocate_pl);
                        pass.set_bind_group(0, &ad_bgs.allocate_bg, &[]);
                        let wg_x = full_w.div_ceil(8);
                        let wg_y = full_h.div_ceil(8);
                        pass.dispatch_workgroups(wg_x, wg_y, 1);
                    }
                    log::trace!("Adaptive: variance+allocate");
                }
            }
        }

        let wf_ms = wf_start.elapsed().as_secs_f64() * 1000.0;
        log::trace!("WF dispatch: done ({:.2}ms)", wf_ms);

        true
    }

    fn create_accum_buffer(device: &wgpu::Device, width: u32, height: u32) -> wgpu::Buffer {
        let size = (width * height) as u64 * 16;
        log::debug!("PT accum buffer: {}x{} -> {} bytes", width, height, size);
        device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("pt_accum"),
            size: size.max(16),
            usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        })
    }

    fn create_variance_buffer(device: &wgpu::Device, width: u32, height: u32) -> wgpu::Buffer {
        let size = (width * height) as u64 * 16; // M2 (vec4) for Welford's algorithm
        log::debug!("PT variance buffer: {}x{} -> {} bytes", width, height, size);
        device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("pt_variance"),
            size: size.max(16),
            usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        })
    }

    fn create_guide_buffer(device: &wgpu::Device, width: u32, height: u32) -> wgpu::Buffer {
        let size = (width * height).max(1) as u64 * 24; // packed guide: 6x u32 per pixel
        log::debug!("PT guide buffer: {}x{} -> {} bytes", width, height, size);
        device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("pt_guide_buffer"),
            size: size.max(16),
            usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        })
    }

    fn clear_guide_buffer(encoder: &mut wgpu::CommandEncoder, guide: &wgpu::Buffer) {
        encoder.clear_buffer(guide, 0, None);
    }

    fn create_output(device: &wgpu::Device, width: u32, height: u32) -> (wgpu::Texture, wgpu::TextureView) {
        let tex = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("pt_output"),
            size: wgpu::Extent3d { width, height, depth_or_array_layers: 1 },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::Rgba32Float,
            usage: wgpu::TextureUsages::STORAGE_BINDING | wgpu::TextureUsages::TEXTURE_BINDING,
            view_formats: &[],
        });
        let view = tex.create_view(&wgpu::TextureViewDescriptor::default());
        (tex, view)
    }

    fn update_scene_bounds(&mut self, instances: &[Instance]) {
        if instances.is_empty() {
            self.scene_bounds = None;
            return;
        }
        let mut min = [f32::INFINITY; 3];
        let mut max = [f32::NEG_INFINITY; 3];
        for inst in instances {
            for i in 0..3 {
                min[i] = min[i].min(inst.aabb.min[i]);
                max[i] = max[i].max(inst.aabb.max[i]);
            }
        }
        self.scene_bounds = Some((min, max));
    }

    fn create_default_env_texture(
        device: &wgpu::Device,
        queue: &wgpu::Queue,
    ) -> (wgpu::Texture, wgpu::TextureView) {
        let bytes: [u8; 8] = [0, 0, 0, 0, 0, 0, 0, 0]; // 1x1 black Rgba16Float
        let tex = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("pt_default_env"),
            size: wgpu::Extent3d { width: 1, height: 1, depth_or_array_layers: 1 },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::Rgba16Float,
            usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
            view_formats: &[],
        });
        queue.write_texture(
            wgpu::TexelCopyTextureInfo {
                texture: &tex,
                mip_level: 0,
                origin: wgpu::Origin3d::ZERO,
                aspect: wgpu::TextureAspect::All,
            },
            &bytes,
            wgpu::TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(8),
                rows_per_image: Some(1),
            },
            wgpu::Extent3d { width: 1, height: 1, depth_or_array_layers: 1 },
        );
        let view = tex.create_view(&wgpu::TextureViewDescriptor::default());
        (tex, view)
    }

    /// Resize output texture if dimensions changed.
    pub fn resize(&mut self, device: &wgpu::Device, width: u32, height: u32) {
        if self.width == width && self.height == height {
            return;
        }
        log::debug!("PT resize: {}x{} -> {}x{}", self.width, self.height, width, height);
        self.width = width;
        self.height = height;
        let (tex, view) = Self::create_output(device, width, height);
        self.output_texture = tex;
        self.output_view = view;
        self.accum_buffer = Self::create_accum_buffer(device, width, height);
        self.variance_buffer = Self::create_variance_buffer(device, width, height);
        self.guide_buffer = Self::create_guide_buffer(device, width, height);
        self.frame_count = 0;
        if let Some(wf) = &mut self.wavefront {
            wf.resize(device, width, height);
        }
        if let Some(rs) = &mut self.restir {
            rs.resize(device, width, height);
        }
        if let Some(ad) = &mut self.adaptive {
            ad.resize(device, width, height);
        }
        let fallback_samples = vec![u32::MAX; (width * height).max(1) as usize];
        self.sample_map_fallback = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("pt_sample_map_fallback"),
            contents: bytemuck::cast_slice(&fallback_samples),
            usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_DST,
        });
        self.rebuild_bind_group(device);
        self.rebuild_adaptive_bind_groups(device);
        self.rebuild_wavefront_bind_groups(device);
        self.rebuild_restir_bind_groups(device);
        self.rebuild_pathguide_bind_groups(device);
    }

    /// Upload instance scene data to GPU.
    pub fn upload_scene(
        &mut self,
        device: &wgpu::Device,
        data: &GpuInstanceSceneData,
        instances: Option<&[Instance]>,
    ) {
        let nodes_bytes = data.nodes_bytes();
        let inst_bytes = data.instances_bytes();
        let mat_bytes = data.materials_bytes();

        let nodes_buffer = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("pt_nodes"),
            contents: if nodes_bytes.is_empty() { &[0u8; 32] } else { nodes_bytes },
            usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_DST,
        });

        let instances_buffer = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("pt_instances"),
            contents: if inst_bytes.is_empty() { &[0u8; 96] } else { inst_bytes },
            usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_DST,
        });

        let materials_buffer = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("pt_materials"),
            contents: if mat_bytes.is_empty() { &[0u8; 144] } else { mat_bytes },
            usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_DST,
        });

        self.nodes_buffer = Some(nodes_buffer);
        self.instances_buffer = Some(instances_buffer);
        self.materials_buffer = Some(materials_buffer);
        self.scene_ready = true;
        self.frame_count = 0;
        if let Some(instances) = instances {
            self.update_scene_bounds(instances);
        }
        self.rebuild_bind_group(device);
        // Also rebuild wavefront bind groups if wavefront is enabled
        if self.wavefront.is_some() {
            self.rebuild_wavefront_bind_groups(device);
        }
        self.rebuild_restir_bind_groups(device);
        self.rebuild_pathguide_bind_groups(device);
    }

    /// Update BVH configuration from UI options.
    pub fn set_bvh_config(&mut self, gpu_enabled: bool, refit_enabled: bool) {
        self.bvh_config.enabled = gpu_enabled;
        // refit is handled automatically based on can_refit()
        let _ = refit_enabled; // reserved for future use
    }

    /// Upload scene with smart BVH build (GPU or CPU based on config).
    /// For animation: tries refit first if structure unchanged.
    #[allow(dead_code)]
    pub fn upload_scene_smart(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        instances: &[Instance],
        data: &GpuInstanceSceneData,
        is_animating: bool,
    ) {
        self.update_scene_bounds(instances);
        // Check if we can refit (same instance count, valid structure)
        let can_refit = is_animating && self.bvh_builder.can_refit(instances.len());
        if can_refit && self.sorted_indices.is_empty() {
            // No valid ordering; force rebuild
        } else if can_refit {
            // NOTE: output nodes are linearized for traversal, but refit expects LBVH layout.
            // Until we have a linearized-refit path, skip refit and rebuild to avoid corrupt AABBs.
            log::warn!("PT refit skipped: linearized node layout incompatible with LBVH refit, rebuilding");
        }

        // Full rebuild path
        let (nodes, sorted_indices) = self.bvh_builder.build(device, queue, instances, &self.bvh_config);

        let gpu_data = pt_core::gpu_data::build_gpu_data_from_nodes(
            nodes,
            &sorted_indices,
            instances,
            &data.materials,
        );

        // Store sorted_indices after use (avoids clone)
        self.sorted_indices = sorted_indices;

        // Upload nodes
        let nodes_bytes = gpu_data.nodes_bytes();
        let nodes_buffer = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("pt_nodes"),
            contents: if nodes_bytes.is_empty() { &[0u8; 32] } else { nodes_bytes },
            usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_DST,
        });

        // Upload instances (sorted by BVH order)
        let inst_bytes = gpu_data.instances_bytes();
        let instances_buffer = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("pt_instances"),
            contents: if inst_bytes.is_empty() { &[0u8; 96] } else { inst_bytes },
            usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_DST,
        });

        // Upload materials
        let mat_bytes = gpu_data.materials_bytes();
        let materials_buffer = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("pt_materials"),
            contents: if mat_bytes.is_empty() { &[0u8; 144] } else { mat_bytes },
            usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_DST,
        });

        self.nodes_buffer = Some(nodes_buffer);
        self.instances_buffer = Some(instances_buffer);
        self.materials_buffer = Some(materials_buffer);
        self.scene_ready = true;
        self.frame_count = 0;
        self.rebuild_bind_group(device);
        if self.wavefront.is_some() {
            self.rebuild_wavefront_bind_groups(device);
        }
        self.rebuild_restir_bind_groups(device);
        self.rebuild_pathguide_bind_groups(device);
    }

    /// Invalidate BVH structure (forces full rebuild on next upload).
    #[allow(dead_code)]
    pub fn invalidate_bvh(&mut self) {
        self.bvh_builder.invalidate();
    }

    /// Build BVH using configured method (GPU or CPU).
    /// Returns (nodes, sorted_indices) for use with build_gpu_data_from_nodes.
    pub fn build_bvh(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        instances: &[Instance],
    ) -> (Vec<pt_core::bvh::BvhNode>, Vec<u32>) {
        self.bvh_builder.build(device, queue, instances, &self.bvh_config)
    }

    /// Check if BVH can be refitted instead of rebuilt (for animation).
    #[allow(dead_code)]
    pub fn can_refit_bvh(&self, instance_count: usize) -> bool {
        self.bvh_builder.can_refit(instance_count)
    }

    /// Rebuild bind groups after buffer/texture change.
    fn rebuild_bind_group(&mut self, device: &wgpu::Device) {
        let (Some(nodes), Some(instances), Some(materials)) =
            (&self.nodes_buffer, &self.instances_buffer, &self.materials_buffer) else {
            self.bind_group = None;
            return;
        };

        let sample_map = if let Some(ad) = &self.adaptive {
            ad.sample_map()
        } else {
            &self.sample_map_fallback
        };
        self.bind_group = Some(device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("pt_bind_group"),
            layout: &self.bind_group_layout,
            entries: &[
                wgpu::BindGroupEntry { binding: 0, resource: nodes.as_entire_binding() },
                wgpu::BindGroupEntry { binding: 1, resource: instances.as_entire_binding() },
                wgpu::BindGroupEntry { binding: 2, resource: self.camera_buffer.as_entire_binding() },
                wgpu::BindGroupEntry { binding: 3, resource: wgpu::BindingResource::TextureView(&self.output_view) },
                wgpu::BindGroupEntry { binding: 4, resource: self.accum_buffer.as_entire_binding() },
                wgpu::BindGroupEntry { binding: 5, resource: materials.as_entire_binding() },
                wgpu::BindGroupEntry { binding: 6, resource: wgpu::BindingResource::TextureView(&self.env_view) },
                wgpu::BindGroupEntry { binding: 7, resource: wgpu::BindingResource::Sampler(&self.env_sampler) },
                wgpu::BindGroupEntry { binding: 8, resource: self.env_uniform_buffer.as_entire_binding() },
                wgpu::BindGroupEntry { binding: 9, resource: self.env_marginal_cdf.as_entire_binding() },
                wgpu::BindGroupEntry { binding: 10, resource: self.env_conditional_cdf.as_entire_binding() },
                wgpu::BindGroupEntry { binding: 11, resource: sample_map.as_entire_binding() },
            ],
        }));

        self.pick_bind_group = Some(device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("pt_pick_bg"),
            layout: &self.pick_bind_group_layout,
            entries: &[
                wgpu::BindGroupEntry { binding: 0, resource: nodes.as_entire_binding() },
                wgpu::BindGroupEntry { binding: 1, resource: instances.as_entire_binding() },
                wgpu::BindGroupEntry { binding: 2, resource: self.pick_params_buffer.as_entire_binding() },
                wgpu::BindGroupEntry { binding: 3, resource: self.pick_result_buffer.as_entire_binding() },
            ],
        }));

        // Rebuild blit bind group
        self.blit_bind_group = Some(device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("pt_blit_bg"),
            layout: &self.blit_bind_group_layout,
            entries: &[
                wgpu::BindGroupEntry { binding: 0, resource: wgpu::BindingResource::TextureView(&self.output_view) },
                wgpu::BindGroupEntry { binding: 1, resource: wgpu::BindingResource::Sampler(&self.blit_sampler) },
            ],
        }));

        // Rebuild wavefront bind groups if enabled
        self.rebuild_wavefront_bind_groups(device);
        // Rebuild adaptive bind groups if enabled
        self.rebuild_adaptive_bind_groups(device);
    }

    /// Update camera uniform.
    pub fn update_camera(&mut self, queue: &wgpu::Queue, uniform: &PtCameraUniform) {
        queue.write_buffer(&self.camera_buffer, 0, bytemuck::bytes_of(uniform));
    }

    /// Update ReSTIR gbuffer camera matrices (prev/curr view-proj).
    pub fn update_view_proj(&mut self, queue: &wgpu::Queue, prev: [[f32; 4]; 4], curr: [[f32; 4]; 4]) {
        if let Some(restir_bgs) = &self.restir_bind_groups {
            let params = GBufferParams {
                width: self.width,
                height: self.height,
                _pad0: [0; 2],
                prev_view_proj: prev,
                curr_view_proj: curr,
            };
            queue.write_buffer(&restir_bgs.gbuffer_params_buf, 0, bytemuck::bytes_of(&params));
        }
    }

    /// Set environment from renderer's texture. Rebuilds bind group.
    pub fn set_environment_texture(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        texture: &wgpu::Texture,
        intensity: f32,
        enabled: bool,
    ) {
        self.env_view = texture.create_view(&wgpu::TextureViewDescriptor::default());
        self.env_intensity = intensity;
        self.env_enabled = if enabled { 1.0 } else { 0.0 };
        let uniform = self.build_env_uniform(1.0, 0.0);
        queue.write_buffer(&self.env_uniform_buffer, 0, bytemuck::bytes_of(&uniform));
        self.rebuild_bind_group(device);
        self.reset_accumulation();
    }

    /// Update environment params (e.g. global opacity, time) without reloading texture.
    #[allow(clippy::too_many_arguments)]
    pub fn update_env_params(
        &mut self,
        queue: &wgpu::Queue,
        intensity: f32,
        rotation: f32,
        enabled: bool,
        use_importance_sampling: bool,
        env_width: u32,
        env_height: u32,
        global_opacity: f32,
        time: f32,
    ) {
        self.env_intensity = intensity;
        self.env_rotation = rotation;
        self.env_enabled = if enabled { 1.0 } else { 0.0 };
        self.env_use_importance_sampling = if use_importance_sampling { 1.0 } else { 0.0 };
        self.env_width = env_width.max(1);
        self.env_height = env_height.max(1);
        let uniform = self.build_env_uniform(global_opacity, time);
        queue.write_buffer(&self.env_uniform_buffer, 0, bytemuck::bytes_of(&uniform));
    }

    /// Upload environment CDF buffers for importance sampling.
    pub fn set_environment_cdfs(
        &mut self,
        device: &wgpu::Device,
        marginal_cdf: &[f32],
        conditional_cdf: &[f32],
        width: u32,
        height: u32,
    ) {
        let marginal = if marginal_cdf.is_empty() { vec![1.0f32] } else { marginal_cdf.to_vec() };
        let conditional = if conditional_cdf.is_empty() { vec![1.0f32] } else { conditional_cdf.to_vec() };

        self.env_marginal_cdf = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("pt_env_marginal_cdf"),
            contents: bytemuck::cast_slice(&marginal),
            usage: wgpu::BufferUsages::STORAGE,
        });
        self.env_conditional_cdf = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("pt_env_conditional_cdf"),
            contents: bytemuck::cast_slice(&conditional),
            usage: wgpu::BufferUsages::STORAGE,
        });
        self.env_use_importance_sampling = if width > 1 && height > 1 { 1.0 } else { 0.0 };
        self.env_width = width.max(1);
        self.env_height = height.max(1);
        self.rebuild_bind_group(device);
        self.reset_accumulation();
    }

    fn build_env_uniform(&self, global_opacity: f32, time: f32) -> PtEnvUniform {
        PtEnvUniform {
            params0: [
                self.env_intensity,
                self.env_rotation,
                self.env_enabled,
                self.env_use_importance_sampling,
            ],
            params1: [
                self.env_width as f32,
                self.env_height as f32,
                global_opacity,
                time,
            ],
        }
    }

    /// Reset progressive accumulation.
    pub fn reset_accumulation(&mut self) {
        self.frame_count = 0;
        log::trace!("PT: reset accumulation");
    }

    /// Mark ReSTIR/path guide history as dirty (cleared on next dispatch).
    pub fn mark_history_dirty(&mut self) {
        self.history_dirty = true;
        log::trace!("PT: mark history dirty");
    }

    /// GPU ray pick against the BVH. Returns (object_id, t) on hit.
    pub fn gpu_pick(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        origin: [f32; 3],
        dir: [f32; 3],
    ) -> Option<(u32, f32)> {
        if !self.scene_ready { return None; }
        let Some(bg) = &self.pick_bind_group else { return None; };

        let pick_start = std::time::Instant::now();
        log::trace!("PT pick: start origin={:?} dir={:?}", origin, dir);
        let params = PickParams {
            origin,
            _pad0: 0.0,
            dir,
            _pad1: 0.0,
        };
        queue.write_buffer(&self.pick_params_buffer, 0, bytemuck::bytes_of(&params));

        let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
            label: Some("pt_pick_encoder"),
        });
        {
            let mut pass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor {
                label: Some("pt_pick_pass"),
                timestamp_writes: None,
            });
            pass.set_pipeline(&self.pick_pipeline);
            pass.set_bind_group(0, bg, &[]);
            pass.dispatch_workgroups(1, 1, 1);
        }
        encoder.copy_buffer_to_buffer(
            &self.pick_result_buffer,
            0,
            &self.pick_readback_buffer,
            0,
            std::mem::size_of::<PickResult>() as u64,
        );
        queue.submit(std::iter::once(encoder.finish()));

        let slice = self.pick_readback_buffer.slice(..);
        let (tx, rx) = std::sync::mpsc::channel();
        slice.map_async(wgpu::MapMode::Read, move |r| { let _ = tx.send(r); });
        log::trace!("PT pick: waiting for map");
        // Must wait for map_async callback before rx.recv()
        let _ = device.poll(wgpu::PollType::wait_indefinitely());
        rx.recv().ok().and_then(|r| r.ok())?;

        let data = slice.get_mapped_range();
        let result = bytemuck::from_bytes::<PickResult>(&data);
        let hit = if result.hit == 1 { Some((result.object_id, result.t)) } else { None };
        drop(data);
        self.pick_readback_buffer.unmap();
        let elapsed_ms = pick_start.elapsed().as_secs_f64() * 1000.0;
        log::trace!("PT pick: done hit={:?} ({:.2}ms)", hit, elapsed_ms);
        hit
    }

    /// Dispatch compute shader. Returns false if not ready.
    pub fn dispatch(&mut self, encoder: &mut wgpu::CommandEncoder, _queue: &wgpu::Queue) -> bool {
        let Some(bg) = &self.bind_group else {
            log::warn!("PT dispatch: bind_group is None!");
            return false;
        };
        if !self.scene_ready {
            log::warn!("PT dispatch: scene_ready=false!");
            return false;
        }
        if self.frame_count >= self.max_samples {
            log::trace!("PT dispatch: max reached {}/{}", self.frame_count, self.max_samples);
            return true;
        }

        let dispatch_start = std::time::Instant::now();
        
        // Clear accum buffer on first frame (after reset)
        if self.frame_count == 0 {
            encoder.clear_buffer(&self.accum_buffer, 0, None);
        }
        
        self.frame_count += 1;
        log::debug!("PT dispatch: frame {}/{}, {}x{}", self.frame_count, self.max_samples, self.width, self.height);

        let wg_x = self.width.div_ceil(WG_SIZE);
        let wg_y = self.height.div_ceil(WG_SIZE);

        let mut pass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor {
            label: Some("pt_compute_pass"),
            timestamp_writes: None,
        });
        pass.set_pipeline(&self.pipeline);
        pass.set_bind_group(0, bg, &[]);
        pass.dispatch_workgroups(wg_x, wg_y, 1);

        let elapsed_ms = dispatch_start.elapsed().as_secs_f64() * 1000.0;
        log::trace!("PT dispatch: done ({:.2}ms)", elapsed_ms);
        true
    }

    /// Blit the path tracer output to a render target with tone mapping.
    pub fn blit(&self, encoder: &mut wgpu::CommandEncoder, target: &wgpu::TextureView) {
        let Some(bg) = &self.blit_bind_group else { return; };

        let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("pt_blit_pass"),
            color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                view: target,
                resolve_target: None,
                depth_slice: None,
                ops: wgpu::Operations {
                    load: wgpu::LoadOp::Clear(wgpu::Color::BLACK),
                    store: wgpu::StoreOp::Store,
                },
            })],
            depth_stencil_attachment: None,
            timestamp_writes: None,
            occlusion_query_set: None,
        });
        pass.set_pipeline(&self.blit_pipeline);
        pass.set_bind_group(0, bg, &[]);
        pass.draw(0..3, 0..1);
    }
}
