//! GPU-accelerated BVH construction with REFIT support.
//!
//! For animated scenes:
//! - First frame: full LBVH build (O(N log N))
//! - Subsequent frames: BVH refit only (O(N)) - just update AABBs
//!
//! Based on Karras 2012 "Maximizing Parallelism in the Construction of BVHs"

use log::{debug, info, trace, warn};
use std::sync::mpsc;

use bytemuck::{Pod, Zeroable};
use wgpu::util::DeviceExt;

use pt_core::bvh::{BvhNode, GpuAabb, Instance};

/// Morton code with primitive index (for sorting).
#[repr(C)]
#[derive(Debug, Clone, Copy, Pod, Zeroable)]
pub struct MortonPrimitive {
    pub code: u32,
    pub index: u32,
}

/// Scene bounds for Morton normalization.
#[repr(C)]
#[derive(Debug, Clone, Copy, Pod, Zeroable)]
pub struct SceneBounds {
    pub min: [f32; 4],
    pub max: [f32; 4],
}

/// Build params uniform.
#[repr(C)]
#[derive(Debug, Clone, Copy, Pod, Zeroable)]
pub struct BuildParams {
    pub count: u32,
    pub is_refit: u32,
    pub _pad: [u32; 2],
}

/// Radix sort params.
#[repr(C)]
#[derive(Debug, Clone, Copy, Pod, Zeroable)]
pub struct RadixParams {
    pub count: u32,
    pub pass: u32,
    pub _pad: [u32; 2],
}

/// GPU LBVH node (matches lbvh_build.wgsl).
#[repr(C)]
#[derive(Debug, Clone, Copy, Pod, Zeroable)]
struct GpuLbvhNode {
    pub left: i32,
    pub right: i32,
    pub parent: i32,
    pub range_start: u32,
    pub range_end: u32,
    pub atomic_visited: u32,
    pub _pad: [u32; 2],
}

/// GPU BVH builder configuration.
#[derive(Debug, Clone)]
pub struct GpuBvhConfig {
    /// Use GPU for BVH construction (vs CPU fallback)
    pub enabled: bool,
    /// High quality BVH via PRBVH reinsertion (slower build) [future]
    #[allow(dead_code)]
    pub high_quality: bool,
    /// Collapse to 8-wide BVH for SIMD traversal [future]
    #[allow(dead_code)]
    pub wide_bvh: bool,
    /// Threshold: use GPU only if instance count >= this
    pub gpu_threshold: usize,
}

impl Default for GpuBvhConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            high_quality: false,
            wide_bvh: false,
            gpu_threshold: 512,
        }
    }
}

/// GPU BVH builder with refit support.
#[allow(dead_code)]
pub struct GpuBvhBuilder {
    // Pipelines
    morton_pipeline: wgpu::ComputePipeline,
    morton_bgl: wgpu::BindGroupLayout,

    radix_count_pipeline: wgpu::ComputePipeline,
    radix_scatter_pipeline: wgpu::ComputePipeline,
    radix_bgl: wgpu::BindGroupLayout,

    lbvh_pipeline: wgpu::ComputePipeline,
    lbvh_init_pipeline: wgpu::ComputePipeline,
    lbvh_bgl: wgpu::BindGroupLayout,

    aabb_leaf_pipeline: wgpu::ComputePipeline,
    aabb_internal_pipeline: wgpu::ComputePipeline,
    aabb_refit_pipeline: wgpu::ComputePipeline,
    aabb_bgl: wgpu::BindGroupLayout,

    // Buffers
    aabb_buffer: Option<wgpu::Buffer>,
    morton_a: Option<wgpu::Buffer>,
    morton_b: Option<wgpu::Buffer>,
    block_hist: Option<wgpu::Buffer>,
    block_offsets: Option<wgpu::Buffer>,
    lbvh_nodes: Option<wgpu::Buffer>,
    leaf_parents: Option<wgpu::Buffer>,
    sorted_in_a: bool,
    /// LBVH scratch output (`output_nodes` in `aabb_compute.wgsl`); persists
    /// between frames so `refit_leaves` can update AABBs before CPU linearize.
    output_nodes_buf: Option<wgpu::Buffer>,

    // Uniforms
    bounds_buffer: wgpu::Buffer,
    build_params_buffer: wgpu::Buffer,
    radix_params_buffer: wgpu::Buffer,

    // State
    capacity: usize,
    last_build_count: usize,
    has_valid_structure: bool,
}

const MORTON_WGSL: &str = include_str!("morton.wgsl");
const RADIX_WGSL: &str = include_str!("radix_sort.wgsl");
const LBVH_WGSL: &str = include_str!("lbvh_build.wgsl");
const AABB_WGSL: &str = include_str!("aabb_compute.wgsl");

impl GpuBvhBuilder {
    pub fn new(device: &wgpu::Device) -> Self {
        let morton_shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("bvh_morton_shader"),
            source: wgpu::ShaderSource::Wgsl(MORTON_WGSL.into()),
        });
        let morton_bgl = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("bvh_morton_bgl"),
            entries: &[
                bgl_entry(0, wgpu::BufferBindingType::Storage { read_only: true }), // aabbs
                bgl_entry(1, wgpu::BufferBindingType::Storage { read_only: false }), // morton out
                bgl_entry(2, wgpu::BufferBindingType::Uniform),                     // bounds
                bgl_entry(3, wgpu::BufferBindingType::Uniform),                     // params
            ],
        });
        let morton_pl = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("bvh_morton_pl"),
            bind_group_layouts: &[Some(&morton_bgl)],
            immediate_size: 0,
        });
        let morton_pipeline = device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
            label: Some("bvh_morton_pipeline"),
            layout: Some(&morton_pl),
            module: &morton_shader,
            entry_point: Some("main"),
            compilation_options: Default::default(),
            cache: None,
        });

        let radix_shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("bvh_radix_shader"),
            source: wgpu::ShaderSource::Wgsl(RADIX_WGSL.into()),
        });
        let radix_bgl = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("bvh_radix_bgl"),
            entries: &[
                bgl_entry(0, wgpu::BufferBindingType::Storage { read_only: true }), // input
                bgl_entry(1, wgpu::BufferBindingType::Storage { read_only: false }), // output
                bgl_entry(2, wgpu::BufferBindingType::Storage { read_only: false }), // histogram
                bgl_entry(3, wgpu::BufferBindingType::Storage { read_only: false }), // offsets
                bgl_entry(4, wgpu::BufferBindingType::Uniform),                     // params
            ],
        });
        let radix_pl = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("bvh_radix_pl"),
            bind_group_layouts: &[Some(&radix_bgl)],
            immediate_size: 0,
        });
        let radix_count_pipeline =
            device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
                label: Some("bvh_radix_count"),
                layout: Some(&radix_pl),
                module: &radix_shader,
                entry_point: Some("count_histogram"),
                compilation_options: Default::default(),
                cache: None,
            });
        let radix_scatter_pipeline =
            device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
                label: Some("bvh_radix_scatter"),
                layout: Some(&radix_pl),
                module: &radix_shader,
                entry_point: Some("scatter"),
                compilation_options: Default::default(),
                cache: None,
            });

        let lbvh_shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("bvh_lbvh_shader"),
            source: wgpu::ShaderSource::Wgsl(LBVH_WGSL.into()),
        });
        let lbvh_bgl = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("bvh_lbvh_bgl"),
            entries: &[
                bgl_entry(0, wgpu::BufferBindingType::Storage { read_only: true }), // sorted morton
                bgl_entry(1, wgpu::BufferBindingType::Storage { read_only: false }), // lbvh nodes
                bgl_entry(2, wgpu::BufferBindingType::Uniform),                     // params
                bgl_entry(3, wgpu::BufferBindingType::Storage { read_only: false }), // leaf parents
            ],
        });
        let lbvh_pl = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("bvh_lbvh_pl"),
            bind_group_layouts: &[Some(&lbvh_bgl)],
            immediate_size: 0,
        });
        let lbvh_pipeline = device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
            label: Some("bvh_lbvh_pipeline"),
            layout: Some(&lbvh_pl),
            module: &lbvh_shader,
            entry_point: Some("main"),
            compilation_options: Default::default(),
            cache: None,
        });
        let lbvh_init_pipeline = device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
            label: Some("bvh_lbvh_init_pipeline"),
            layout: Some(&lbvh_pl),
            module: &lbvh_shader,
            entry_point: Some("init_nodes"),
            compilation_options: Default::default(),
            cache: None,
        });

        let aabb_shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("bvh_aabb_shader"),
            source: wgpu::ShaderSource::Wgsl(AABB_WGSL.into()),
        });
        let aabb_bgl = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("bvh_aabb_bgl"),
            entries: &[
                bgl_entry(0, wgpu::BufferBindingType::Storage { read_only: true }), // aabbs
                bgl_entry(1, wgpu::BufferBindingType::Storage { read_only: true }), // sorted indices
                bgl_entry(2, wgpu::BufferBindingType::Storage { read_only: false }), // lbvh nodes
                bgl_entry(3, wgpu::BufferBindingType::Storage { read_only: false }), // output nodes
                bgl_entry(4, wgpu::BufferBindingType::Uniform),                     // params
                bgl_entry(5, wgpu::BufferBindingType::Storage { read_only: true }), // leaf parents
            ],
        });
        let aabb_pl = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("bvh_aabb_pl"),
            bind_group_layouts: &[Some(&aabb_bgl)],
            immediate_size: 0,
        });
        let aabb_leaf_pipeline = device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
            label: Some("bvh_aabb_leaf"),
            layout: Some(&aabb_pl),
            module: &aabb_shader,
            entry_point: Some("main"),
            compilation_options: Default::default(),
            cache: None,
        });
        // merged into leaf pipeline
        let aabb_internal_pipeline = aabb_leaf_pipeline.clone();
        let aabb_refit_pipeline =
            device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
                label: Some("bvh_aabb_refit"),
                layout: Some(&aabb_pl),
                module: &aabb_shader,
                entry_point: Some("refit_leaves"),
                compilation_options: Default::default(),
                cache: None,
            });

        let bounds_buffer = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("bvh_bounds"),
            size: std::mem::size_of::<SceneBounds>() as u64,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let build_params_buffer = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("bvh_build_params"),
            size: std::mem::size_of::<BuildParams>() as u64,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let radix_params_buffer = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("bvh_radix_params"),
            size: std::mem::size_of::<RadixParams>() as u64,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });

        Self {
            morton_pipeline,
            morton_bgl,
            radix_count_pipeline,
            radix_scatter_pipeline,
            radix_bgl,
            lbvh_pipeline,
            lbvh_init_pipeline,
            lbvh_bgl,
            aabb_leaf_pipeline,
            aabb_internal_pipeline,
            aabb_refit_pipeline,
            aabb_bgl,
            aabb_buffer: None,
            morton_a: None,
            morton_b: None,
            block_hist: None,
            block_offsets: None,
            lbvh_nodes: None,
            leaf_parents: None,
            sorted_in_a: true,
            output_nodes_buf: None,
            bounds_buffer,
            build_params_buffer,
            radix_params_buffer,
            capacity: 0,
            last_build_count: 0,
            has_valid_structure: false,
        }
    }

    /// Check if we can refit instead of rebuild (same instance count, valid structure).
    #[allow(dead_code)]
    pub fn can_refit(&self, instance_count: usize) -> bool {
        self.has_valid_structure && instance_count == self.last_build_count
    }

    fn ensure_output_nodes_buffer(&mut self, device: &wgpu::Device, leaf_count: usize) {
        let node_count = (2 * leaf_count).saturating_sub(1).max(1);
        let size = (node_count * std::mem::size_of::<BvhNode>()) as u64;
        let need_new = self
            .output_nodes_buf
            .as_ref()
            .map(|b| b.size() < size)
            .unwrap_or(true);
        if need_new {
            self.output_nodes_buf = Some(device.create_buffer(&wgpu::BufferDescriptor {
                label: Some("bvh_output_nodes"),
                size: size.max(std::mem::size_of::<BvhNode>() as u64),
                usage: wgpu::BufferUsages::STORAGE
                    | wgpu::BufferUsages::COPY_SRC
                    | wgpu::BufferUsages::COPY_DST,
                mapped_at_creation: false,
            }));
        }
    }

    /// Animation fast path: refit internal LBVH output, read back, linearize for PT.
    /// `sorted_leaf_indices` must match the last GPU full build (leaf instance order).
    pub fn try_refit_linearized(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        instances: &[Instance],
        sorted_leaf_indices: &[u32],
    ) -> Option<Vec<BvhNode>> {
        if !self.can_refit(instances.len())
            || sorted_leaf_indices.len() != instances.len()
            || self.output_nodes_buf.is_none()
        {
            return None;
        }
        let n = instances.len();
        self.update_aabbs(device, queue, instances);
        self.update_bounds(queue, instances);

        let n32 = n as u32;
        queue.write_buffer(
            &self.build_params_buffer,
            0,
            bytemuck::bytes_of(&BuildParams {
                count: n32,
                is_refit: 1,
                _pad: [0; 2],
            }),
        );

        let aabb_buf = self.aabb_buffer.as_ref()?;
        let sorted_buf = if self.sorted_in_a {
            self.morton_a.as_ref()?
        } else {
            self.morton_b.as_ref()?
        };
        let lbvh_buf = self.lbvh_nodes.as_ref()?;
        let leaf_parents = self.leaf_parents.as_ref()?;
        let out_buf = self.output_nodes_buf.as_ref()?;

        let aabb_bg = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("bvh_aabb_refit_linearized_bg"),
            layout: &self.aabb_bgl,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: aabb_buf.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: sorted_buf.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 2,
                    resource: lbvh_buf.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 3,
                    resource: out_buf.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 4,
                    resource: self.build_params_buffer.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 5,
                    resource: leaf_parents.as_entire_binding(),
                },
            ],
        });

        let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
            label: Some("bvh_refit_linearized_encoder"),
        });
        {
            let mut pass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor {
                label: Some("bvh_refit_leaves_linearized"),
                timestamp_writes: None,
            });
            pass.set_pipeline(&self.aabb_refit_pipeline);
            pass.set_bind_group(0, &aabb_bg, &[]);
            let wg = n32.div_ceil(256);
            pass.dispatch_workgroups(wg, 1, 1);
        }
        queue.submit(Some(encoder.finish()));

        let node_count = (2 * n).saturating_sub(1);
        let output_nodes: Vec<BvhNode> = read_buffer_vec(device, queue, out_buf, node_count);
        let lbvh_cpu: Vec<GpuLbvhNode> = read_buffer_vec(device, queue, lbvh_buf, n.saturating_sub(1));
        let leaf_parents_cpu: Vec<u32> = read_buffer_vec(device, queue, leaf_parents, n);

        validate_lbvh(&lbvh_cpu, n)
            .and_then(|_| validate_root_aabb(&output_nodes, instances, &lbvh_cpu))
            .and_then(|_| validate_leaf_parents(&lbvh_cpu, &leaf_parents_cpu, n))
            .ok()?;

        let nodes = linearize_lbvh(&lbvh_cpu, &output_nodes, n, sorted_leaf_indices);
        trace!("GpuBvhBuilder::try_refit_linearized OK (n={})", n);
        Some(nodes)
    }

    /// Full BVH build (first frame or structure change).
    pub fn build(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        instances: &[Instance],
        config: &GpuBvhConfig,
    ) -> (Vec<BvhNode>, Vec<u32>) {
        let n = instances.len();
        debug!(
            "GpuBvhBuilder::build n={}, gpu_enabled={}, threshold={}, last_build_count={}, has_valid_structure={}",
            n,
            config.enabled,
            config.gpu_threshold,
            self.last_build_count,
            self.has_valid_structure
        );

        if n < config.gpu_threshold || !config.enabled {
            self.has_valid_structure = false;
            debug!("GpuBvhBuilder::build: fallback to CPU (threshold/disabled)");
            return self.build_cpu(instances);
        }
        info!("GpuBvhBuilder::build: using GPU LBVH (n={})", n);
        match self.build_gpu(device, queue, instances) {
            Ok((nodes, indices)) => {
                info!("GpuBvhBuilder::build: GPU LBVH OK (n={})", n);
                self.last_build_count = n;
                self.has_valid_structure = true;
                (nodes, indices)
            }
            Err(err) => {
                warn!(
                    "GpuBvhBuilder::build: GPU LBVH invalid ({}), falling back to CPU",
                    err
                );
                self.has_valid_structure = false;
                self.build_cpu(instances)
            }
        }
    }

    /// Fast BVH refit for animation (AABBs only, preserve hierarchy).
    /// Returns true if refit was performed, false if full rebuild needed.
    #[allow(dead_code)]
    pub fn refit(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        instances: &[Instance],
        nodes_buffer: &wgpu::Buffer,
    ) -> bool {
        if !self.can_refit(instances.len()) {
            return false;
        }

        self.update_aabbs(device, queue, instances);

        let n = instances.len() as u32;
        queue.write_buffer(
            &self.build_params_buffer,
            0,
            bytemuck::bytes_of(&BuildParams {
                count: n,
                is_refit: 1,
                _pad: [0; 2],
            }),
        );

        let aabb_buf = self.aabb_buffer.as_ref().unwrap();
        let sorted_buf = if self.sorted_in_a {
            self.morton_a.as_ref().unwrap()
        } else {
            self.morton_b.as_ref().unwrap()
        };
        let lbvh_buf = self.lbvh_nodes.as_ref().unwrap();
        let leaf_parents = self.leaf_parents.as_ref().unwrap();

        let aabb_bg = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("bvh_aabb_refit_bg"),
            layout: &self.aabb_bgl,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: aabb_buf.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: sorted_buf.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 2,
                    resource: lbvh_buf.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 3,
                    resource: nodes_buffer.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 4,
                    resource: self.build_params_buffer.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 5,
                    resource: leaf_parents.as_entire_binding(),
                },
            ],
        });

        let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
            label: Some("bvh_refit_encoder"),
        });
        {
            let mut pass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor {
                label: Some("bvh_refit_leaves"),
                timestamp_writes: None,
            });
            pass.set_pipeline(&self.aabb_refit_pipeline);
            pass.set_bind_group(0, &aabb_bg, &[]);
            let wg = n.div_ceil(256);
            pass.dispatch_workgroups(wg, 1, 1);
        }
        // Internal AABB update merged into leaf/refit shader
        queue.submit(Some(encoder.finish()));

        true
    }

    fn build_gpu(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        instances: &[Instance],
    ) -> Result<(Vec<BvhNode>, Vec<u32>), String> {
        let n = instances.len();
        trace!("GpuBvhBuilder::build_gpu n={}", n);
        self.ensure_capacity(device, n);
        self.update_aabbs(device, queue, instances);
        self.update_bounds(queue, instances);
        self.ensure_output_nodes_buffer(device, n);

        let aabb_buf = self.aabb_buffer.as_ref().unwrap();
        let morton_a = self.morton_a.as_ref().unwrap();
        let morton_b = self.morton_b.as_ref().unwrap();
        let block_hist = self.block_hist.as_ref().unwrap();
        let block_offsets = self.block_offsets.as_ref().unwrap();
        let lbvh_nodes = self.lbvh_nodes.as_ref().unwrap();
        let leaf_parents = self.leaf_parents.as_ref().unwrap();

        queue.write_buffer(
            &self.build_params_buffer,
            0,
            bytemuck::bytes_of(&BuildParams {
                count: n as u32,
                is_refit: 0,
                _pad: [0; 2],
            }),
        );

        let morton_bg = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("bvh_morton_bg"),
            layout: &self.morton_bgl,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: aabb_buf.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: morton_a.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 2,
                    resource: self.bounds_buffer.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 3,
                    resource: self.build_params_buffer.as_entire_binding(),
                },
            ],
        });

        {
            let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
                label: Some("bvh_morton_encoder"),
            });
            {
                let mut pass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor {
                    label: Some("bvh_morton_pass"),
                    timestamp_writes: None,
                });
                pass.set_pipeline(&self.morton_pipeline);
                pass.set_bind_group(0, &morton_bg, &[]);
                let wg = (n as u32).div_ceil(256);
                pass.dispatch_workgroups(wg, 1, 1);
            }
            queue.submit(Some(encoder.finish()));
        }

        // Radix sort (4 passes for 32-bit, Morton uses 30-bit)
        let mut input = morton_a;
        let mut output = morton_b;
        for pass_idx in 0..4u32 {
            trace!("GpuBvhBuilder::radix pass {}", pass_idx);
            queue.write_buffer(
                &self.radix_params_buffer,
                0,
                bytemuck::bytes_of(&RadixParams {
                    count: n as u32,
                    pass: pass_idx,
                    _pad: [0; 2],
                }),
            );

            let num_blocks = (n as u32).div_ceil(256);
            let block_hist_len = (num_blocks as usize) * 256;
            let zeros = vec![0u32; block_hist_len];
            queue.write_buffer(block_hist, 0, bytemuck::cast_slice(&zeros));

            let radix_bg = device.create_bind_group(&wgpu::BindGroupDescriptor {
                label: Some("bvh_radix_bg"),
                layout: &self.radix_bgl,
                entries: &[
                    wgpu::BindGroupEntry {
                        binding: 0,
                        resource: input.as_entire_binding(),
                    },
                    wgpu::BindGroupEntry {
                        binding: 1,
                        resource: output.as_entire_binding(),
                    },
                    wgpu::BindGroupEntry {
                        binding: 2,
                        resource: block_hist.as_entire_binding(),
                    },
                    wgpu::BindGroupEntry {
                        binding: 3,
                        resource: block_offsets.as_entire_binding(),
                    },
                    wgpu::BindGroupEntry {
                        binding: 4,
                        resource: self.radix_params_buffer.as_entire_binding(),
                    },
                ],
            });

            {
                let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
                    label: Some("bvh_radix_count_encoder"),
                });
                {
                    let mut pass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor {
                        label: Some("bvh_radix_count"),
                        timestamp_writes: None,
                    });
                    pass.set_pipeline(&self.radix_count_pipeline);
                    pass.set_bind_group(0, &radix_bg, &[]);
                    pass.dispatch_workgroups(num_blocks, 1, 1);
                }
                queue.submit(Some(encoder.finish()));
            }
            let block_hist_cpu: Vec<u32> =
                read_buffer_vec(device, queue, block_hist, block_hist_len);
            let block_offsets_cpu = compute_block_offsets(&block_hist_cpu, num_blocks as usize);
            queue.write_buffer(block_offsets, 0, bytemuck::cast_slice(&block_offsets_cpu));

            {
                let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
                    label: Some("bvh_radix_scatter_encoder"),
                });
                {
                    let mut pass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor {
                        label: Some("bvh_radix_scatter"),
                        timestamp_writes: None,
                    });
                    pass.set_pipeline(&self.radix_scatter_pipeline);
                    pass.set_bind_group(0, &radix_bg, &[]);
                    pass.dispatch_workgroups(num_blocks, 1, 1);
                }
                queue.submit(Some(encoder.finish()));
            }

            std::mem::swap(&mut input, &mut output);
        }

        let sorted = input;
        self.sorted_in_a = std::ptr::eq(sorted, self.morton_a.as_ref().unwrap());

        let output_nodes_buf = self.output_nodes_buf.as_ref().unwrap();

        let run_lbvh = |sorted_buf: &wgpu::Buffer, output_buf: &wgpu::Buffer| {
            let lbvh_bg = device.create_bind_group(&wgpu::BindGroupDescriptor {
                label: Some("bvh_lbvh_bg"),
                layout: &self.lbvh_bgl,
                entries: &[
                    wgpu::BindGroupEntry {
                        binding: 0,
                        resource: sorted_buf.as_entire_binding(),
                    },
                    wgpu::BindGroupEntry {
                        binding: 1,
                        resource: lbvh_nodes.as_entire_binding(),
                    },
                    wgpu::BindGroupEntry {
                        binding: 2,
                        resource: self.build_params_buffer.as_entire_binding(),
                    },
                    wgpu::BindGroupEntry {
                        binding: 3,
                        resource: leaf_parents.as_entire_binding(),
                    },
                ],
            });

            let aabb_bg = device.create_bind_group(&wgpu::BindGroupDescriptor {
                label: Some("bvh_aabb_bg"),
                layout: &self.aabb_bgl,
                entries: &[
                    wgpu::BindGroupEntry {
                        binding: 0,
                        resource: aabb_buf.as_entire_binding(),
                    },
                    wgpu::BindGroupEntry {
                        binding: 1,
                        resource: sorted_buf.as_entire_binding(),
                    },
                    wgpu::BindGroupEntry {
                        binding: 2,
                        resource: lbvh_nodes.as_entire_binding(),
                    },
                    wgpu::BindGroupEntry {
                        binding: 3,
                        resource: output_buf.as_entire_binding(),
                    },
                    wgpu::BindGroupEntry {
                        binding: 4,
                        resource: self.build_params_buffer.as_entire_binding(),
                    },
                    wgpu::BindGroupEntry {
                        binding: 5,
                        resource: leaf_parents.as_entire_binding(),
                    },
                ],
            });

            let mut enc = device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
                label: Some("bvh_lbvh_encoder"),
            });
            enc.clear_buffer(leaf_parents, 0, Some((n * 4) as u64));
            {
                let mut pass = enc.begin_compute_pass(&wgpu::ComputePassDescriptor {
                    label: Some("bvh_lbvh_init"),
                    timestamp_writes: None,
                });
                pass.set_pipeline(&self.lbvh_init_pipeline);
                pass.set_bind_group(0, &lbvh_bg, &[]);
                let wg = (n as u32).saturating_sub(1).div_ceil(256);
                pass.dispatch_workgroups(wg, 1, 1);
            }
            {
                let mut pass = enc.begin_compute_pass(&wgpu::ComputePassDescriptor {
                    label: Some("bvh_lbvh_pass"),
                    timestamp_writes: None,
                });
                pass.set_pipeline(&self.lbvh_pipeline);
                pass.set_bind_group(0, &lbvh_bg, &[]);
                let wg = (n as u32).saturating_sub(1).div_ceil(256);
                pass.dispatch_workgroups(wg, 1, 1);
            }
            {
                let mut pass = enc.begin_compute_pass(&wgpu::ComputePassDescriptor {
                    label: Some("bvh_aabb_leaf"),
                    timestamp_writes: None,
                });
                pass.set_pipeline(&self.aabb_leaf_pipeline);
                pass.set_bind_group(0, &aabb_bg, &[]);
                let wg = (n as u32).div_ceil(256);
                pass.dispatch_workgroups(wg, 1, 1);
            }
            queue.submit(Some(enc.finish()));
        };

        let mut sorted_morton: Vec<MortonPrimitive> = read_buffer_vec(device, queue, sorted, n);
        let mut used_cpu_sort = false;
        if let Err(err) = validate_sorted_morton(&sorted_morton) {
            warn!("GpuBvhBuilder::build_gpu: GPU radix produced unsorted Morton codes ({}), retrying with CPU sort", err);
            sorted_morton
                .sort_unstable_by(|a, b| a.code.cmp(&b.code).then_with(|| a.index.cmp(&b.index)));
            let cpu_sorted_buf = self.morton_a.as_ref().unwrap();
            queue.write_buffer(cpu_sorted_buf, 0, bytemuck::cast_slice(&sorted_morton));
            self.sorted_in_a = true;
            used_cpu_sort = true;
            run_lbvh(cpu_sorted_buf, output_nodes_buf);
        } else {
            run_lbvh(sorted, output_nodes_buf);
        }

        let node_count = (2 * n).saturating_sub(1);
        let mut output_nodes: Vec<BvhNode> =
            read_buffer_vec(device, queue, output_nodes_buf, node_count);
        let mut lbvh_cpu: Vec<GpuLbvhNode> =
            read_buffer_vec(device, queue, lbvh_nodes, n.saturating_sub(1));
        let mut leaf_parents_cpu: Vec<u32> = read_buffer_vec(device, queue, leaf_parents, n);

        let mut validation_err = validate_lbvh(&lbvh_cpu, n)
            .and_then(|_| validate_root_aabb(&output_nodes, instances, &lbvh_cpu))
            .and_then(|_| validate_leaf_parents(&lbvh_cpu, &leaf_parents_cpu, n))
            .err();

        if let Some(err) = validation_err.take() {
            log_lbvh_roots(
                "GpuBvhBuilder::build_gpu: LBVH invalid (initial)",
                &lbvh_cpu,
            );
            if used_cpu_sort {
                return Err(err);
            }
            warn!("GpuBvhBuilder::build_gpu: LBVH invalid after GPU radix ({}), retrying with CPU-sorted Morton codes", err);
            sorted_morton
                .sort_unstable_by(|a, b| a.code.cmp(&b.code).then_with(|| a.index.cmp(&b.index)));
            let cpu_sorted_buf = self.morton_a.as_ref().unwrap();
            queue.write_buffer(cpu_sorted_buf, 0, bytemuck::cast_slice(&sorted_morton));
            self.sorted_in_a = true;
            run_lbvh(cpu_sorted_buf, output_nodes_buf);

            output_nodes = read_buffer_vec(device, queue, output_nodes_buf, node_count);
            lbvh_cpu = read_buffer_vec(device, queue, lbvh_nodes, n.saturating_sub(1));
            leaf_parents_cpu = read_buffer_vec(device, queue, leaf_parents, n);

            if let Err(err) = validate_lbvh(&lbvh_cpu, n)
                .and_then(|_| validate_root_aabb(&output_nodes, instances, &lbvh_cpu))
                .and_then(|_| validate_leaf_parents(&lbvh_cpu, &leaf_parents_cpu, n))
            {
                log_lbvh_roots(
                    "GpuBvhBuilder::build_gpu: LBVH invalid (after CPU sort)",
                    &lbvh_cpu,
                );
                return Err(err);
            }
        }

        let indices: Vec<u32> = sorted_morton.iter().map(|m| m.index).collect();
        warn!(
            "GpuBvhBuilder::build_gpu: LBVH validated successfully (n={})",
            n
        );
        let nodes = linearize_lbvh(&lbvh_cpu, &output_nodes, n, &indices);

        Ok((nodes, indices))
    }

    fn ensure_capacity(&mut self, device: &wgpu::Device, count: usize) {
        if count <= self.capacity {
            return;
        }
        self.capacity = count.max(1);

        let morton_size = (self.capacity * std::mem::size_of::<MortonPrimitive>()) as u64;
        let aabb_size = (self.capacity * std::mem::size_of::<GpuAabb>()) as u64;
        let lbvh_size =
            (self.capacity.saturating_sub(1).max(1) * std::mem::size_of::<GpuLbvhNode>()) as u64;
        let leaf_parents_size = (self.capacity * std::mem::size_of::<u32>()) as u64;
        let max_blocks = self.capacity.div_ceil(256);
        let block_hist_size = (max_blocks * 256 * std::mem::size_of::<u32>()) as u64;
        debug!(
            "GpuBvhBuilder::ensure_capacity capacity={}, morton={}KB, aabb={}KB, lbvh={}KB",
            self.capacity,
            morton_size / 1024,
            aabb_size / 1024,
            lbvh_size / 1024
        );

        self.aabb_buffer = Some(device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("bvh_aabb_buffer"),
            size: aabb_size.max(16),
            usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        }));
        self.morton_a = Some(device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("bvh_morton_a"),
            size: morton_size.max(16),
            usage: wgpu::BufferUsages::STORAGE
                | wgpu::BufferUsages::COPY_SRC
                | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        }));
        self.morton_b = Some(device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("bvh_morton_b"),
            size: morton_size.max(16),
            usage: wgpu::BufferUsages::STORAGE
                | wgpu::BufferUsages::COPY_SRC
                | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        }));
        self.block_hist = Some(device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("bvh_block_hist"),
            size: block_hist_size.max(16),
            usage: wgpu::BufferUsages::STORAGE
                | wgpu::BufferUsages::COPY_DST
                | wgpu::BufferUsages::COPY_SRC,
            mapped_at_creation: false,
        }));
        self.block_offsets = Some(device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("bvh_block_offsets"),
            size: block_hist_size.max(16),
            usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        }));
        self.lbvh_nodes = Some(device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("bvh_lbvh_nodes"),
            size: lbvh_size.max(16),
            usage: wgpu::BufferUsages::STORAGE
                | wgpu::BufferUsages::COPY_DST
                | wgpu::BufferUsages::COPY_SRC,
            mapped_at_creation: false,
        }));
        self.leaf_parents = Some(device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("bvh_leaf_parents"),
            size: leaf_parents_size.max(16),
            usage: wgpu::BufferUsages::STORAGE
                | wgpu::BufferUsages::COPY_DST
                | wgpu::BufferUsages::COPY_SRC,
            mapped_at_creation: false,
        }));
    }

    fn update_aabbs(&mut self, device: &wgpu::Device, queue: &wgpu::Queue, instances: &[Instance]) {
        let aabbs: Vec<GpuAabb> = instances.iter().map(|i| i.aabb_to_gpu()).collect();
        if self.aabb_buffer.is_none() {
            self.aabb_buffer = Some(
                device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
                    label: Some("bvh_aabb_buffer"),
                    contents: bytemuck::cast_slice(&aabbs),
                    usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_DST,
                }),
            );
        } else if let Some(buf) = &self.aabb_buffer {
            queue.write_buffer(buf, 0, bytemuck::cast_slice(&aabbs));
        }
    }

    fn update_bounds(&self, queue: &wgpu::Queue, instances: &[Instance]) {
        let mut min = [f32::INFINITY; 3];
        let mut max = [f32::NEG_INFINITY; 3];
        for inst in instances {
            for i in 0..3 {
                min[i] = min[i].min(inst.aabb.min[i]);
                max[i] = max[i].max(inst.aabb.max[i]);
            }
        }
        let bounds = SceneBounds {
            min: [min[0], min[1], min[2], 0.0],
            max: [max[0], max[1], max[2], 0.0],
        };
        queue.write_buffer(&self.bounds_buffer, 0, bytemuck::bytes_of(&bounds));
    }

    /// CPU BVH build using SAH.
    fn build_cpu(&self, instances: &[Instance]) -> (Vec<BvhNode>, Vec<u32>) {
        let bvh = pt_core::build::build_instance_bvh(instances);
        let indices: Vec<u32> = bvh.tri_indices.iter().map(|&i| i as u32).collect();
        (bvh.nodes, indices)
    }

    /// Invalidate structure (forces full rebuild next time).
    #[allow(dead_code)]
    pub fn invalidate(&mut self) {
        self.has_valid_structure = false;
    }
}

fn linearize_lbvh(
    lbvh: &[GpuLbvhNode],
    output_nodes: &[BvhNode],
    leaf_count: usize,
    sorted_indices: &[u32],
) -> Vec<BvhNode> {
    if leaf_count == 0 {
        return vec![];
    }
    if leaf_count == 1 {
        let leaf = &output_nodes[0];
        return vec![BvhNode {
            aabb_min: leaf.aabb_min,
            aabb_max: leaf.aabb_max,
            left_or_first: 0,
            count: 1,
        }];
    }

    let mut nodes: Vec<BvhNode> = Vec::with_capacity(2 * leaf_count - 1);
    nodes.push(BvhNode {
        aabb_min: [0.0; 3],
        left_or_first: 0,
        aabb_max: [0.0; 3],
        count: 0,
    });

    struct Task {
        gpu_idx: i32,
        out_idx: usize,
    }

    // Ищем корень: в LBVH Karras это узел, который не является ничьим потомком.
    // Если не найдем (или их несколько), начнем с 0, но с защитой.
    let mut root_idx = 0i32;
    for (i, node) in lbvh.iter().enumerate() {
        if node.parent == -1 {
            root_idx = i as i32;
            break;
        }
    }

    let mut stack = vec![Task {
        gpu_idx: root_idx,
        out_idx: 0,
    }];
    let mut visited = std::collections::HashSet::new();
    let mut safety_counter = 0;
    let max_iters = leaf_count * 4;

    while let Some(task) = stack.pop() {
        safety_counter += 1;
        if safety_counter > max_iters {
            log::error!("linearize_lbvh: detected infinite loop or too deep tree!");
            break;
        }

        if task.gpu_idx < 0 {
            let leaf_idx = (-task.gpu_idx - 1) as usize;
            if leaf_idx >= leaf_count {
                continue;
            }

            let leaf_out = &output_nodes[leaf_count - 1 + leaf_idx];
            nodes[task.out_idx] = BvhNode {
                aabb_min: leaf_out.aabb_min,
                aabb_max: leaf_out.aabb_max,
                left_or_first: leaf_idx as u32,
                count: 1,
            };
            continue;
        }

        let gpu_idx_u = task.gpu_idx as usize;
        if gpu_idx_u >= lbvh.len() {
            continue;
        }

        if !visited.insert(task.gpu_idx) {
            log::error!("linearize_lbvh: detected cycle at node {}!", task.gpu_idx);
            continue;
        }

        let gpu = &lbvh[gpu_idx_u];
        let aabb = &output_nodes[gpu_idx_u];

        let left_idx = nodes.len();
        nodes.push(BvhNode {
            aabb_min: [0.0; 3],
            left_or_first: 0,
            aabb_max: [0.0; 3],
            count: 0,
        });
        let right_idx = nodes.len();
        nodes.push(BvhNode {
            aabb_min: [0.0; 3],
            left_or_first: 0,
            aabb_max: [0.0; 3],
            count: 0,
        });

        nodes[task.out_idx] = BvhNode {
            aabb_min: aabb.aabb_min,
            aabb_max: aabb.aabb_max,
            left_or_first: left_idx as u32,
            count: 0,
        };

        stack.push(Task {
            gpu_idx: gpu.right,
            out_idx: right_idx,
        });
        stack.push(Task {
            gpu_idx: gpu.left,
            out_idx: left_idx,
        });
    }

    // Ensure leaf ordering matches sorted_indices length
    if nodes.iter().filter(|n| n.count > 0).count() != sorted_indices.len() {
        return nodes;
    }

    nodes
}

fn validate_lbvh(lbvh: &[GpuLbvhNode], leaf_count: usize) -> Result<(), String> {
    if leaf_count <= 1 {
        return Ok(());
    }
    let internal_count = leaf_count.saturating_sub(1);
    if lbvh.len() != internal_count {
        return Err(format!(
            "lbvh invalid: internal_count={}, lbvh_len={}",
            internal_count,
            lbvh.len()
        ));
    }

    let mut root_idx: i32 = -1;
    for (i, node) in lbvh.iter().enumerate() {
        let idx = i as i32;

        if node.left == idx || node.right == idx {
            return Err(format!("lbvh invalid: self-child at node {}", idx));
        }

        for &child in &[node.left, node.right] {
            if child < 0 {
                let leaf_idx = (-child - 1) as usize;
                if leaf_idx >= leaf_count {
                    return Err(format!("lbvh invalid: leaf idx out of range {}", leaf_idx));
                }
            } else {
                let child_u = child as usize;
                if child_u >= internal_count {
                    return Err(format!("lbvh invalid: child idx out of range {}", child_u));
                }
            }
        }

        if node.parent < 0 {
            if root_idx >= 0 {
                return Err("lbvh invalid: multiple roots".to_string());
            }
            root_idx = idx;
        } else {
            let parent_u = node.parent as usize;
            if parent_u >= internal_count {
                return Err(format!(
                    "lbvh invalid: parent idx out of range {}",
                    parent_u
                ));
            }
        }
    }

    if root_idx < 0 {
        return Err("lbvh invalid: no root".to_string());
    }
    let root_u = root_idx as usize;
    if root_u >= lbvh.len() {
        return Err(format!("lbvh invalid: root index out of bounds {}", root_u));
    }
    let root = &lbvh[root_u];
    if root.range_start != 0 || root.range_end != (leaf_count.saturating_sub(1) as u32) {
        return Err(format!(
            "lbvh invalid: root range not full (start={}, end={}, expected 0..{})",
            root.range_start,
            root.range_end,
            leaf_count.saturating_sub(1)
        ));
    }

    let mut visited = vec![false; internal_count];
    let mut stack = vec![root_idx];
    while let Some(idx) = stack.pop() {
        let idx_u = idx as usize;
        if visited[idx_u] {
            return Err(format!("lbvh invalid: cycle detected at node {}", idx));
        }
        visited[idx_u] = true;
        let node = &lbvh[idx_u];
        for &child in &[node.left, node.right] {
            if child >= 0 {
                let child_u = child as usize;
                if lbvh[child_u].parent != idx {
                    return Err(format!(
                        "lbvh invalid: parent mismatch child={}, parent={}",
                        child_u, idx
                    ));
                }
                let child_node = &lbvh[child_u];
                if child_node.range_start < node.range_start
                    || child_node.range_end > node.range_end
                {
                    return Err(format!(
                        "lbvh invalid: child range out of parent bounds child={} ({}..{}) parent={} ({}..{})",
                        child_u,
                        child_node.range_start,
                        child_node.range_end,
                        idx,
                        node.range_start,
                        node.range_end
                    ));
                }
                stack.push(child);
            }
        }
    }

    let visited_count = visited.iter().filter(|v| **v).count();
    if visited_count != internal_count {
        return Err(format!(
            "lbvh invalid: not all nodes reachable (visited {}, total {})",
            visited_count, internal_count
        ));
    }

    Ok(())
}

fn validate_root_aabb(
    output_nodes: &[BvhNode],
    instances: &[Instance],
    lbvh: &[GpuLbvhNode],
) -> Result<(), String> {
    if instances.is_empty() {
        return Ok(());
    }
    if output_nodes.is_empty() {
        return Err("lbvh invalid: output_nodes empty".to_string());
    }

    let mut root_idx: i32 = -1;
    for (i, node) in lbvh.iter().enumerate() {
        if node.parent < 0 {
            if root_idx >= 0 {
                return Err("lbvh invalid: multiple roots (root AABB)".to_string());
            }
            root_idx = i as i32;
        }
    }
    if root_idx < 0 {
        return Err("lbvh invalid: no root (root AABB)".to_string());
    }
    let root_u = root_idx as usize;
    if root_u >= output_nodes.len() {
        return Err(format!(
            "lbvh invalid: root index out of bounds (root={}, nodes={})",
            root_u,
            output_nodes.len()
        ));
    }

    let mut min = [f32::INFINITY; 3];
    let mut max = [f32::NEG_INFINITY; 3];
    for inst in instances {
        for i in 0..3 {
            min[i] = min[i].min(inst.aabb.min[i]);
            max[i] = max[i].max(inst.aabb.max[i]);
        }
    }

    let root = &output_nodes[root_u];
    let eps = 1e-3f32;
    for i in 0..3 {
        if (root.aabb_min[i] - min[i]).abs() > eps || (root.aabb_max[i] - max[i]).abs() > eps {
            return Err(format!(
                "lbvh invalid: root AABB mismatch axis {} (gpu min/max = [{:.6},{:.6}], cpu min/max = [{:.6},{:.6}])",
                i,
                root.aabb_min[i],
                root.aabb_max[i],
                min[i],
                max[i]
            ));
        }
    }

    Ok(())
}

fn validate_leaf_parents(
    lbvh: &[GpuLbvhNode],
    leaf_parents: &[u32],
    leaf_count: usize,
) -> Result<(), String> {
    if leaf_count == 0 {
        return Ok(());
    }
    let internal_count = leaf_count.saturating_sub(1);
    if leaf_parents.len() != leaf_count {
        return Err(format!(
            "lbvh invalid: leaf_parents len={} leaf_count={}",
            leaf_parents.len(),
            leaf_count
        ));
    }
    for (leaf_idx, &parent_u32) in leaf_parents.iter().enumerate() {
        if leaf_count == 1 {
            break;
        }
        if parent_u32 as usize >= internal_count {
            return Err(format!(
                "lbvh invalid: leaf_parent out of range leaf={} parent={}",
                leaf_idx, parent_u32
            ));
        }
        let parent = &lbvh[parent_u32 as usize];
        let leaf_ref = -((leaf_idx as i32) + 1);
        if parent.left != leaf_ref && parent.right != leaf_ref {
            return Err(format!(
                "lbvh invalid: leaf_parent mismatch leaf={} parent={} (left={}, right={})",
                leaf_idx, parent_u32, parent.left, parent.right
            ));
        }
    }
    Ok(())
}

fn validate_sorted_morton(sorted: &[MortonPrimitive]) -> Result<(), String> {
    if sorted.is_empty() {
        return Ok(());
    }
    let mut prev = &sorted[0];
    for (i, cur) in sorted.iter().enumerate().skip(1) {
        if cur.code < prev.code || (cur.code == prev.code && cur.index < prev.index) {
            return Err(format!(
                "unsorted at {}: prev(code={}, idx={}) cur(code={}, idx={})",
                i, prev.code, prev.index, cur.code, cur.index
            ));
        }
        prev = cur;
    }
    Ok(())
}

fn log_lbvh_roots(tag: &str, lbvh: &[GpuLbvhNode]) {
    if lbvh.is_empty() {
        return;
    }
    let roots: Vec<usize> = lbvh
        .iter()
        .enumerate()
        .filter(|(_, node)| node.parent < 0)
        .map(|(i, _)| i)
        .collect();
    if roots.len() <= 1 {
        return;
    }
    let sample: Vec<String> = roots
        .iter()
        .take(6)
        .map(|&i| {
            let node = &lbvh[i];
            format!(
                "{}(l={}, r={}, rs={}, re={})",
                i, node.left, node.right, node.range_start, node.range_end
            )
        })
        .collect();
    warn!(
        "{}: multiple roots detected (count={}): {}",
        tag,
        roots.len(),
        sample.join(", ")
    );
}

fn compute_block_offsets(block_hist: &[u32], num_blocks: usize) -> Vec<u32> {
    let mut offsets = vec![0u32; num_blocks * 256];
    let mut digit_totals = [0u32; 256];
    for d in 0..256usize {
        let mut total = 0u32;
        for b in 0..num_blocks {
            total = total.saturating_add(block_hist[b * 256 + d]);
        }
        digit_totals[d] = total;
    }

    let mut digit_base = [0u32; 256];
    let mut running = 0u32;
    for d in 0..256usize {
        digit_base[d] = running;
        running = running.saturating_add(digit_totals[d]);
    }

    #[allow(clippy::needless_range_loop)]
    for d in 0..256usize {
        let mut sum = digit_base[d];
        for b in 0..num_blocks {
            let idx = b * 256 + d;
            offsets[idx] = sum;
            sum = sum.saturating_add(block_hist[idx]);
        }
    }
    offsets
}

fn read_buffer_vec<T: Pod>(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    src: &wgpu::Buffer,
    count: usize,
) -> Vec<T> {
    if count == 0 {
        return Vec::new();
    }
    let size = (count * std::mem::size_of::<T>()) as u64;
    let staging = device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("bvh_readback"),
        size,
        usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
        mapped_at_creation: false,
    });
    let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
        label: Some("bvh_readback_encoder"),
    });
    encoder.copy_buffer_to_buffer(src, 0, &staging, 0, size);
    queue.submit(Some(encoder.finish()));

    let slice = staging.slice(..);
    let (tx, rx) = mpsc::channel();
    slice.map_async(wgpu::MapMode::Read, move |r| {
        let _ = tx.send(r);
    });
    // Must wait for map_async callback before rx.recv()
    let _ = device.poll(wgpu::PollType::wait_indefinitely());
    rx.recv().unwrap().unwrap();
    let data = slice.get_mapped_range();
    let out = bytemuck::cast_slice(&data[..(size as usize)]).to_vec();
    drop(data);
    staging.unmap();
    out
}

// Helper to create bind group layout entry
fn bgl_entry(binding: u32, ty: wgpu::BufferBindingType) -> wgpu::BindGroupLayoutEntry {
    wgpu::BindGroupLayoutEntry {
        binding,
        visibility: wgpu::ShaderStages::COMPUTE,
        ty: wgpu::BindingType::Buffer {
            ty,
            has_dynamic_offset: false,
            min_binding_size: None,
        },
        count: None,
    }
}

#[cfg(test)]
mod tests {
    use super::{validate_lbvh, GpuLbvhNode};

    fn node(left: i32, right: i32, parent: i32) -> GpuLbvhNode {
        GpuLbvhNode {
            left,
            right,
            parent,
            range_start: 0,
            range_end: 0,
            atomic_visited: 0,
            _pad: [0; 2],
        }
    }

    #[test]
    fn validate_lbvh_accepts_minimal_tree() {
        // Root covers leaf range 0..1 (2 leaves)
        let mut root = node(-1, -2, -1);
        root.range_end = 1;
        let lbvh = vec![root];
        assert!(validate_lbvh(&lbvh, 2).is_ok());
    }

    #[test]
    fn validate_lbvh_rejects_cycle() {
        let lbvh = vec![node(0, -2, -1)];
        assert!(validate_lbvh(&lbvh, 2).is_err());
    }
}
