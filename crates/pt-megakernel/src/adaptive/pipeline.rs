//! Adaptive sampling pipeline.

use bytemuck::{Pod, Zeroable};

const VARIANCE_WGSL: &str = include_str!("variance.wgsl");
const ALLOCATE_WGSL: &str = include_str!("allocate.wgsl");

/// Per-pixel variance tracking.
#[repr(C)]
#[derive(Debug, Clone, Copy, Pod, Zeroable)]
pub struct VarianceData {
    /// Running mean (Welford's algorithm)
    pub mean: [f32; 3],
    pub _pad0: u32,
    /// Running M2 for variance
    pub m2: [f32; 3],
    /// Sample count
    pub count: u32,
}

/// Adaptive sampling pipeline.
pub struct AdaptivePipeline {
    // Pipelines
    variance_pipeline: wgpu::ComputePipeline,
    allocate_pipeline: wgpu::ComputePipeline,

    // Bind group layouts
    variance_bgl: wgpu::BindGroupLayout,
    allocate_bgl: wgpu::BindGroupLayout,

    // Buffers
    variance_buf: Option<wgpu::Buffer>,
    sample_map: Option<wgpu::Buffer>, // SPP per pixel

    // Dimensions
    width: u32,
    height: u32,
}

impl AdaptivePipeline {
    pub fn new(device: &wgpu::Device, width: u32, height: u32) -> Self {
        let (variance_pipeline, variance_bgl) = create_pipeline(
            device,
            VARIANCE_WGSL,
            "variance",
            &[
                bgl_storage_ro(0), // current sample
                bgl_storage_rw(1), // variance data
                bgl_uniform(2),    // params
            ],
        );

        let (allocate_pipeline, allocate_bgl) = create_pipeline(
            device,
            ALLOCATE_WGSL,
            "allocate",
            &[
                bgl_storage_ro(0), // variance data
                bgl_storage_rw(1), // sample map output
                bgl_uniform(2),    // params
            ],
        );

        let mut p = Self {
            variance_pipeline,
            allocate_pipeline,
            variance_bgl,
            allocate_bgl,
            variance_buf: None,
            sample_map: None,
            width: 0,
            height: 0,
        };
        p.resize(device, width, height);
        p
    }

    pub fn resize(&mut self, device: &wgpu::Device, width: u32, height: u32) {
        if self.width == width && self.height == height {
            return;
        }
        self.width = width;
        self.height = height;

        let n = (width * height) as u64;
        let var_sz = std::mem::size_of::<VarianceData>() as u64;

        self.variance_buf = Some(create_buf(device, "adaptive_variance", n * var_sz));
        self.sample_map = Some(create_buf(device, "adaptive_spp", n * 4)); // u32 per pixel
    }

    pub fn variance_buffer(&self) -> &wgpu::Buffer {
        self.variance_buf.as_ref().unwrap()
    }

    pub fn sample_map(&self) -> &wgpu::Buffer {
        self.sample_map.as_ref().unwrap()
    }

    pub fn pipelines(&self) -> (&wgpu::ComputePipeline, &wgpu::ComputePipeline) {
        (&self.variance_pipeline, &self.allocate_pipeline)
    }

    pub fn bgls(&self) -> (&wgpu::BindGroupLayout, &wgpu::BindGroupLayout) {
        (&self.variance_bgl, &self.allocate_bgl)
    }
}

fn create_pipeline(
    device: &wgpu::Device,
    wgsl: &str,
    name: &str,
    entries: &[wgpu::BindGroupLayoutEntry],
) -> (wgpu::ComputePipeline, wgpu::BindGroupLayout) {
    let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
        label: Some(&format!("adaptive_{name}_shader")),
        source: wgpu::ShaderSource::Wgsl(wgsl.into()),
    });
    let bgl = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
        label: Some(&format!("adaptive_{name}_bgl")),
        entries,
    });
    let pl = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
        label: Some(&format!("adaptive_{name}_pl")),
        bind_group_layouts: &[Some(&bgl)],
        immediate_size: 0,
    });
    let pipeline = device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
        label: Some(&format!("adaptive_{name}_pipeline")),
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
        usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_DST,
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
