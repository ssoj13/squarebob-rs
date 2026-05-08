use std::sync::Arc;

/// Viewport state for pan/zoom
#[derive(Debug, Clone)]
pub struct Viewport {
    /// Pan offset in world coordinates
    pub pan: [f32; 2],
    /// Zoom level (1.0 = 100%, 2.0 = 200%, etc.)
    pub zoom: f32,
    /// Target zoom for smooth animation
    pub zoom_target: f32,
    /// Screen size
    pub width: u32,
    pub height: u32,
}

impl Default for Viewport {
    fn default() -> Self {
        Self {
            pan: [0.0, 0.0],
            zoom: 1.0,
            zoom_target: 1.0,
            width: 800,
            height: 600,
        }
    }
}

#[allow(dead_code)]
impl Viewport {
    /// Convert screen coordinates to world coordinates
    pub fn screen_to_world(&self, screen_x: f32, screen_y: f32) -> (f32, f32) {
        let world_x = screen_x / self.zoom + self.pan[0];
        let world_y = screen_y / self.zoom + self.pan[1];
        (world_x, world_y)
    }

    /// Convert world coordinates to screen coordinates
    pub fn world_to_screen(&self, world_x: f32, world_y: f32) -> (f32, f32) {
        let screen_x = (world_x - self.pan[0]) * self.zoom;
        let screen_y = (world_y - self.pan[1]) * self.zoom;
        (screen_x, screen_y)
    }

    /// Zoom toward a screen point
    pub fn zoom_toward(&mut self, screen_x: f32, screen_y: f32, factor: f32) {
        let (world_x, world_y) = self.screen_to_world(screen_x, screen_y);

        self.zoom_target = (self.zoom_target * factor).clamp(0.1, 100.0);

        // Adjust pan to keep the point under cursor
        let new_zoom = self.zoom_target;
        self.pan[0] = world_x - screen_x / new_zoom;
        self.pan[1] = world_y - screen_y / new_zoom;
    }

    /// Animate zoom smoothly
    pub fn update(&mut self, dt: f32) {
        let speed = 10.0 * dt;
        self.zoom = self.zoom + (self.zoom_target - self.zoom) * speed.min(1.0);
    }

    /// Reset to default view
    pub fn reset(&mut self) {
        self.pan = [0.0, 0.0];
        self.zoom = 1.0;
        self.zoom_target = 1.0;
    }
}

/// Shared GPU context for wgpu-based rendering
pub mod gpu {
    use super::*;

    /// Shared GPU context - holds wgpu device and queue
    pub struct GpuContext {
        pub device: Arc<wgpu::Device>,
        pub queue: Arc<wgpu::Queue>,
    }

    impl GpuContext {
        /// Create GpuContext from existing device/queue (for eframe integration)
        pub fn from_eframe(device: Arc<wgpu::Device>, queue: Arc<wgpu::Queue>) -> Self {
            Self { device, queue }
        }

        /// Create a new standalone GPU context
        pub fn new() -> Option<Self> {
            let mut inst_desc = wgpu::InstanceDescriptor::new_without_display_handle();
            inst_desc.backends = wgpu::Backends::all();
            let instance = wgpu::Instance::new(inst_desc);

            let adapter =
                pollster::block_on(instance.request_adapter(&wgpu::RequestAdapterOptions {
                    power_preference: wgpu::PowerPreference::HighPerformance,
                    compatible_surface: None,
                    force_fallback_adapter: false,
                }))
                .ok()?;

            let (device, queue) =
                pollster::block_on(adapter.request_device(&wgpu::DeviceDescriptor {
                    label: Some("DirStat GPU Device"),
                    required_features: wgpu::Features::POLYGON_MODE_LINE,
                    required_limits: wgpu::Limits::default(),
                    memory_hints: Default::default(),
                    trace: Default::default(),
                    experimental_features: Default::default(),
                }))
                .ok()?;

            Some(Self {
                device: Arc::new(device),
                queue: Arc::new(queue),
            })
        }
    }

    /// Copy a GPU texture to encoder and submit, then read back pixels as Vec<u8>.
    /// Handles row alignment (256-byte), buffer mapping, and padding removal.
    pub fn readback_texture(
        ctx: &GpuContext,
        encoder: &mut wgpu::CommandEncoder,
        texture: &wgpu::Texture,
        width: u32,
        height: u32,
    ) -> wgpu::Buffer {
        let bytes_per_row = 4 * width;
        let padded_bytes_per_row = (bytes_per_row + 255) & !255;

        let output_buffer = ctx.device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("Readback Buffer"),
            size: (padded_bytes_per_row * height) as u64,
            usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
            mapped_at_creation: false,
        });

        encoder.copy_texture_to_buffer(
            wgpu::TexelCopyTextureInfo {
                texture,
                mip_level: 0,
                origin: wgpu::Origin3d::ZERO,
                aspect: wgpu::TextureAspect::All,
            },
            wgpu::TexelCopyBufferInfo {
                buffer: &output_buffer,
                layout: wgpu::TexelCopyBufferLayout {
                    offset: 0,
                    bytes_per_row: Some(padded_bytes_per_row),
                    rows_per_image: Some(height),
                },
            },
            wgpu::Extent3d {
                width,
                height,
                depth_or_array_layers: 1,
            },
        );

        output_buffer
    }

    /// Map a readback buffer and extract pixels, removing row padding.
    pub fn map_readback(
        ctx: &GpuContext,
        buffer: &wgpu::Buffer,
        width: u32,
        height: u32,
    ) -> Vec<u8> {
        let bytes_per_row = 4 * width;
        let padded_bytes_per_row = (bytes_per_row + 255) & !255;

        let buffer_slice = buffer.slice(..);
        let (tx, rx) = std::sync::mpsc::channel();
        buffer_slice.map_async(wgpu::MapMode::Read, move |result| {
            tx.send(result).unwrap();
        });
        // Must wait for map_async callback before rx.recv()
        let _ = ctx.device.poll(wgpu::PollType::wait_indefinitely());
        rx.recv().unwrap().unwrap();

        let data = buffer_slice.get_mapped_range();
        let mut pixels = Vec::with_capacity((width * height * 4) as usize);
        for row in 0..height {
            let start = (row * padded_bytes_per_row) as usize;
            let end = start + (width * 4) as usize;
            pixels.extend_from_slice(&data[start..end]);
        }
        drop(data);
        buffer.unmap();
        pixels
    }
}
