//! GPU-accelerated 2D Treemap Renderer using wgpu
//! Renders cushion-shaded treemap tiles as instanced quads

use std::sync::Arc;
use bytemuck::{Pod, Zeroable};
use log::debug;
use wgpu::util::DeviceExt;

use render_core::gpu::{self, GpuContext};
use render_core::Viewport;
use dirstat_core::DirEntry;
use crate::{self as treemap, TreeMapOptions};

/// A single rectangle instance for GPU rendering
#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
pub struct RectInstance {
    /// Rectangle bounds (x, y, w, h) in world pixels
    pub bounds: [f32; 4],
    /// Color (RGB + unused)
    pub color: [f32; 4],
    /// Cushion surface coefficients (a_x, a_y, b_x, b_y)
    pub surface: [f32; 4],
}

impl RectInstance {
    const ATTRIBS: [wgpu::VertexAttribute; 3] = wgpu::vertex_attr_array![
        // bounds at location 1
        1 => Float32x4,
        // color at location 2
        2 => Float32x4,
        // surface at location 3
        3 => Float32x4,
    ];

    pub fn desc() -> wgpu::VertexBufferLayout<'static> {
        wgpu::VertexBufferLayout {
            array_stride: std::mem::size_of::<RectInstance>() as wgpu::BufferAddress,
            step_mode: wgpu::VertexStepMode::Instance,
            attributes: &Self::ATTRIBS,
        }
    }
}

/// Uniform buffer for rendering parameters
#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
pub struct Uniforms {
    /// Viewport size (width, height, 1/width, 1/height)
    pub viewport: [f32; 4],
    /// Pan offset (x, y) + zoom + brightness
    pub pan_zoom_bright: [f32; 4],
    /// Light direction (x, y, z, ambient)
    pub light_dir: [f32; 4],
    /// Grid settings (enabled, r, g, b)
    pub grid_color: [f32; 4],
}

/// Vertex for unit quad (will be transformed per-instance)
#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
pub struct Vertex {
    /// Position in unit quad space (0-1)
    pub position: [f32; 2],
}

impl Vertex {
    const ATTRIBS: [wgpu::VertexAttribute; 1] = wgpu::vertex_attr_array![
        0 => Float32x2,
    ];

    pub fn desc() -> wgpu::VertexBufferLayout<'static> {
        wgpu::VertexBufferLayout {
            array_stride: std::mem::size_of::<Vertex>() as wgpu::BufferAddress,
            step_mode: wgpu::VertexStepMode::Vertex,
            attributes: &Self::ATTRIBS,
        }
    }
}

/// Unit quad vertices (0,0) to (1,1) - two triangles
const UNIT_QUAD: &[Vertex] = &[
    Vertex { position: [0.0, 0.0] },
    Vertex { position: [1.0, 0.0] },
    Vertex { position: [1.0, 1.0] },
    Vertex { position: [0.0, 0.0] },
    Vertex { position: [1.0, 1.0] },
    Vertex { position: [0.0, 1.0] },
];

/// WGSL shader for 2D cushion-shaded treemap with instanced quads
const SHADER_SOURCE: &str = r#"
struct Uniforms {
    viewport: vec4<f32>,        // width, height, 1/width, 1/height
    pan_zoom_bright: vec4<f32>, // pan.x, pan.y, zoom, brightness
    light_dir: vec4<f32>,       // light.x, light.y, light.z, ambient
    grid_color: vec4<f32>,      // grid_width, r, g, b
}

@group(0) @binding(0)
var<uniform> uniforms: Uniforms;

struct VertexInput {
    @location(0) pos: vec2<f32>,        // unit quad position (0-1)
}

struct InstanceInput {
    @location(1) bounds: vec4<f32>,     // x, y, w, h in world space
    @location(2) color: vec4<f32>,      // r, g, b, unused
    @location(3) surface: vec4<f32>,    // cushion coefficients: a_x, a_y, b_x, b_y
}

struct VertexOutput {
    @builtin(position) clip_position: vec4<f32>,
    @location(0) world_pos: vec2<f32>,  // position in world space
    @location(1) color: vec3<f32>,
    @location(2) surface: vec4<f32>,
    @location(3) rect_bounds: vec4<f32>, // for grid line check
}

@vertex
fn vs_main(vertex: VertexInput, instance: InstanceInput) -> VertexOutput {
    let pan = uniforms.pan_zoom_bright.xy;
    let zoom = uniforms.pan_zoom_bright.z;
    
    // Transform unit quad (0-1) to world space rectangle
    let world_x = instance.bounds.x + vertex.pos.x * instance.bounds.z;
    let world_y = instance.bounds.y + vertex.pos.y * instance.bounds.w;
    
    // Transform world space to screen space (apply pan and zoom)
    let screen_x = (world_x - pan.x) * zoom;
    let screen_y = (world_y - pan.y) * zoom;
    
    // Transform screen space to clip space (-1 to 1)
    // Note: Y is flipped because screen Y=0 is top, but clip Y=+1 is top
    let clip_x = screen_x * uniforms.viewport.z * 2.0 - 1.0;
    let clip_y = 1.0 - screen_y * uniforms.viewport.w * 2.0;
    
    var out: VertexOutput;
    out.clip_position = vec4<f32>(clip_x, clip_y, 0.0, 1.0);
    out.world_pos = vec2<f32>(world_x, world_y);
    out.color = instance.color.rgb;
    out.surface = instance.surface;
    out.rect_bounds = instance.bounds;
    return out;
}

@fragment
fn fs_main(in: VertexOutput) -> @location(0) vec4<f32> {
    let brightness = uniforms.pan_zoom_bright.w;
    let ambient = uniforms.light_dir.w;
    let grid_width = uniforms.grid_color.x;
    let grid_col = uniforms.grid_color.yzw;
    
    // Grid line check
    if (grid_width > 0.0) {
        let dx = in.world_pos.x - in.rect_bounds.x;
        let dy = in.world_pos.y - in.rect_bounds.y;
        if (dx < grid_width || dy < grid_width) {
            return vec4<f32>(grid_col, 1.0);
        }
    }
    
    // Compute cushion shading
    let surface = in.surface;
    let px = in.world_pos.x + 0.5;
    let py = in.world_pos.y + 0.5;
    
    // Normal from cushion surface: n = (-dz/dx, -dz/dy, 1)
    // z = a_x * x^2 + a_y * y^2 + b_x * x + b_y * y
    // dz/dx = 2*a_x*x + b_x
    // dz/dy = 2*a_y*y + b_y
    let nx = -(2.0 * surface.x * px + surface.z);
    let ny = -(2.0 * surface.y * py + surface.w);
    let nz = 1.0;
    
    let n_len = sqrt(nx * nx + ny * ny + nz * nz);
    let normal = vec3<f32>(nx / n_len, ny / n_len, nz / n_len);
    
    // Lighting
    let light = normalize(uniforms.light_dir.xyz);
    let ndotl = max(dot(normal, light), 0.0);
    
    let is = 1.0 - ambient;
    let intensity = (is * ndotl + ambient) * brightness;
    
    let final_color = in.color * intensity;
    
    return vec4<f32>(final_color, 1.0);
}
"#;

/// GPU 2D Renderer using instanced quad rendering
pub struct GpuRenderer2D {
    ctx: Arc<GpuContext>,
    pipeline: wgpu::RenderPipeline,
    vertex_buffer: wgpu::Buffer,
    uniform_buffer: wgpu::Buffer,
    bind_group: wgpu::BindGroup,
    render_texture: Option<wgpu::Texture>,
    render_view: Option<wgpu::TextureView>,
    current_size: (u32, u32),
    instance_buffer: Option<wgpu::Buffer>,
    instance_count: u32,
}

impl GpuRenderer2D {
    /// Reset render targets - call when switching modes to avoid corruption
    pub fn reset_render_targets(&mut self) {
        // Wait for any pending GPU work to complete
        let _ = self.ctx.device.poll(wgpu::PollType::wait_indefinitely());
        
        // Clear render targets so they'll be recreated
        self.render_texture = None;
        self.render_view = None;
        self.instance_buffer = None;
        self.instance_count = 0;
        self.current_size = (0, 0);
    }
    
    pub fn new(ctx: Arc<GpuContext>) -> Self {
        let device = &ctx.device;

        // Create shader module
        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("2D Treemap Shader"),
            source: wgpu::ShaderSource::Wgsl(SHADER_SOURCE.into()),
        });

        // Create uniform buffer
        let uniform_buffer = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("2D Uniform Buffer"),
            size: std::mem::size_of::<Uniforms>() as u64,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });

        // Create bind group layout
        let bind_group_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("2D Bind Group Layout"),
            entries: &[
                wgpu::BindGroupLayoutEntry {
                    binding: 0,
                    visibility: wgpu::ShaderStages::VERTEX | wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Uniform,
                        has_dynamic_offset: false,
                        min_binding_size: None,
                    },
                    count: None,
                },
            ],
        });

        // Create bind group
        let bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("2D Bind Group"),
            layout: &bind_group_layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: uniform_buffer.as_entire_binding(),
                },
            ],
        });

        // Create pipeline layout
        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("2D Pipeline Layout"),
            bind_group_layouts: &[Some(&bind_group_layout)],
            immediate_size: 0,
        });

        // Create render pipeline with vertex + instance buffers
        let pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("2D Render Pipeline"),
            layout: Some(&pipeline_layout),
            vertex: wgpu::VertexState {
                module: &shader,
                entry_point: Some("vs_main"),
                buffers: &[Vertex::desc(), RectInstance::desc()],
                compilation_options: Default::default(),
            },
            fragment: Some(wgpu::FragmentState {
                module: &shader,
                entry_point: Some("fs_main"),
                targets: &[Some(wgpu::ColorTargetState {
                    format: wgpu::TextureFormat::Rgba8Unorm,
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
        });

        // Create vertex buffer for unit quad
        let vertex_buffer = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("2D Vertex Buffer"),
            contents: bytemuck::cast_slice(UNIT_QUAD),
            usage: wgpu::BufferUsages::VERTEX,
        });

        Self {
            ctx,
            pipeline,
            vertex_buffer,
            uniform_buffer,
            bind_group,
            render_texture: None,
            render_view: None,
            current_size: (0, 0),
            instance_buffer: None,
            instance_count: 0,
        }
    }

    fn ensure_render_target(&mut self, width: u32, height: u32) {
        if self.current_size == (width, height) && self.render_texture.is_some() {
            return;
        }

        let render_texture = self.ctx.device.create_texture(&wgpu::TextureDescriptor {
            label: Some("2D Render Texture"),
            size: wgpu::Extent3d {
                width,
                height,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::Rgba8Unorm,
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_SRC,
            view_formats: &[],
        });

        let render_view = render_texture.create_view(&wgpu::TextureViewDescriptor::default());

        self.render_texture = Some(render_texture);
        self.render_view = Some(render_view);
        self.current_size = (width, height);
    }

    
    /// Collect rectangle instances from the treemap with consolidation
    fn collect_rects(&self, root: &DirEntry, opts: &TreeMapOptions) -> Vec<RectInstance> {
        let mut rects = Vec::new();
        let grid_w = if opts.grid { 1.0 } else { 0.0 };
        self.collect_rects_recursive(root, opts, grid_w, [0.0; 4], opts.height, true, 0, &mut rects);
        
        debug!("Collected {} rectangles after consolidation", rects.len());
        rects
    }

    #[allow(clippy::too_many_arguments)]
    fn collect_rects_recursive(
        &self,
        node: &DirEntry,
        opts: &TreeMapOptions,
        grid_w: f32,
        surface: [f32; 4],
        h: f64,
        is_root: bool,
        dir_hash: u32,
        rects: &mut Vec<RectInstance>,
    ) {
        let [x, y, w, h_px] = node.rect.get();
        if w <= 0.0 || h_px <= 0.0 {
            return;
        }

        let cushion = opts.ambient_light < 1.0 && opts.height > 0.0 && opts.scale_factor > 0.0;

        // Add ridge for cushion (not for root)
        let surface = if cushion && !is_root {
            treemap::add_ridge_f32(x, y, w, h_px, surface, h)
        } else {
            surface
        };

        // Check if this node is too small to recurse into
        let too_small = w < treemap::MIN_RECT_SIZE || h_px < treemap::MIN_RECT_SIZE;
        
        if !node.is_dir || node.children.is_empty() || too_small {
            // Leaf node OR consolidated small directory
            let color = if node.is_dir && !node.children.is_empty() {
                treemap::compute_avg_color(node, dir_hash)
            } else {
                treemap::dir_tinted_color(&node.ext, dir_hash)
            };
            
            let color_f = [
                color[0] as f32 / 255.0,
                color[1] as f32 / 255.0,
                color[2] as f32 / 255.0,
                1.0,
            ];

            rects.push(RectInstance {
                bounds: [x + grid_w, y + grid_w, w - grid_w, h_px - grid_w],
                color: color_f,
                surface,
            });
        } else {
            // Directory large enough to show children: recurse
            let my_hash = treemap::path_hash(&node.name, dir_hash);
            let next_h = h * opts.scale_factor;
            for child in &node.children {
                self.collect_rects_recursive(child, opts, grid_w, surface, next_h, false, my_hash, rects);
            }
        }
    }
    
    /// Render the 2D treemap to a pixel buffer
    pub fn render(
        &mut self,
        root: &DirEntry,
        viewport: &Viewport,
        opts: &TreeMapOptions,
    ) -> Vec<u8> {
        let width = viewport.width;
        let height = viewport.height;

        if width == 0 || height == 0 {
            return vec![];
        }

        // Ensure any previous GPU work is complete
        let _ = self.ctx.device.poll(wgpu::PollType::wait_indefinitely());

        // Layout treemap
        let world_w = width as f32 / viewport.zoom;
        let world_h = height as f32 / viewport.zoom;
        treemap::layout(root, -viewport.pan[0], -viewport.pan[1], world_w, world_h, opts);

        // Ensure render target
        self.ensure_render_target(width, height);

        // Collect rectangles (with consolidation for small areas)
        let rects = self.collect_rects(root, opts);
        self.instance_count = rects.len() as u32;

        if rects.is_empty() {
            return vec![30; (width * height * 4) as usize];
        }

        // Log rectangle count and buffer size for debugging
        let bytes_per_rect = std::mem::size_of::<RectInstance>();
        let needed_bytes = rects.len() * bytes_per_rect;
        debug!(
            "GPU 2D render: {} rects, buffer size: {} bytes ({:.2} MB), viewport: {}x{}",
            rects.len(),
            needed_bytes,
            needed_bytes as f64 / (1024.0 * 1024.0),
            width,
            height
        );

        let needed_u64 = needed_bytes as u64;
        let reuse = self.instance_buffer.as_ref().map_or(false, |b| b.size() >= needed_u64);
        if reuse {
            let buf = self.instance_buffer.as_ref().unwrap();
            self.ctx
                .queue
                .write_buffer(buf, 0, bytemuck::cast_slice(&rects));
        } else {
            let min_bytes = (256 * bytes_per_rect) as u64;
            let new_size = needed_u64.max(min_bytes).next_power_of_two();
            let instance_buffer = self.ctx.device.create_buffer(&wgpu::BufferDescriptor {
                label: Some("2D Instance Buffer"),
                size: new_size,
                usage: wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::COPY_DST,
                mapped_at_creation: false,
            });
            self.ctx
                .queue
                .write_buffer(&instance_buffer, 0, bytemuck::cast_slice(&rects));
            self.instance_buffer = Some(instance_buffer);
        }

        // Update uniforms
        let light_len = (opts.light_x * opts.light_x + opts.light_y * opts.light_y + 100.0).sqrt();
        let uniforms = Uniforms {
            viewport: [width as f32, height as f32, 1.0 / width as f32, 1.0 / height as f32],
            pan_zoom_bright: [
                viewport.pan[0],
                viewport.pan[1],
                viewport.zoom,
                // Match CPU: brightness / PALETTE_BRIGHTNESS (0.6)
                (opts.brightness / 0.6) as f32,
            ],
            light_dir: [
                (opts.light_x / light_len) as f32,
                (opts.light_y / light_len) as f32,
                (10.0 / light_len) as f32,
                opts.ambient_light as f32,
            ],
            grid_color: [
                if opts.grid { 1.0 } else { 0.0 },
                opts.grid_color[0] as f32 / 255.0,
                opts.grid_color[1] as f32 / 255.0,
                opts.grid_color[2] as f32 / 255.0,
            ],
        };

        self.ctx.queue.write_buffer(&self.uniform_buffer, 0, bytemuck::bytes_of(&uniforms));

        // Create command encoder
        let mut encoder = self.ctx.device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
            label: Some("2D Render Encoder"),
        });

        // Render pass
        {
            let render_view = self.render_view.as_ref().unwrap();

            let mut render_pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("2D Render Pass"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: render_view,
                    resolve_target: None,
                    depth_slice: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Clear(wgpu::Color {
                            r: opts.grid_color[0] as f64 / 255.0,
                            g: opts.grid_color[1] as f64 / 255.0,
                            b: opts.grid_color[2] as f64 / 255.0,
                            a: 1.0,
                        }),
                        store: wgpu::StoreOp::Store,
                    },
                })],
                depth_stencil_attachment: None,
                timestamp_writes: None,
                occlusion_query_set: None,
                multiview_mask: None,
            });

            render_pass.set_pipeline(&self.pipeline);
            render_pass.set_bind_group(0, &self.bind_group, &[]);
            render_pass.set_vertex_buffer(0, self.vertex_buffer.slice(..));
            render_pass.set_vertex_buffer(1, self.instance_buffer.as_ref().unwrap().slice(..));
            // Draw 6 vertices (unit quad) for each instance
            render_pass.draw(0..6, 0..self.instance_count);
        }

        // Copy render texture to readback buffer
        let output_buffer = gpu::readback_texture(
            &self.ctx, &mut encoder, self.render_texture.as_ref().unwrap(), width, height,
        );

        // Submit and wait
        self.ctx.queue.submit(std::iter::once(encoder.finish()));

        // Map and extract pixels
        gpu::map_readback(&self.ctx, &output_buffer, width, height)
    }
}
