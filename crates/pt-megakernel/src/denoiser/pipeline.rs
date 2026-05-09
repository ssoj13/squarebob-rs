//! À-trous denoiser pipeline.
//!
//! Reads the path-tracer output texture (Rgba32Float) and produces a
//! denoised texture of the same format/size by running 1–5 iterations
//! of the à-trous edge-aware filter (color-only edge stopping in this
//! MVP — see `atrous.wgsl` for the algorithm).
//!
//! Architecture:
//! - Two ping-pong textures (`tex_a`, `tex_b`) sized to the viewport.
//! - For odd `iterations`, the final result lands in `tex_a`; for even,
//!   in `tex_b`. Caller reads back via `output_view()`.
//! - Stride doubles each iteration (1, 2, 4, 8, 16). After 5 passes
//!   the effective filter footprint is ~64×64 pixels.
//!
//! Expansion path for G-buffer guidance (deferred): add normal/depth
//! texture bindings + sigma_normal/sigma_depth params; the WGSL
//! kernel structure stays the same. The wavefront PT already produces
//! these in `wavefront/gbuffer.wgsl`; megakernel would need its own.

use bytemuck::{Pod, Zeroable};

const ATROUS_WGSL: &str = include_str!("atrous.wgsl");

#[repr(C)]
#[derive(Debug, Clone, Copy, Pod, Zeroable)]
struct Params {
    width: u32,
    height: u32,
    stride: u32,
    sigma_color_inv: f32,
}

pub struct DenoiserPipeline {
    pipeline: wgpu::ComputePipeline,
    bgl: wgpu::BindGroupLayout,
    params_buf: wgpu::Buffer,

    /// Ping-pong target textures. Both Rgba32Float STORAGE+TEXTURE.
    tex_a: Option<wgpu::Texture>,
    tex_b: Option<wgpu::Texture>,
    view_a: Option<wgpu::TextureView>,
    view_b: Option<wgpu::TextureView>,

    /// Tracks which texture holds the most recent valid output.
    /// Updated by `dispatch`. Caller reads via `output_view()`.
    last_output_is_a: bool,

    width: u32,
    height: u32,
}

impl DenoiserPipeline {
    pub fn new(device: &wgpu::Device, width: u32, height: u32) -> Self {
        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("denoiser_atrous_shader"),
            source: wgpu::ShaderSource::Wgsl(ATROUS_WGSL.into()),
        });

        let bgl = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("denoiser_atrous_bgl"),
            entries: &[
                // input_tex: texture_2d<f32>
                wgpu::BindGroupLayoutEntry {
                    binding: 0,
                    visibility: wgpu::ShaderStages::COMPUTE,
                    ty: wgpu::BindingType::Texture {
                        sample_type: wgpu::TextureSampleType::Float { filterable: false },
                        view_dimension: wgpu::TextureViewDimension::D2,
                        multisampled: false,
                    },
                    count: None,
                },
                // output_tex: texture_storage_2d<rgba32float, write>
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
                // params: uniform
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

        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("denoiser_atrous_pl"),
            bind_group_layouts: &[Some(&bgl)],
            immediate_size: 0,
        });

        let pipeline = device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
            label: Some("denoiser_atrous_pipeline"),
            layout: Some(&pipeline_layout),
            module: &shader,
            entry_point: Some("atrous"),
            compilation_options: Default::default(),
            cache: None,
        });

        let params_buf = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("denoiser_atrous_params"),
            size: std::mem::size_of::<Params>() as u64,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });

        let mut p = Self {
            pipeline,
            bgl,
            params_buf,
            tex_a: None,
            tex_b: None,
            view_a: None,
            view_b: None,
            last_output_is_a: false,
            width: 0,
            height: 0,
        };
        p.resize(device, width, height);
        p
    }

    pub fn resize(&mut self, device: &wgpu::Device, width: u32, height: u32) {
        if self.width == width && self.height == height && self.tex_a.is_some() {
            return;
        }
        let (tex_a, view_a) = make_pingpong(device, "denoiser_a", width, height);
        let (tex_b, view_b) = make_pingpong(device, "denoiser_b", width, height);
        self.tex_a = Some(tex_a);
        self.view_a = Some(view_a);
        self.tex_b = Some(tex_b);
        self.view_b = Some(view_b);
        self.width = width;
        self.height = height;
        self.last_output_is_a = false;
    }

    /// Run `iterations` à-trous passes reading initially from `noisy_input_view`
    /// and ping-ponging between internal textures. After this call,
    /// `output_view()` returns the final denoised texture view.
    ///
    /// `sigma_color` is in linear-color units. Smaller values give more
    /// aggressive smoothing (less edge preservation). Typical: 0.1–0.5.
    pub fn dispatch(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        encoder: &mut wgpu::CommandEncoder,
        noisy_input_view: &wgpu::TextureView,
        iterations: u32,
        sigma_color: f32,
    ) {
        let iterations = iterations.clamp(1, 5);
        let sigma_color = sigma_color.max(1e-3);
        let sigma_color_inv = 1.0 / (sigma_color * sigma_color);

        let view_a = self.view_a.as_ref().expect("denoiser textures missing");
        let view_b = self.view_b.as_ref().expect("denoiser textures missing");

        // Iteration 0: input → tex_a (output_is_a = true)
        // Iteration 1: tex_a → tex_b (output_is_a = false)
        // Iteration 2: tex_b → tex_a (output_is_a = true)
        // ...
        let mut output_is_a = true;
        for i in 0..iterations {
            let stride = 1u32 << i; // 1, 2, 4, 8, 16

            // Update params.
            let p = Params {
                width: self.width,
                height: self.height,
                stride,
                sigma_color_inv,
            };
            queue.write_buffer(&self.params_buf, 0, bytemuck::bytes_of(&p));

            let (in_view, out_view) = if i == 0 {
                // First pass: read from noisy_input_view, write to tex_a.
                (noisy_input_view, view_a)
            } else if output_is_a {
                // Previous output was in tex_b → read tex_b, write tex_a.
                (view_b, view_a)
            } else {
                // Previous output was in tex_a → read tex_a, write tex_b.
                (view_a, view_b)
            };

            let bg = device.create_bind_group(&wgpu::BindGroupDescriptor {
                label: Some("denoiser_atrous_bg"),
                layout: &self.bgl,
                entries: &[
                    wgpu::BindGroupEntry {
                        binding: 0,
                        resource: wgpu::BindingResource::TextureView(in_view),
                    },
                    wgpu::BindGroupEntry {
                        binding: 1,
                        resource: wgpu::BindingResource::TextureView(out_view),
                    },
                    wgpu::BindGroupEntry {
                        binding: 2,
                        resource: self.params_buf.as_entire_binding(),
                    },
                ],
            });

            {
                let mut pass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor {
                    label: Some("denoiser_atrous_pass"),
                    timestamp_writes: None,
                });
                pass.set_pipeline(&self.pipeline);
                pass.set_bind_group(0, &bg, &[]);
                let groups_x = self.width.div_ceil(8);
                let groups_y = self.height.div_ceil(8);
                pass.dispatch_workgroups(groups_x, groups_y, 1);
            }

            output_is_a = !output_is_a;
        }

        // After the loop, output_is_a was flipped one extra time. The
        // ACTUAL last output texture is the OPPOSITE of output_is_a.
        self.last_output_is_a = !output_is_a;
    }

    /// Returns the texture view holding the most-recent denoised output.
    /// Caller's blit pass binds this instead of the raw PT output when
    /// the denoiser is enabled. Returns None if `dispatch` was never
    /// called or `resize` hasn't run.
    pub fn output_view(&self) -> Option<&wgpu::TextureView> {
        if self.last_output_is_a {
            self.view_a.as_ref()
        } else {
            self.view_b.as_ref()
        }
    }
}

fn make_pingpong(
    device: &wgpu::Device,
    label: &str,
    width: u32,
    height: u32,
) -> (wgpu::Texture, wgpu::TextureView) {
    let tex = device.create_texture(&wgpu::TextureDescriptor {
        label: Some(label),
        size: wgpu::Extent3d {
            width,
            height,
            depth_or_array_layers: 1,
        },
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
