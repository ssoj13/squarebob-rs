//! Wavefront pipeline orchestration.
#![allow(dead_code)]

use super::buffers::{WfHit, WfRay};
use bytemuck::{Pod, Zeroable};
use log::debug;
use wgpu::util::DeviceExt;

/// Dims uniform for raygen pass.
#[repr(C)]
#[derive(Debug, Clone, Copy, Pod, Zeroable)]
pub struct WfDims {
    pub full_width: u32,
    pub full_height: u32,
    pub tile_width: u32,
    pub tile_height: u32,
    pub tile_x: u32,
    pub tile_y: u32,
    pub _pad: [u32; 2],
}

const RAYGEN_WGSL: &str = include_str!("raygen.wgsl");
const INTERSECT_WGSL: &str = include_str!("intersect.wgsl");
const SHADE_WGSL: &str = include_str!("shade.wgsl");
const FINALIZE_WGSL: &str = include_str!("finalize.wgsl");
const COUNT_SWAP_WGSL: &str = include_str!("count_swap.wgsl");

/// Wavefront path tracer pipeline.
pub struct WavefrontPipeline {
    // Pipelines
    raygen_pipeline: wgpu::ComputePipeline,
    intersect_pipeline: wgpu::ComputePipeline,
    shade_pipeline: wgpu::ComputePipeline,
    finalize_pipeline: wgpu::ComputePipeline,
    count_swap_pipeline: wgpu::ComputePipeline,

    // Bind group layouts
    raygen_bgl: wgpu::BindGroupLayout,
    intersect_bgl: wgpu::BindGroupLayout,
    shade_bgl: wgpu::BindGroupLayout,
    finalize_bgl: wgpu::BindGroupLayout,
    count_swap_bgl: wgpu::BindGroupLayout,

    // Double-buffered ray buffers (ping-pong)
    ray_buf_a: Option<wgpu::Buffer>,
    ray_buf_b: Option<wgpu::Buffer>,
    hit_buf: Option<wgpu::Buffer>,

    // Count buffer (count_in at [0], count_out at [1])
    count_buf: wgpu::Buffer,

    // Dims uniform
    dims_buf: wgpu::Buffer,

    // Dimensions and state
    width: u32,
    height: u32,
    cur_buf: u32,
}

impl WavefrontPipeline {
    pub fn new(device: &wgpu::Device, width: u32, height: u32) -> Self {
        debug!("WavefrontPipeline::new {}x{}", width, height);
        let (raygen_pipeline, raygen_bgl) = create_pipeline(
            device,
            RAYGEN_WGSL,
            "raygen",
            &[
                bgl_uniform(0),    // camera
                bgl_uniform(1),    // dims
                bgl_storage_rw(2), // ray output
                bgl_storage_rw(3), // count
                bgl_storage_ro(4), // sample map
                bgl_storage_ro(5), // accum buffer (for sample count check)
            ],
        );

        let (intersect_pipeline, intersect_bgl) = create_pipeline(
            device,
            INTERSECT_WGSL,
            "intersect",
            &[
                bgl_storage_ro(0), // nodes
                bgl_storage_ro(1), // instances
                bgl_storage_ro(2), // rays
                bgl_storage_rw(3), // hits
                bgl_storage_rw(4), // count
            ],
        );

        let (shade_pipeline, shade_bgl) = create_pipeline(
            device,
            SHADE_WGSL,
            "shade",
            &[
                bgl_storage_ro(0),  // instances
                bgl_storage_ro(1),  // materials
                bgl_storage_ro(2),  // rays in
                bgl_storage_ro(3),  // hits
                bgl_storage_rw(4),  // rays out
                bgl_storage_rw(5),  // accum
                bgl_storage_rw(6),  // counts (in/out)
                bgl_uniform(7),     // params
                bgl_texture_2d(8),  // env_map
                bgl_sampler(9),     // env_sampler
                bgl_uniform(10),    // env_params
                bgl_storage_rw(11), // guide buffer
            ],
        );

        let (finalize_pipeline, finalize_bgl) = create_finalize_pipeline(device);
        let (count_swap_pipeline, count_swap_bgl) = create_pipeline(
            device,
            COUNT_SWAP_WGSL,
            "count_swap",
            &[
                bgl_storage_rw(0), // counts (in/out)
            ],
        );

        let count_buf = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("wf_counts"),
            size: 16, // count_in, count_out, padding
            usage: wgpu::BufferUsages::STORAGE
                | wgpu::BufferUsages::COPY_SRC
                | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });

        let dims = WfDims {
            full_width: width,
            full_height: height,
            tile_width: width,
            tile_height: height,
            tile_x: 0,
            tile_y: 0,
            _pad: [0, 0],
        };
        let dims_buf = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("wf_dims"),
            contents: bytemuck::bytes_of(&dims),
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
        });

        let mut p = Self {
            raygen_pipeline,
            intersect_pipeline,
            shade_pipeline,
            finalize_pipeline,
            count_swap_pipeline,
            raygen_bgl,
            intersect_bgl,
            shade_bgl,
            finalize_bgl,
            count_swap_bgl,
            ray_buf_a: None,
            ray_buf_b: None,
            hit_buf: None,
            count_buf,
            dims_buf,
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
        let (clamped_w, clamped_h) = Self::clamp_dimensions(device, width, height);
        if clamped_w != width || clamped_h != height {
            debug!(
                "WavefrontPipeline::resize clamp {}x{} -> {}x{} (binding limit)",
                width, height, clamped_w, clamped_h
            );
        } else {
            debug!("WavefrontPipeline::resize {}x{}", width, height);
        }
        self.width = clamped_w;
        self.height = clamped_h;

        let n = (self.width * self.height) as u64;
        let ray_sz = std::mem::size_of::<WfRay>() as u64;
        let hit_sz = std::mem::size_of::<WfHit>() as u64;

        self.ray_buf_a = Some(create_buf(device, "wf_ray_a", n * ray_sz));
        self.ray_buf_b = Some(create_buf(device, "wf_ray_b", n * ray_sz));
        self.hit_buf = Some(create_buf(device, "wf_hit", n * hit_sz));
        self.cur_buf = 0;

        // Update dims uniform
        let dims = WfDims {
            full_width: self.width,
            full_height: self.height,
            tile_width: self.width,
            tile_height: self.height,
            tile_x: 0,
            tile_y: 0,
            _pad: [0, 0],
        };
        self.dims_buf = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("wf_dims"),
            contents: bytemuck::bytes_of(&dims),
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
        });
    }

    fn clamp_dimensions(device: &wgpu::Device, width: u32, height: u32) -> (u32, u32) {
        let n = (width * height).max(1) as u64;
        let ray_sz = std::mem::size_of::<WfRay>() as u64;
        let hit_sz = std::mem::size_of::<WfHit>() as u64;
        let per_pixel = ray_sz.max(hit_sz).max(1);
        let limit = device.limits().max_storage_buffer_binding_size;
        let max_pixels = (limit / per_pixel).max(1);
        if n <= max_pixels {
            return (width, height);
        }
        let scale = (max_pixels as f64 / n as f64).sqrt();
        let new_w = (width as f64 * scale).floor().max(1.0) as u32;
        let new_h = (height as f64 * scale).floor().max(1.0) as u32;
        log::warn!(
            "WavefrontPipeline: dimensions clamped {}x{} -> {}x{} (limit={}, per_pixel={}, max_pixels={})",
            width, height, new_w, new_h, limit, per_pixel, max_pixels
        );
        (new_w.max(1), new_h.max(1))
    }

    /// Get current/next ray buffers (ping-pong).
    pub fn ray_bufs(&self) -> (&wgpu::Buffer, &wgpu::Buffer) {
        let a = self.ray_buf_a.as_ref().unwrap();
        let b = self.ray_buf_b.as_ref().unwrap();
        if self.cur_buf == 0 {
            (a, b)
        } else {
            (b, a)
        }
    }

    /// Get raw ray buffers (a, b) without ping-pong logic.
    pub fn ray_bufs_raw(&self) -> (&wgpu::Buffer, &wgpu::Buffer) {
        (
            self.ray_buf_a.as_ref().unwrap(),
            self.ray_buf_b.as_ref().unwrap(),
        )
    }

    /// Swap buffers after shade pass.
    pub fn swap_bufs(&mut self) {
        self.cur_buf = 1 - self.cur_buf;
    }

    /// Get buffers and pipelines.
    pub fn hit_buf(&self) -> &wgpu::Buffer {
        self.hit_buf.as_ref().unwrap()
    }

    pub fn pipelines(
        &self,
    ) -> (
        &wgpu::ComputePipeline,
        &wgpu::ComputePipeline,
        &wgpu::ComputePipeline,
    ) {
        (
            &self.raygen_pipeline,
            &self.intersect_pipeline,
            &self.shade_pipeline,
        )
    }
    pub fn bgls(
        &self,
    ) -> (
        &wgpu::BindGroupLayout,
        &wgpu::BindGroupLayout,
        &wgpu::BindGroupLayout,
    ) {
        (&self.raygen_bgl, &self.intersect_bgl, &self.shade_bgl)
    }

    pub fn finalize_pipeline(&self) -> &wgpu::ComputePipeline {
        &self.finalize_pipeline
    }
    pub fn finalize_bgl(&self) -> &wgpu::BindGroupLayout {
        &self.finalize_bgl
    }
    pub fn count_swap_pipeline(&self) -> &wgpu::ComputePipeline {
        &self.count_swap_pipeline
    }
    pub fn count_swap_bgl(&self) -> &wgpu::BindGroupLayout {
        &self.count_swap_bgl
    }

    pub fn dims_buf(&self) -> &wgpu::Buffer {
        &self.dims_buf
    }
    pub fn dimensions(&self) -> (u32, u32) {
        (self.width, self.height)
    }
    pub fn count_buf(&self) -> &wgpu::Buffer {
        &self.count_buf
    }
    pub fn write_dims(&self, queue: &wgpu::Queue, dims: &WfDims) {
        queue.write_buffer(&self.dims_buf, 0, bytemuck::bytes_of(dims));
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
        label: Some(&format!("wf_{name}_shader")),
        source: wgpu::ShaderSource::Wgsl(wgsl.into()),
    });
    let bgl = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
        label: Some(&format!("wf_{name}_bgl")),
        entries,
    });
    let pl = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
        label: Some(&format!("wf_{name}_pl")),
        bind_group_layouts: &[Some(&bgl)],
        immediate_size: 0,
    });
    let pipeline = device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
        label: Some(&format!("wf_{name}_pipeline")),
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

fn bgl_sampler(binding: u32) -> wgpu::BindGroupLayoutEntry {
    wgpu::BindGroupLayoutEntry {
        binding,
        visibility: wgpu::ShaderStages::COMPUTE,
        ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
        count: None,
    }
}

// Finalize pipeline: copy accum buffer to output texture
fn create_finalize_pipeline(
    device: &wgpu::Device,
) -> (wgpu::ComputePipeline, wgpu::BindGroupLayout) {
    let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
        label: Some("wf_finalize_shader"),
        source: wgpu::ShaderSource::Wgsl(FINALIZE_WGSL.into()),
    });

    let bgl = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
        label: Some("wf_finalize_bgl"),
        entries: &[
            // @binding(0) accum buffer (read)
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
            // @binding(1) output texture (write)
            wgpu::BindGroupLayoutEntry {
                binding: 1,
                visibility: wgpu::ShaderStages::COMPUTE,
                ty: wgpu::BindingType::StorageTexture {
                    access: wgpu::StorageTextureAccess::WriteOnly,
                    format: wgpu::TextureFormat::Rgba32Float,
                    view_dimension: wgpu::TextureViewDimension::D2,
                },
                count: None,
            },
            // @binding(2) params uniform
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
        ],
    });

    let pl = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
        label: Some("wf_finalize_pl"),
        bind_group_layouts: &[Some(&bgl)],
        immediate_size: 0,
    });

    let pipeline = device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
        label: Some("wf_finalize_pipeline"),
        layout: Some(&pl),
        module: &shader,
        entry_point: Some("main"),
        compilation_options: Default::default(),
        cache: None,
    });

    (pipeline, bgl)
}
