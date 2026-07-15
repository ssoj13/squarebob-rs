//! Adaptive sampling pipeline.

use bytemuck::{Pod, Zeroable};

pub(crate) const VARIANCE_DATA_WGSL: &str = include_str!("variance_data.wgsl");
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

impl VarianceData {
    pub const SIZE: u64 = std::mem::size_of::<Self>() as u64;
}

const _: () = {
    assert!(VarianceData::SIZE == 32);
    assert!(std::mem::align_of::<VarianceData>() == 4);
    assert!(std::mem::offset_of!(VarianceData, mean) == 0);
    assert!(std::mem::offset_of!(VarianceData, _pad0) == 12);
    assert!(std::mem::offset_of!(VarianceData, m2) == 16);
    assert!(std::mem::offset_of!(VarianceData, count) == 28);
};

/// Adaptive sampling pipeline.
pub struct AdaptivePipeline {
    // Pipelines
    variance_pipeline: wgpu::ComputePipeline,
    allocate_pipeline: wgpu::ComputePipeline,

    // Bind group layouts
    variance_bgl: wgpu::BindGroupLayout,
    allocate_bgl: wgpu::BindGroupLayout,

    // SPP target per pixel. Variance state is owned by PathTraceCompute and
    // shared by the megakernel, wavefront estimator, and allocator.
    sample_map: wgpu::Buffer,

    // Dimensions
    width: u32,
    height: u32,
}

impl AdaptivePipeline {
    pub fn new(
        device: &wgpu::Device,
        width: u32,
        height: u32,
    ) -> Result<Self, render_core::GpuLayoutError> {
        let variance_source = format!("{VARIANCE_DATA_WGSL}\n{VARIANCE_WGSL}");
        let (variance_pipeline, variance_bgl) = create_pipeline(
            device,
            &variance_source,
            "variance",
            &[
                bgl_storage_ro(0, std::mem::size_of::<[f32; 4]>() as u64), // cumulative radiance
                bgl_storage_rw(1, VarianceData::SIZE),                     // variance data
                bgl_uniform(2),                                            // params
            ],
        );

        let allocate_source = format!("{VARIANCE_DATA_WGSL}\n{ALLOCATE_WGSL}");
        let (allocate_pipeline, allocate_bgl) = create_pipeline(
            device,
            &allocate_source,
            "allocate",
            &[
                bgl_storage_ro(0, VarianceData::SIZE), // variance data
                bgl_storage_rw(1, std::mem::size_of::<u32>() as u64), // sample map output
                bgl_uniform(2),                        // params
            ],
        );

        let sample_map = Self::build_sample_map(device, width, height)?;
        Ok(Self {
            variance_pipeline,
            allocate_pipeline,
            variance_bgl,
            allocate_bgl,
            sample_map,
            width,
            height,
        })
    }

    fn build_sample_map(
        device: &wgpu::Device,
        width: u32,
        height: u32,
    ) -> Result<wgpu::Buffer, render_core::GpuLayoutError> {
        let size = render_core::checked_2d_storage_buffer_size(
            device,
            "adaptive sample map",
            width,
            height,
            std::mem::size_of::<u32>() as u64,
        )?;
        Ok(render_core::gpu::make_buffer(
            device,
            "adaptive_spp",
            size,
            wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_DST,
        ))
    }

    pub fn resize(
        &mut self,
        device: &wgpu::Device,
        width: u32,
        height: u32,
    ) -> Result<(), render_core::GpuLayoutError> {
        if self.width == width && self.height == height {
            return Ok(());
        }
        let sample_map = Self::build_sample_map(device, width, height)?;
        self.width = width;
        self.height = height;
        self.sample_map = sample_map;
        Ok(())
    }

    pub fn sample_map(&self) -> &wgpu::Buffer {
        &self.sample_map
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

fn bgl_storage_ro(binding: u32, min_size: u64) -> wgpu::BindGroupLayoutEntry {
    wgpu::BindGroupLayoutEntry {
        binding,
        visibility: wgpu::ShaderStages::COMPUTE,
        ty: wgpu::BindingType::Buffer {
            ty: wgpu::BufferBindingType::Storage { read_only: true },
            has_dynamic_offset: false,
            min_binding_size: std::num::NonZeroU64::new(min_size),
        },
        count: None,
    }
}

fn bgl_storage_rw(binding: u32, min_size: u64) -> wgpu::BindGroupLayoutEntry {
    wgpu::BindGroupLayoutEntry {
        binding,
        visibility: wgpu::ShaderStages::COMPUTE,
        ty: wgpu::BindingType::Buffer {
            ty: wgpu::BufferBindingType::Storage { read_only: false },
            has_dynamic_offset: false,
            min_binding_size: std::num::NonZeroU64::new(min_size),
        },
        count: None,
    }
}
