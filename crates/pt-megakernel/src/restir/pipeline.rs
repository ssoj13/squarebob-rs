//! ReSTIR pipeline orchestration.

use super::reservoir::{MotionVector, Reservoir};
use std::num::NonZeroU64;

const INITIAL_WGSL: &str = include_str!("initial.wgsl");
const TEMPORAL_WGSL: &str = include_str!("temporal.wgsl");
const SPATIAL_WGSL: &str = include_str!("spatial.wgsl");
const SHADE_WGSL: &str = include_str!("shade.wgsl");

/// WGSL `Params` struct sizes. Keep in sync with the matching Rust structs
/// in `crate::compute` and the WGSL declarations. All are 16-byte aligned
/// (uniform buffer rules) and per-tile slots are 256-byte strided in the
/// dynamic-offset buffer.
pub const RESTIR_INITIAL_PARAMS_SIZE: u64 = 32;
pub const RESTIR_TEMPORAL_PARAMS_SIZE: u64 = 48;
pub const RESTIR_SPATIAL_PARAMS_SIZE: u64 = 48;
pub const RESTIR_SHADE_PARAMS_SIZE: u64 = 48;

/// ReSTIR pipeline state.
pub struct ReSTIRPipeline {
    // Pipelines
    initial_pipeline: wgpu::ComputePipeline,
    temporal_pipeline: wgpu::ComputePipeline,
    spatial_pipeline: wgpu::ComputePipeline,
    shade_pipeline: wgpu::ComputePipeline,

    // Bind group layouts
    initial_bgl: wgpu::BindGroupLayout,
    temporal_bgl: wgpu::BindGroupLayout,
    spatial_bgl: wgpu::BindGroupLayout,
    shade_bgl: wgpu::BindGroupLayout,

    // Double-buffered reservoirs (temporal ping-pong)
    reservoir_a: Option<wgpu::Buffer>,
    reservoir_b: Option<wgpu::Buffer>,

    // Motion vectors for temporal reprojection
    motion_buf: Option<wgpu::Buffer>,

    // G-buffer for visibility checks
    gbuf_depth: Option<wgpu::Buffer>,
    gbuf_normal: Option<wgpu::Buffer>,
    /// Per-pixel hit instance id (0xFFFFFFFF for miss). Lets ReSTIR shaders
    /// look up materials and identify hit geometry without reading the
    /// wavefront's tile-local rays/hits buffers.
    gbuf_instance_id: Option<wgpu::Buffer>,

    // Dimensions
    width: u32,
    height: u32,
    cur_buf: u32,
}

impl ReSTIRPipeline {
    pub fn new(device: &wgpu::Device, width: u32, height: u32) -> Self {
        let (initial_pipeline, initial_bgl) = create_pipeline(
            device,
            INITIAL_WGSL,
            "initial",
            &[
                bgl_storage_ro(0),                              // hits
                bgl_storage_rw(1),                              // reservoirs
                bgl_uniform_dyn(2, RESTIR_INITIAL_PARAMS_SIZE), // params (per-tile)
                bgl_texture_2d(3),                              // env map
                bgl_sampler(4),                                 // env sampler
                bgl_uniform(5),                                 // env params
                bgl_storage_ro(6),                              // env marginal cdf
                bgl_storage_ro(7),                              // env conditional cdf
                bgl_storage_ro(8),                              // rays
                bgl_storage_ro(9),                              // bvh nodes
                bgl_storage_ro(10),                             // instances
                bgl_texture_2d_unfilterable(11),                // emissive light texture
                bgl_uniform(12),                                // emissive light params
            ],
        );

        let (temporal_pipeline, temporal_bgl) = create_pipeline(
            device,
            TEMPORAL_WGSL,
            "temporal",
            &[
                bgl_storage_ro(0),                               // prev reservoirs
                bgl_storage_rw(1),                               // curr reservoirs
                bgl_storage_ro(2),                               // motion vectors
                bgl_storage_ro(3),                               // prev depth
                bgl_storage_ro(4),                               // curr depth
                bgl_uniform_dyn(5, RESTIR_TEMPORAL_PARAMS_SIZE), // params (per-tile)
            ],
        );

        let (spatial_pipeline, spatial_bgl) = create_pipeline(
            device,
            SPATIAL_WGSL,
            "spatial",
            &[
                bgl_storage_ro(0),                              // reservoirs input
                bgl_storage_rw(1),                              // reservoirs output
                bgl_storage_ro(2),                              // depth
                bgl_storage_ro(3),                              // normal
                bgl_uniform_dyn(4, RESTIR_SPATIAL_PARAMS_SIZE), // params (per-tile)
            ],
        );

        let (shade_pipeline, shade_bgl) = create_pipeline(
            device,
            SHADE_WGSL,
            "shade",
            &[
                bgl_storage_ro(0),                            // reservoirs
                bgl_storage_ro(1),                            // hits
                bgl_storage_rw(2),                            // output
                bgl_uniform_dyn(3, RESTIR_SHADE_PARAMS_SIZE), // params (per-tile)
                bgl_storage_ro(4),                            // instances
                bgl_storage_ro(5),                            // materials
                bgl_storage_ro(6),                            // sample_map
                bgl_storage_ro(7),                            // rays
                bgl_texture_2d(8),                            // env map
                bgl_sampler(9),                               // env sampler
                bgl_uniform(10),                              // env params
            ],
        );

        let mut p = Self {
            initial_pipeline,
            temporal_pipeline,
            spatial_pipeline,
            shade_pipeline,
            initial_bgl,
            temporal_bgl,
            spatial_bgl,
            shade_bgl,
            reservoir_a: None,
            reservoir_b: None,
            motion_buf: None,
            gbuf_depth: None,
            gbuf_normal: None,
            gbuf_instance_id: None,
            width: 0,
            height: 0,
            cur_buf: 0,
        };
        p.resize(device, width, height);
        p
    }

    /// Resize buffers for new dimensions.
    pub fn resize(&mut self, device: &wgpu::Device, width: u32, height: u32) {
        if self.width == width && self.height == height {
            return;
        }
        self.width = width;
        self.height = height;

        let n = (width * height) as u64;
        let res_sz = Reservoir::SIZE as u64;
        let mv_sz = std::mem::size_of::<MotionVector>() as u64;

        self.reservoir_a = Some(create_buf(device, "restir_res_a", n * res_sz));
        self.reservoir_b = Some(create_buf(device, "restir_res_b", n * res_sz));
        self.motion_buf = Some(create_buf(device, "restir_motion", n * mv_sz));
        self.gbuf_depth = Some(create_buf(device, "restir_depth", n * 4));
        self.gbuf_normal = Some(create_buf(device, "restir_normal", n * 16));
        self.gbuf_instance_id = Some(create_buf(device, "restir_instance_id", n * 4));
        self.cur_buf = 0;
    }

    /// Get current/previous reservoirs (ping-pong for temporal).
    pub fn reservoirs(&self) -> (&wgpu::Buffer, &wgpu::Buffer) {
        let a = self.reservoir_a.as_ref().unwrap();
        let b = self.reservoir_b.as_ref().unwrap();
        if self.cur_buf == 0 {
            (a, b)
        } else {
            (b, a)
        }
    }

    /// Swap buffers after frame.
    pub fn swap_bufs(&mut self) {
        self.cur_buf = 1 - self.cur_buf;
    }

    /// Get pipelines.
    pub fn pipelines(
        &self,
    ) -> (
        &wgpu::ComputePipeline,
        &wgpu::ComputePipeline,
        &wgpu::ComputePipeline,
        &wgpu::ComputePipeline,
    ) {
        (
            &self.initial_pipeline,
            &self.temporal_pipeline,
            &self.spatial_pipeline,
            &self.shade_pipeline,
        )
    }

    /// Get bind group layouts.
    pub fn bgls(
        &self,
    ) -> (
        &wgpu::BindGroupLayout,
        &wgpu::BindGroupLayout,
        &wgpu::BindGroupLayout,
        &wgpu::BindGroupLayout,
    ) {
        (
            &self.initial_bgl,
            &self.temporal_bgl,
            &self.spatial_bgl,
            &self.shade_bgl,
        )
    }

    pub fn motion_buffer(&self) -> &wgpu::Buffer {
        self.motion_buf.as_ref().unwrap()
    }

    pub fn depth_buffer(&self) -> &wgpu::Buffer {
        self.gbuf_depth.as_ref().unwrap()
    }

    pub fn normal_buffer(&self) -> &wgpu::Buffer {
        self.gbuf_normal.as_ref().unwrap()
    }

    pub fn instance_id_buffer(&self) -> &wgpu::Buffer {
        self.gbuf_instance_id.as_ref().unwrap()
    }
}

// Helper: create compute pipeline
fn create_pipeline(
    device: &wgpu::Device,
    wgsl: &str,
    name: &str,
    entries: &[wgpu::BindGroupLayoutEntry],
) -> (wgpu::ComputePipeline, wgpu::BindGroupLayout) {
    let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
        label: Some(&format!("restir_{name}_shader")),
        source: wgpu::ShaderSource::Wgsl(wgsl.into()),
    });
    let bgl = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
        label: Some(&format!("restir_{name}_bgl")),
        entries,
    });
    let pl = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
        label: Some(&format!("restir_{name}_pl")),
        bind_group_layouts: &[Some(&bgl)],
        immediate_size: 0,
    });
    let pipeline = device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
        label: Some(&format!("restir_{name}_pipeline")),
        layout: Some(&pl),
        module: &shader,
        entry_point: Some("main"),
        compilation_options: Default::default(),
        cache: None,
    });
    (pipeline, bgl)
}

fn create_buf(device: &wgpu::Device, label: &str, size: u64) -> wgpu::Buffer {
    device.create_buffer(&wgpu::BufferDescriptor {
        label: Some(label),
        size: size.max(16),
        usage: wgpu::BufferUsages::STORAGE
            | wgpu::BufferUsages::COPY_DST
            | wgpu::BufferUsages::COPY_SRC,
        mapped_at_creation: false,
    })
}

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

fn bgl_uniform_dyn(binding: u32, size: u64) -> wgpu::BindGroupLayoutEntry {
    wgpu::BindGroupLayoutEntry {
        binding,
        visibility: wgpu::ShaderStages::COMPUTE,
        ty: wgpu::BindingType::Buffer {
            ty: wgpu::BufferBindingType::Uniform,
            has_dynamic_offset: true,
            min_binding_size: NonZeroU64::new(size),
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

fn bgl_texture_2d(binding: u32) -> wgpu::BindGroupLayoutEntry {
    wgpu::BindGroupLayoutEntry {
        binding,
        visibility: wgpu::ShaderStages::COMPUTE,
        ty: wgpu::BindingType::Texture {
            sample_type: wgpu::TextureSampleType::Float { filterable: true },
            view_dimension: wgpu::TextureViewDimension::D2,
            multisampled: false,
        },
        count: None,
    }
}

fn bgl_texture_2d_unfilterable(binding: u32) -> wgpu::BindGroupLayoutEntry {
    wgpu::BindGroupLayoutEntry {
        binding,
        visibility: wgpu::ShaderStages::COMPUTE,
        ty: wgpu::BindingType::Texture {
            sample_type: wgpu::TextureSampleType::Float { filterable: false },
            view_dimension: wgpu::TextureViewDimension::D2,
            multisampled: false,
        },
        count: None,
    }
}

fn bgl_sampler(binding: u32) -> wgpu::BindGroupLayoutEntry {
    wgpu::BindGroupLayoutEntry {
        binding,
        visibility: wgpu::ShaderStages::COMPUTE,
        ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
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
