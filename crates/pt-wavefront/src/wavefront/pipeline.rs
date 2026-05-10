//! Wavefront pipeline orchestration.
//!
//! Per-tile state (dims uniform + counts) lives in N-slot persistent buffers
//! addressed via *dynamic offsets* in the bind group, so all per-tile values
//! for one frame can be uploaded with a single `queue.write_buffer` per buffer
//! before the encoder runs. Per-tile resets happen via
//! `encoder.copy_buffer_to_buffer`, which is ordered with the dispatches and
//! avoids the WebGPU pre-submit write race that loses per-tile state when
//! `queue.write_buffer` is called repeatedly inside a tile loop.
//!
//! WGSL shaders see a plain `Dims` uniform / `array<atomic<u32>>` count and
//! are unaware of the multi-slot layout — dynamic offset is transparent.

use super::buffers::{WfHit, WfRay};
use bytemuck::{Pod, Zeroable};
use log::debug;
use std::num::NonZeroU64;

/// Dims uniform for raygen pass. Mirrored 1:1 in raygen.wgsl `struct Dims`.
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

/// Per-tile slot stride for dynamic-offset bind groups.
/// 256 bytes is the WebGPU minimum dynamic offset alignment for both
/// uniform and storage buffers; using it for both keeps offsets uniform.
pub const TILE_SLOT_STRIDE: u64 = 256;
/// Bytes actually consumed per tile slot for `WfDims` (the rest is alignment pad).
pub const WF_DIMS_SIZE: u64 = std::mem::size_of::<WfDims>() as u64;
/// Bytes actually consumed per tile slot for the [count_in, count_out, _, _] u32x4 block.
pub const WF_COUNTS_SIZE: u64 = 16;
/// Default initial tile slot capacity. Grows on demand via `prepare_tiles`.
const DEFAULT_TILE_CAPACITY: u32 = 64;

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

    // Per-tile dims uniform: tile_capacity * TILE_SLOT_STRIDE bytes.
    // Bound with has_dynamic_offset; per-dispatch offset selects a tile.
    tile_dims_buf: wgpu::Buffer,
    // Per-tile [count_in, count_out, _, _] storage: tile_capacity * TILE_SLOT_STRIDE.
    // Mutated via atomic ops in shaders; reset before each tile via
    // `reset_tile_count` from `count_init_src`.
    tile_counts_buf: wgpu::Buffer,
    // Source buffer holding initial [tile_pixels, 0, 0, 0] for each tile slot.
    // Filled once per dispatch via prepare_tiles, copied per-tile via reset_tile_count.
    count_init_src: wgpu::Buffer,
    tile_capacity: u32,

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
                bgl_uniform(0),                                // camera
                bgl_uniform_dyn(1, WF_DIMS_SIZE),              // dims (per-tile)
                bgl_storage_rw(2),                             // ray output
                bgl_storage_rw_dyn(3, WF_COUNTS_SIZE),         // count (per-tile)
                bgl_storage_ro(4),                             // sample map
                bgl_storage_ro(5),                             // accum (read-only check)
            ],
        );

        let (intersect_pipeline, intersect_bgl) = create_pipeline(
            device,
            INTERSECT_WGSL,
            "intersect",
            &[
                bgl_storage_ro(0),                             // nodes
                bgl_storage_ro(1),                             // instances
                bgl_storage_ro(2),                             // rays
                bgl_storage_rw(3),                             // hits
                bgl_storage_rw_dyn(4, WF_COUNTS_SIZE),         // count (per-tile)
            ],
        );

        let (shade_pipeline, shade_bgl) = create_pipeline(
            device,
            SHADE_WGSL,
            "shade",
            &[
                bgl_storage_ro(0),                             // instances
                bgl_storage_ro(1),                             // materials
                bgl_storage_ro(2),                             // rays in
                bgl_storage_ro(3),                             // hits
                bgl_storage_rw(4),                             // rays out
                bgl_storage_rw(5),                             // accum
                bgl_storage_rw_dyn(6, WF_COUNTS_SIZE),         // counts (per-tile)
                bgl_uniform(7),                                // params
                bgl_texture_2d(8),                             // env_map
                bgl_sampler(9),                                // env_sampler
                bgl_uniform(10),                               // env_params
                bgl_storage_rw(11),                            // guide buffer
            ],
        );

        let (finalize_pipeline, finalize_bgl) = create_finalize_pipeline(device);
        let (count_swap_pipeline, count_swap_bgl) = create_pipeline(
            device,
            COUNT_SWAP_WGSL,
            "count_swap",
            &[
                bgl_storage_rw_dyn(0, WF_COUNTS_SIZE),         // counts (per-tile)
            ],
        );

        let tile_dims_buf = create_tile_dims_buf(device, DEFAULT_TILE_CAPACITY);
        let tile_counts_buf = create_tile_counts_buf(device, DEFAULT_TILE_CAPACITY);
        let count_init_src = create_count_init_src(device, DEFAULT_TILE_CAPACITY);

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
            tile_dims_buf,
            tile_counts_buf,
            count_init_src,
            tile_capacity: DEFAULT_TILE_CAPACITY,
            width: 0,
            height: 0,
            cur_buf: 0,
        };
        p.resize(device, width, height);
        p
    }

    /// Resize ray/hit buffers for new viewport (or wavefront tile capacity).
    /// Caller must rebuild bind groups after this.
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

    /// Per-tile dims uniform buffer. Bind with `has_dynamic_offset: true`
    /// and `min_binding_size = WF_DIMS_SIZE`.
    pub fn tile_dims_buf(&self) -> &wgpu::Buffer {
        &self.tile_dims_buf
    }
    /// Per-tile counts storage buffer. Bind with `has_dynamic_offset: true`
    /// and `min_binding_size = WF_COUNTS_SIZE`.
    pub fn tile_counts_buf(&self) -> &wgpu::Buffer {
        &self.tile_counts_buf
    }

    pub fn dimensions(&self) -> (u32, u32) {
        (self.width, self.height)
    }

    /// Dynamic offset (bytes) for tile `idx` into the per-tile dims/counts buffers.
    pub fn tile_offset(&self, idx: u32) -> u32 {
        debug_assert!(idx < self.tile_capacity, "tile_idx out of range");
        idx.checked_mul(TILE_SLOT_STRIDE as u32).expect("tile offset overflow")
    }

    /// Upload all per-tile dims and the per-tile count-init source for one frame's
    /// dispatch with **one `queue.write_buffer` per buffer**. Grows tile capacity
    /// (and therefore reallocates the per-tile buffers) if `dims.len()` exceeds
    /// the current capacity. Returns true if a reallocation happened (caller must
    /// rebuild bind groups in that case).
    pub fn prepare_tiles(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        dims: &[WfDims],
        count_inits: &[[u32; 4]],
    ) -> bool {
        assert_eq!(dims.len(), count_inits.len(), "tile param length mismatch");
        let n = dims.len() as u32;
        if n == 0 {
            return false;
        }
        let realloc = if n > self.tile_capacity {
            // Grow to next power-of-two ≥ n, capped reasonably.
            let new_cap = n.next_power_of_two().max(DEFAULT_TILE_CAPACITY);
            debug!(
                "WavefrontPipeline::prepare_tiles grow capacity {} -> {}",
                self.tile_capacity, new_cap
            );
            self.tile_dims_buf = create_tile_dims_buf(device, new_cap);
            self.tile_counts_buf = create_tile_counts_buf(device, new_cap);
            self.count_init_src = create_count_init_src(device, new_cap);
            self.tile_capacity = new_cap;
            true
        } else {
            false
        };

        // Pack dims into stride-padded blob: TILE_SLOT_STRIDE bytes per slot,
        // first WF_DIMS_SIZE bytes hold WfDims, rest is zero pad.
        let stride = TILE_SLOT_STRIDE as usize;
        let mut dims_blob = vec![0u8; n as usize * stride];
        for (i, d) in dims.iter().enumerate() {
            let off = i * stride;
            let bytes = bytemuck::bytes_of(d);
            dims_blob[off..off + bytes.len()].copy_from_slice(bytes);
        }
        queue.write_buffer(&self.tile_dims_buf, 0, &dims_blob);

        // Pack count-init values similarly.
        let mut count_blob = vec![0u8; n as usize * stride];
        for (i, c) in count_inits.iter().enumerate() {
            let off = i * stride;
            let bytes = bytemuck::bytes_of(c);
            count_blob[off..off + bytes.len()].copy_from_slice(bytes);
        }
        queue.write_buffer(&self.count_init_src, 0, &count_blob);

        realloc
    }

    /// Reset tile `idx`'s [count_in, count_out, _, _] block by copying the
    /// init slot into the live counts buffer. **This goes through the encoder
    /// and is therefore ordered with the subsequent dispatches**, fixing the
    /// race that exists when using `queue.write_buffer` per-tile.
    pub fn reset_tile_count(&self, encoder: &mut wgpu::CommandEncoder, idx: u32) {
        debug_assert!(idx < self.tile_capacity, "tile_idx out of range");
        let off = u64::from(idx) * TILE_SLOT_STRIDE;
        encoder.copy_buffer_to_buffer(
            &self.count_init_src,
            off,
            &self.tile_counts_buf,
            off,
            WF_COUNTS_SIZE,
        );
    }
}

// ── pipeline / bgl helpers ──

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

fn create_tile_dims_buf(device: &wgpu::Device, capacity: u32) -> wgpu::Buffer {
    device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("wf_tile_dims"),
        size: u64::from(capacity) * TILE_SLOT_STRIDE,
        usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
        mapped_at_creation: false,
    })
}

fn create_tile_counts_buf(device: &wgpu::Device, capacity: u32) -> wgpu::Buffer {
    device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("wf_tile_counts"),
        size: u64::from(capacity) * TILE_SLOT_STRIDE,
        usage: wgpu::BufferUsages::STORAGE
            | wgpu::BufferUsages::COPY_DST
            | wgpu::BufferUsages::COPY_SRC,
        mapped_at_creation: false,
    })
}

fn create_count_init_src(device: &wgpu::Device, capacity: u32) -> wgpu::Buffer {
    device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("wf_tile_count_init_src"),
        size: u64::from(capacity) * TILE_SLOT_STRIDE,
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

fn bgl_storage_rw_dyn(binding: u32, size: u64) -> wgpu::BindGroupLayoutEntry {
    wgpu::BindGroupLayoutEntry {
        binding,
        visibility: wgpu::ShaderStages::COMPUTE,
        ty: wgpu::BindingType::Buffer {
            ty: wgpu::BufferBindingType::Storage { read_only: false },
            has_dynamic_offset: true,
            min_binding_size: NonZeroU64::new(size),
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
