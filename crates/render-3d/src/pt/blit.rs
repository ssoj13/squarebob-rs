//! Path-tracer display blit pipeline, OCIO shader integration, and CPU display-color scratch.
//!
//! Decouples `squarebob-rs`'s viewport display transform (physical exposure, built-in tonemapping,
//! ACES matrices, and GPU/CPU OCIO canvas adaptation) from `render-rs`'s `pt-megakernel` compute
//! engine so both raw PT output and denoised OIDN output share one display blit pass.

use std::sync::Arc;

const BLIT_WGSL: &str = include_str!("../../shaders/pt_blit.wgsl");
const NO_OCIO_WGSL: &str =
    "fn ocio_transform(c: vec4<f32>) -> vec4<f32> { return vec4<f32>(display_encode(c.rgb), c.a); }\n";
const BLIT_PARAMS_BYTES: u64 = 128;

/// Fullscreen display blit and optional CPU color-processor scratch for path-traced frames.
pub struct PtDisplayBlit {
    width: u32,
    height: u32,
    format: wgpu::TextureFormat,
    blit_pipeline: wgpu::RenderPipeline,
    blit_bind_group_layout: wgpu::BindGroupLayout,
    blit_sampler: wgpu::Sampler,
    blit_uniform_buffer: wgpu::Buffer,
    display_ocio: Option<Arc<render_color_gpu::OcioShader>>,
    cpu_color_readback: render_core::gpu::TextureReadback,
    cpu_color_output: Option<wgpu::Texture>,
}

impl PtDisplayBlit {
    /// Create a new display blit targeting `format` for a `width x height` viewport.
    pub fn new(
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        width: u32,
        height: u32,
        format: wgpu::TextureFormat,
    ) -> Self {
        let blit_bind_group_layout =
            device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
                label: Some("pt_blit_bgl"),
                entries: &[
                    wgpu::BindGroupLayoutEntry {
                        binding: 0,
                        visibility: wgpu::ShaderStages::FRAGMENT,
                        ty: wgpu::BindingType::Texture {
                            sample_type: wgpu::TextureSampleType::Float { filterable: false },
                            view_dimension: wgpu::TextureViewDimension::D2,
                            multisampled: false,
                        },
                        count: None,
                    },
                    wgpu::BindGroupLayoutEntry {
                        binding: 1,
                        visibility: wgpu::ShaderStages::FRAGMENT,
                        ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::NonFiltering),
                        count: None,
                    },
                    wgpu::BindGroupLayoutEntry {
                        binding: 2,
                        visibility: wgpu::ShaderStages::FRAGMENT,
                        ty: wgpu::BindingType::Buffer {
                            ty: wgpu::BufferBindingType::Uniform,
                            has_dynamic_offset: false,
                            min_binding_size: None,
                        },
                        count: None,
                    },
                ],
            });

        let blit_sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("pt_blit_sampler"),
            mag_filter: wgpu::FilterMode::Nearest,
            min_filter: wgpu::FilterMode::Nearest,
            ..Default::default()
        });

        let blit_uniform_buffer = render_core::gpu::make_buffer(
            device,
            "pt_blit_uniform_buffer",
            BLIT_PARAMS_BYTES,
            wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
        );
        let init_exposure: [f32; 4] = [1.0, 0.0, 0.0, 0.0];
        let init_color: [f32; 4] = [3.0, 0.0, 1.0, 0.0];
        let identity_mat3: [[f32; 4]; 3] = [
            [1.0, 0.0, 0.0, 0.0],
            [0.0, 1.0, 0.0, 0.0],
            [0.0, 0.0, 1.0, 0.0],
        ];
        queue.write_buffer(
            &blit_uniform_buffer,
            0,
            bytemuck::cast_slice(&init_exposure),
        );
        queue.write_buffer(&blit_uniform_buffer, 16, bytemuck::cast_slice(&init_color));
        queue.write_buffer(
            &blit_uniform_buffer,
            32,
            bytemuck::cast_slice(&identity_mat3),
        );
        queue.write_buffer(
            &blit_uniform_buffer,
            80,
            bytemuck::cast_slice(&identity_mat3),
        );

        let blit_pipeline =
            make_blit_pipeline(device, &blit_bind_group_layout, format, None);

        Self {
            width,
            height,
            format,
            blit_pipeline,
            blit_bind_group_layout,
            blit_sampler,
            blit_uniform_buffer,
            display_ocio: None,
            cpu_color_readback: render_core::gpu::TextureReadback::default(),
            cpu_color_output: None,
        }
    }

    /// Update the expected source dimensions for CPU display-color readback.
    pub fn resize(&mut self, width: u32, height: u32) {
        self.width = width;
        self.height = height;
    }

    /// Push the scene-linear exposure multiplier into `exposure.x` (offset 0).
    pub fn set_blit_exposure(&self, queue: &wgpu::Queue, exposure: f32) {
        let params: [f32; 1] = [exposure];
        queue.write_buffer(&self.blit_uniform_buffer, 0, bytemuck::cast_slice(&params));
    }

    /// Push the legacy ACES ODT tag into `exposure.y` (offset 4).
    pub fn set_blit_odt_tag(&self, queue: &wgpu::Queue, odt_tag: u32) {
        let params: [f32; 1] = [odt_tag as f32];
        queue.write_buffer(&self.blit_uniform_buffer, 4, bytemuck::cast_slice(&params));
    }

    /// Push the ACES RRT tag into `exposure.z` (offset 8).
    pub fn set_blit_rrt_tag(&self, queue: &wgpu::Queue, rrt_tag: u32) {
        let params: [f32; 1] = [rrt_tag as f32];
        queue.write_buffer(&self.blit_uniform_buffer, 8, bytemuck::cast_slice(&params));
    }

    /// Mark whether the input texture already contains canvas-encoded values (`exposure.w`, offset 12).
    pub fn set_blit_source_encoded(&self, queue: &wgpu::Queue, encoded: bool) {
        let params: [f32; 1] = [if encoded { 1.0 } else { 0.0 }];
        queue.write_buffer(&self.blit_uniform_buffer, 12, bytemuck::cast_slice(&params));
    }

    /// Push the colour-pipeline lane (`color`, offset 16).
    pub fn set_blit_color(
        &self,
        queue: &wgpu::Queue,
        tonemap_tag: u32,
        display_exposure_ev: f32,
        white_balance_norm: f32,
        gamut_compress: f32,
    ) {
        let params: [f32; 4] = [
            tonemap_tag as f32,
            display_exposure_ev,
            white_balance_norm,
            gamut_compress,
        ];
        queue.write_buffer(&self.blit_uniform_buffer, 16, bytemuck::cast_slice(&params));
    }

    /// Upload the pre-RRT (IDT∘LMT) and post-RRT (ODT) 3×3 matrices in std140 layout (offsets 32 and 80).
    pub fn set_blit_aces_matrices(
        &self,
        queue: &wgpu::Queue,
        pre_cols: &[[f32; 4]; 3],
        post_cols: &[[f32; 4]; 3],
    ) {
        queue.write_buffer(&self.blit_uniform_buffer, 32, bytemuck::cast_slice(pre_cols));
        queue.write_buffer(
            &self.blit_uniform_buffer,
            80,
            bytemuck::cast_slice(post_cols),
        );
    }

    /// Install or clear the compiled GPU OCIO shader and rebuild the blit render pipeline.
    pub fn set_display_transform(
        &mut self,
        device: &wgpu::Device,
        ocio: Option<Arc<render_color_gpu::OcioShader>>,
    ) {
        self.blit_pipeline = make_blit_pipeline(
            device,
            &self.blit_bind_group_layout,
            self.format,
            ocio.as_deref(),
        );
        self.display_ocio = ocio;
    }

    /// Read a raw `Rgba32Float` PT or denoiser source, transform RGB in host staging in place,
    /// and upload to a separate `Rgba32Float` display-light scratch texture. Preserves raw sources.
    pub fn apply_cpu_color_in_place(
        &mut self,
        ctx: &render_core::gpu::GpuContext,
        source: &wgpu::Texture,
        apply: impl FnOnce(&mut [[f32; 3]]),
    ) -> Result<wgpu::TextureView, render_core::ReadbackError> {
        const BYTES_PER_PIXEL: u32 = 16;
        const LABEL: &str = "PT CPU color readback";
        if source.format() != wgpu::TextureFormat::Rgba32Float {
            return Err(render_core::ReadbackError::UnsupportedFormat(
                source.format(),
            ));
        }
        for (expected, actual) in [(self.width, source.width()), (self.height, source.height())] {
            if expected != actual {
                return Err(render_core::GpuLayoutError::LengthMismatch {
                    context: LABEL,
                    left: expected as usize,
                    right: actual as usize,
                }
                .into());
            }
        }

        let layout = render_core::gpu::TextureReadbackLayout::new(
            LABEL,
            self.width,
            self.height,
            BYTES_PER_PIXEL,
        )?;
        let texel_count = render_core::checked_2d_buffer_len(LABEL, self.width, self.height)?;

        let mut encoder = ctx
            .device
            .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                label: Some("pt_cpu_color_copy"),
            });
        render_core::gpu::readback_texture_bytes(
            ctx,
            &mut encoder,
            source,
            layout,
            &mut self.cpu_color_readback,
        )?;
        ctx.queue.submit(std::iter::once(encoder.finish()));

        let raw = render_core::gpu::map_readback(ctx, &self.cpu_color_readback)?;
        let mut pixels = Vec::new();
        pixels.try_reserve_exact(texel_count)?;
        let mut alphas = Vec::new();
        alphas.try_reserve_exact(texel_count)?;
        for texel in raw.chunks_exact(BYTES_PER_PIXEL as usize) {
            let channel = |offset: usize| {
                f32::from_ne_bytes([
                    texel[offset],
                    texel[offset + 1],
                    texel[offset + 2],
                    texel[offset + 3],
                ])
            };
            pixels.push([channel(0), channel(4), channel(8)]);
            alphas.push(channel(12));
        }

        apply(&mut pixels);

        let mut packed = Vec::new();
        packed.try_reserve_exact(texel_count)?;
        for (pixel, alpha) in pixels.into_iter().zip(alphas) {
            packed.push([pixel[0], pixel[1], pixel[2], alpha]);
        }
        if self
            .cpu_color_output
            .as_ref()
            .is_none_or(|texture| texture.width() != self.width || texture.height() != self.height)
        {
            self.cpu_color_output = Some(ctx.device.create_texture(&wgpu::TextureDescriptor {
                label: Some("PT CPU display-light scratch"),
                size: wgpu::Extent3d {
                    width: self.width,
                    height: self.height,
                    depth_or_array_layers: 1,
                },
                mip_level_count: 1,
                sample_count: 1,
                dimension: wgpu::TextureDimension::D2,
                format: wgpu::TextureFormat::Rgba32Float,
                usage: wgpu::TextureUsages::TEXTURE_BINDING
                    | wgpu::TextureUsages::COPY_DST
                    | wgpu::TextureUsages::COPY_SRC,
                view_formats: &[],
            }));
        }
        let output = self
            .cpu_color_output
            .as_ref()
            .ok_or(render_core::ReadbackError::MissingTarget)?;
        ctx.queue.write_texture(
            wgpu::TexelCopyTextureInfo {
                texture: output,
                mip_level: 0,
                origin: wgpu::Origin3d::ZERO,
                aspect: wgpu::TextureAspect::All,
            },
            bytemuck::cast_slice(&packed),
            wgpu::TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(layout.row_bytes()),
                rows_per_image: Some(self.height),
            },
            wgpu::Extent3d {
                width: self.width,
                height: self.height,
                depth_or_array_layers: 1,
            },
        );
        Ok(output.create_view(&Default::default()))
    }

    /// Encode the fullscreen display blit from `source` (`Rgba32Float`) into `target`.
    pub fn blit_texture(
        &self,
        device: &wgpu::Device,
        encoder: &mut wgpu::CommandEncoder,
        target: &wgpu::TextureView,
        source: &wgpu::TextureView,
    ) {
        let bg = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("pt_blit_bg"),
            layout: &self.blit_bind_group_layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: wgpu::BindingResource::TextureView(source),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: wgpu::BindingResource::Sampler(&self.blit_sampler),
                },
                wgpu::BindGroupEntry {
                    binding: 2,
                    resource: self.blit_uniform_buffer.as_entire_binding(),
                },
            ],
        });
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
            multiview_mask: None,
        });
        pass.set_pipeline(&self.blit_pipeline);
        pass.set_bind_group(0, &bg, &[]);
        if let Some(ocio) = &self.display_ocio {
            for (group, _, ocio_bg) in ocio.groups() {
                pass.set_bind_group(group, ocio_bg, &[]);
            }
        }
        pass.draw(0..3, 0..1);
    }
}

fn make_blit_pipeline(
    device: &wgpu::Device,
    blit_bind_group_layout: &wgpu::BindGroupLayout,
    format: wgpu::TextureFormat,
    ocio: Option<&render_color_gpu::OcioShader>,
) -> wgpu::RenderPipeline {
    let ocio_wgsl = ocio.map_or(NO_OCIO_WGSL, |shader| shader.wgsl());
    let source = format!(
        "{}\n{}{}",
        render_core::DISPLAY_TRANSFER_WGSL,
        ocio_wgsl,
        BLIT_WGSL
    );
    let module = device.create_shader_module(wgpu::ShaderModuleDescriptor {
        label: Some("pt_blit_shader"),
        source: wgpu::ShaderSource::Wgsl(source.into()),
    });
    let [g1, g2] = ocio.map_or([None, None], |shader| shader.layouts());
    let layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
        label: Some("pt_blit_pipeline_layout"),
        bind_group_layouts: &[Some(blit_bind_group_layout), g1, g2],
        immediate_size: 0,
    });
    device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
        label: Some("pt_blit_pipeline"),
        layout: Some(&layout),
        vertex: wgpu::VertexState {
            module: &module,
            entry_point: Some("vs_main"),
            buffers: &[],
            compilation_options: Default::default(),
        },
        fragment: Some(wgpu::FragmentState {
            module: &module,
            entry_point: Some("fs_main"),
            targets: &[Some(wgpu::ColorTargetState {
                format,
                blend: Some(wgpu::BlendState::REPLACE),
                write_mask: wgpu::ColorWrites::ALL,
            })],
            compilation_options: Default::default(),
        }),
        primitive: wgpu::PrimitiveState {
            topology: wgpu::PrimitiveTopology::TriangleList,
            ..Default::default()
        },
        depth_stencil: None,
        multisample: wgpu::MultisampleState::default(),
        multiview_mask: None,
        cache: None,
    })
}
