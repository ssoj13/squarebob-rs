//! Path guiding pipeline.
#![allow(dead_code)]

use super::svo::SvoNode;
use log::debug;

const UPDATE_WGSL: &str = include_str!("update.wgsl");
const SAMPLE_WGSL: &str = include_str!("sample.wgsl");

/// Path guiding pipeline state.
pub struct PathGuidePipeline {
    // Pipelines
    update_pipeline: wgpu::ComputePipeline,
    sample_pipeline: wgpu::ComputePipeline,

    // Bind group layouts
    update_bgl: wgpu::BindGroupLayout,
    sample_bgl: wgpu::BindGroupLayout,

    // SVO buffer
    svo_buf: Option<wgpu::Buffer>,

    // Configuration
    resolution: u32,
    frame_count: u32,
}

impl PathGuidePipeline {
    pub fn new(device: &wgpu::Device, resolution: u32) -> Self {
        debug!("PathGuidePipeline::new res={}", resolution);
        let (update_pipeline, update_bgl) = create_pipeline(
            device,
            UPDATE_WGSL,
            "update",
            &[
                bgl_storage_rw(0), // SVO nodes
                bgl_storage_ro(1), // guide buffer
                bgl_uniform(2),    // params
            ],
        );

        let (sample_pipeline, sample_bgl) = create_pipeline(
            device,
            SAMPLE_WGSL,
            "sample",
            &[
                bgl_storage_ro(0), // SVO nodes
                bgl_storage_rw(1), // guide buffer
                bgl_uniform(2),    // params
            ],
        );

        let mut p = Self {
            update_pipeline,
            sample_pipeline,
            update_bgl,
            sample_bgl,
            svo_buf: None,
            resolution,
            frame_count: 0,
        };
        p.allocate_svo(device);
        p
    }

    fn allocate_svo(&mut self, device: &wgpu::Device) {
        // Allocate enough nodes for full octree.
        // Worst case: sum of 8^i for i=0..depth, where depth = log2(resolution).
        let mut depth = 0u32;
        let mut r = self.resolution.max(1);
        while r > 1 {
            r >>= 1;
            depth += 1;
        }
        let max_nodes = (8u64.pow(depth) - 1) / 7; // Geometric series
        let size = max_nodes * SvoNode::SIZE as u64;

        debug!(
            "PathGuidePipeline::allocate_svo res={} depth={} max_nodes={} size={}MB",
            self.resolution,
            depth,
            max_nodes,
            (size as f64 / (1024.0 * 1024.0))
        );
        self.svo_buf = Some(device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("pathguide_svo"),
            size: size.min(128 * 1024 * 1024), // Cap at 128MB
            usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        }));
    }

    pub fn svo_buffer(&self) -> &wgpu::Buffer {
        self.svo_buf.as_ref().unwrap()
    }

    pub fn pipelines(&self) -> (&wgpu::ComputePipeline, &wgpu::ComputePipeline) {
        (&self.update_pipeline, &self.sample_pipeline)
    }

    pub fn bgls(&self) -> (&wgpu::BindGroupLayout, &wgpu::BindGroupLayout) {
        (&self.update_bgl, &self.sample_bgl)
    }

    pub fn tick(&mut self) {
        self.frame_count += 1;
    }
}

fn create_pipeline(
    device: &wgpu::Device,
    wgsl: &str,
    name: &str,
    entries: &[wgpu::BindGroupLayoutEntry],
) -> (wgpu::ComputePipeline, wgpu::BindGroupLayout) {
    let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
        label: Some(&format!("pathguide_{name}_shader")),
        source: wgpu::ShaderSource::Wgsl(wgsl.into()),
    });
    let bgl = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
        label: Some(&format!("pathguide_{name}_bgl")),
        entries,
    });
    let pl = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
        label: Some(&format!("pathguide_{name}_pl")),
        bind_group_layouts: &[Some(&bgl)],
        immediate_size: 0,
    });
    let pipeline = device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
        label: Some(&format!("pathguide_{name}_pipeline")),
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
