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

/// Checked-size failure shared by GPU buffers, textures, and CPU staging layouts.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum GpuLayoutError {
    ZeroExtent {
        context: &'static str,
        width: u32,
        height: u32,
    },
    ZeroBytesPerElement {
        context: &'static str,
    },
    Overflow {
        context: &'static str,
        width: u32,
        height: u32,
        bytes_per_element: u64,
    },
    ValueTooLarge {
        context: &'static str,
        value: u64,
        target: &'static str,
    },
    InvalidAlignment {
        context: &'static str,
        alignment: u32,
    },
    LimitExceeded {
        context: &'static str,
        value: u64,
        limit: u64,
        limit_name: &'static str,
    },
    HostAllocation {
        context: &'static str,
        elements: usize,
    },
    LengthMismatch {
        context: &'static str,
        left: usize,
        right: usize,
    },
}

impl std::fmt::Display for GpuLayoutError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::ZeroExtent {
                context,
                width,
                height,
            } => write!(f, "{context}: zero extent {width}x{height}"),
            Self::ZeroBytesPerElement { context } => {
                write!(f, "{context}: bytes per element must be non-zero")
            }
            Self::Overflow {
                context,
                width,
                height,
                bytes_per_element,
            } => write!(
                f,
                "{context}: {width}x{height} at {bytes_per_element} bytes per element overflows"
            ),
            Self::ValueTooLarge {
                context,
                value,
                target,
            } => write!(f, "{context}: {value} does not fit {target}"),
            Self::InvalidAlignment { context, alignment } => {
                write!(
                    f,
                    "{context}: alignment {alignment} is not a non-zero power of two"
                )
            }
            Self::LimitExceeded {
                context,
                value,
                limit,
                limit_name,
            } => write!(
                f,
                "{context}: {value} exceeds device {limit_name} limit {limit}"
            ),
            Self::HostAllocation { context, elements } => {
                write!(f, "{context}: failed to allocate {elements} host elements")
            }
            Self::LengthMismatch {
                context,
                left,
                right,
            } => write!(f, "{context}: length mismatch ({left} != {right})"),
        }
    }
}

impl std::error::Error for GpuLayoutError {}

/// Checked element count for a non-empty two-dimensional allocation.
pub fn checked_2d_element_count(
    context: &'static str,
    width: u32,
    height: u32,
) -> Result<u64, GpuLayoutError> {
    if width == 0 || height == 0 {
        return Err(GpuLayoutError::ZeroExtent {
            context,
            width,
            height,
        });
    }
    u64::from(width)
        .checked_mul(u64::from(height))
        .ok_or(GpuLayoutError::Overflow {
            context,
            width,
            height,
            bytes_per_element: 1,
        })
}

/// Checked byte size for a non-empty two-dimensional allocation.
pub fn checked_2d_buffer_size(
    context: &'static str,
    width: u32,
    height: u32,
    bytes_per_element: u64,
) -> Result<u64, GpuLayoutError> {
    if bytes_per_element == 0 {
        return Err(GpuLayoutError::ZeroBytesPerElement { context });
    }
    checked_2d_element_count(context, width, height)?
        .checked_mul(bytes_per_element)
        .ok_or(GpuLayoutError::Overflow {
            context,
            width,
            height,
            bytes_per_element,
        })
}

/// Check a storage-buffer byte size against device limits.
pub fn checked_storage_buffer_size(
    device: &wgpu::Device,
    context: &'static str,
    size: u64,
) -> Result<u64, GpuLayoutError> {
    let limits = device.limits();
    let max_buffer_size = limits.max_buffer_size;
    if size > max_buffer_size {
        return Err(GpuLayoutError::LimitExceeded {
            context,
            value: size,
            limit: max_buffer_size,
            limit_name: "max_buffer_size",
        });
    }
    let max_binding_size = u64::from(limits.max_storage_buffer_binding_size);
    if size > max_binding_size {
        return Err(GpuLayoutError::LimitExceeded {
            context,
            value: size,
            limit: max_binding_size,
            limit_name: "max_storage_buffer_binding_size",
        });
    }
    Ok(size)
}

/// Checked storage-buffer byte size against arithmetic and device limits.
pub fn checked_2d_storage_buffer_size(
    device: &wgpu::Device,
    context: &'static str,
    width: u32,
    height: u32,
    bytes_per_element: u64,
) -> Result<u64, GpuLayoutError> {
    let size = checked_2d_buffer_size(context, width, height, bytes_per_element)?;
    checked_storage_buffer_size(device, context, size)
}

/// Validate a non-empty 2D texture extent against the device limit.
pub fn checked_texture_extent_2d(
    device: &wgpu::Device,
    context: &'static str,
    width: u32,
    height: u32,
) -> Result<(), GpuLayoutError> {
    checked_2d_element_count(context, width, height)?;
    let limit = u64::from(device.limits().max_texture_dimension_2d);
    let largest = u64::from(width.max(height));
    if largest > limit {
        return Err(GpuLayoutError::LimitExceeded {
            context,
            value: largest,
            limit,
            limit_name: "max_texture_dimension_2d",
        });
    }
    Ok(())
}

/// Checked host-vector length in elements for a non-empty 2D allocation.
pub fn checked_2d_buffer_len(
    context: &'static str,
    width: u32,
    height: u32,
) -> Result<usize, GpuLayoutError> {
    let count = checked_2d_element_count(context, width, height)?;
    usize::try_from(count).map_err(|_| GpuLayoutError::ValueTooLarge {
        context,
        value: count,
        target: "usize",
    })
}

/// Checked host-vector byte length for a non-empty 2D allocation.
pub fn checked_2d_byte_len(
    context: &'static str,
    width: u32,
    height: u32,
    bytes_per_element: u64,
) -> Result<usize, GpuLayoutError> {
    let bytes = checked_2d_buffer_size(context, width, height, bytes_per_element)?;
    usize::try_from(bytes).map_err(|_| GpuLayoutError::ValueTooLarge {
        context,
        value: bytes,
        target: "usize",
    })
}

/// Allocate and initialize a host vector without aborting on capacity failure.
pub fn try_vec_filled<T: Clone>(
    context: &'static str,
    len: usize,
    value: T,
) -> Result<Vec<T>, GpuLayoutError> {
    let mut values = Vec::new();
    values
        .try_reserve_exact(len)
        .map_err(|_| GpuLayoutError::HostAllocation {
            context,
            elements: len,
        })?;
    values.resize(len, value);
    Ok(values)
}

/// Checked aligned row pitch. Alignment must be a non-zero power of two.
pub fn checked_aligned_bytes_per_row(
    context: &'static str,
    width: u32,
    bytes_per_pixel: u32,
    alignment: u32,
) -> Result<u32, GpuLayoutError> {
    if !alignment.is_power_of_two() {
        return Err(GpuLayoutError::InvalidAlignment { context, alignment });
    }
    if bytes_per_pixel == 0 {
        return Err(GpuLayoutError::ZeroBytesPerElement { context });
    }
    let unaligned = u64::from(width)
        .checked_mul(u64::from(bytes_per_pixel))
        .ok_or(GpuLayoutError::Overflow {
            context,
            width,
            height: 1,
            bytes_per_element: u64::from(bytes_per_pixel),
        })?;
    let mask = u64::from(alignment - 1);
    let aligned = unaligned
        .checked_add(mask)
        .map(|value| value & !mask)
        .ok_or(GpuLayoutError::Overflow {
            context,
            width,
            height: 1,
            bytes_per_element: u64::from(bytes_per_pixel),
        })?;
    u32::try_from(aligned).map_err(|_| GpuLayoutError::ValueTooLarge {
        context,
        value: aligned,
        target: "u32",
    })
}

/// Shared GPU context for wgpu-based rendering.
///
/// Owns the full wgpu setup quartet (`Instance` / `Adapter` / `Device` / `Queue`)
/// behind `Arc`. We hold all four because:
///
/// * `Instance` + `Adapter` are required by `cubecl_wgpu::WgpuSetup` when sharing
///   the device with Burn-wgpu for the OIDN denoiser (see `pt-denoise-oidn`).
/// * `eframe` is initialised with `WgpuSetup::Existing` from these handles so
///   the GUI renders on the *same* device as PT / treemap / OIDN — no parallel
///   adapters, no readback to bridge between subsystems.
///
/// There is exactly **one** way to construct this: [`GpuContext::new`]. The
/// `from_eframe` path was removed; eframe is now a consumer of our setup, not
/// its source.
pub mod gpu {
    use super::*;

    pub struct GpuContext {
        pub instance: Arc<wgpu::Instance>,
        pub adapter: Arc<wgpu::Adapter>,
        pub device: Arc<wgpu::Device>,
        pub queue: Arc<wgpu::Queue>,
        /// Best-effort VRAM intel queried at init via `gpu-mem`. `None`
        /// when no platform method (nvidia-smi / registry / sysfs /
        /// system_profiler) could determine it. Consumers should treat
        /// missing data as "limits unknown — use conservative defaults".
        pub gpu_info: Option<gpu_mem::GpuMemInfo>,
    }

    impl GpuContext {
        /// Build the wgpu setup with the limits/features squarebob needs.
        ///
        /// Returns `None` (with a logged reason) if adapter/device acquisition
        /// fails. Callers should treat this as a hard failure — there is no
        /// CPU-only fallback path for the renderer.
        ///
        /// Limits enforced here:
        /// * `max_storage_buffers_per_shader_stage`: bumped to 16 (default 8)
        ///   for the megakernel's ReSTIR + path-guide + denoise bindings.
        /// * `Features::POLYGON_MODE_LINE` for wireframe rendering.
        pub fn new() -> Option<Self> {
            let mut inst_desc = wgpu::InstanceDescriptor::new_without_display_handle();
            inst_desc.backends = wgpu::Backends::all();
            let instance = wgpu::Instance::new(inst_desc);

            let adapter =
                match pollster::block_on(instance.request_adapter(&wgpu::RequestAdapterOptions {
                    power_preference: wgpu::PowerPreference::HighPerformance,
                    compatible_surface: None,
                    force_fallback_adapter: false,
                })) {
                    Ok(a) => a,
                    Err(e) => {
                        log::error!("GPU init: request_adapter failed: {e}");
                        return None;
                    }
                };

            // Burn-cubecl's `init_device(WgpuSetup::Existing(...))` does NOT
            // re-check features or limits — it trusts our setup. When the
            // shared device is missing features Burn would normally request
            // (it asks for `adapter.features() - MAPPABLE_PRIMARY_BUFFERS`
            // and full `adapter.limits()` plus `experimental_features =
            // ExperimentalFeatures::enabled()` for SPIR-V passthrough on
            // Vulkan), compute kernels silently no-op and tensors come back
            // full of zeros. So we mirror Burn's own request here.
            //
            // Source: `cubecl-wgpu-0.10.0/src/backend/base.rs::request_device`.
            let required_features = adapter
                .features()
                .difference(wgpu::Features::MAPPABLE_PRIMARY_BUFFERS)
                | wgpu::Features::POLYGON_MODE_LINE; // also keep our wireframe need
            let required_limits = adapter.limits();

            let (device, queue) =
                match pollster::block_on(adapter.request_device(&wgpu::DeviceDescriptor {
                    label: Some("squarebob GPU Device"),
                    required_features,
                    required_limits,
                    memory_hints: wgpu::MemoryHints::MemoryUsage,
                    trace: Default::default(),
                    // SAFETY: `required_features` mirrors cubecl-wgpu and may
                    // include adapter-reported experimental features. Wgpu's
                    // contract explicitly warns those APIs may contain UB even
                    // when reached through safe calls. This process accepts that
                    // upstream implementation risk so cubecl kernels can use the
                    // feature set they compile against; the opt-in remains here,
                    // at the single device-creation boundary.
                    experimental_features: unsafe { wgpu::ExperimentalFeatures::enabled() },
                })) {
                    Ok(pair) => pair,
                    Err(e) => {
                        log::error!("GPU init: request_device failed: {e}");
                        return None;
                    }
                };

            let gpu_info = gpu_mem::query();
            if let Some(info) = &gpu_info {
                let to_mib = |b: u64| b / (1024 * 1024);
                log::info!(
                    "GPU: {} — VRAM {} MiB total, {} MiB free ({})",
                    info.name,
                    to_mib(info.dedicated_vram),
                    to_mib(info.free_vram),
                    if info.unified { "unified" } else { "dedicated" },
                );
            } else {
                log::info!("GPU: VRAM query unavailable on this platform");
            }

            Some(Self {
                instance: Arc::new(instance),
                adapter: Arc::new(adapter),
                device: Arc::new(device),
                queue: Arc::new(queue),
                gpu_info,
            })
        }

        /// Conservative VRAM budget for large transient buffers
        /// (BVH, wavefront tile state, OIDN aux). Delegates to
        /// [`gpu_mem::budget_from`] so the 75 %-of-free rule lives in
        /// one place. Returns `None` when `gpu-mem` could not
        /// determine VRAM size for the active adapter.
        pub fn vram_budget(&self) -> Option<u64> {
            gpu_mem::budget_from(self.gpu_info.as_ref()?)
        }
    }

    /// Single unified entry point for creating a GPU buffer.
    ///
    /// Behaviour:
    /// - Applies the WebGPU 16-byte minimum (`size.max(16)`) so callers
    ///   don't each repeat the same guard.
    /// - Calls [`gpu_mem::note_alloc`] *before* allocation so the log
    ///   shows the requested size even if `create_buffer` itself
    ///   triggers an OOM panic.
    /// - `mapped_at_creation` is always `false`. Use [`make_buffer_init`]
    ///   when initial contents are known up-front.
    ///
    /// Use this instead of `device.create_buffer(...)` for every
    /// buffer in the workspace — storage, uniform, vertex, index,
    /// readback staging. The helper is the single place that knows
    /// about VRAM accounting; callers stay focused on size + usage
    /// flags.
    pub fn make_buffer(
        device: &wgpu::Device,
        label: &str,
        size: u64,
        usage: wgpu::BufferUsages,
    ) -> wgpu::Buffer {
        gpu_mem::note_alloc(label, size);
        device.create_buffer(&wgpu::BufferDescriptor {
            label: Some(label),
            size: size.max(16),
            usage,
            mapped_at_creation: false,
        })
    }

    /// Pre-populated variant of [`make_buffer`]. Wraps
    /// `wgpu::util::DeviceExt::create_buffer_init` so initial contents
    /// land in the buffer via a single mapped-at-creation copy
    /// (faster than `create_buffer + queue.write_buffer` for small
    /// param buffers that change rarely).
    ///
    /// Size is taken from `contents.len()` and the same
    /// [`gpu_mem::note_alloc`] visibility hook runs first.
    pub fn make_buffer_init(
        device: &wgpu::Device,
        label: &str,
        contents: &[u8],
        usage: wgpu::BufferUsages,
    ) -> wgpu::Buffer {
        use wgpu::util::DeviceExt as _;
        gpu_mem::note_alloc(label, contents.len() as u64);
        device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some(label),
            contents,
            usage,
        })
    }

    #[derive(Debug, Clone, Copy, PartialEq, Eq)]
    pub struct TextureReadbackLayout {
        width: u32,
        height: u32,
        row_bytes: u32,
        padded_row_bytes: u32,
        buffer_size: u64,
        output_size: usize,
    }

    impl TextureReadbackLayout {
        pub fn new(
            context: &'static str,
            width: u32,
            height: u32,
            bytes_per_pixel: u32,
        ) -> Result<Self, GpuLayoutError> {
            let row_bytes_u64 = u64::from(width)
                .checked_mul(u64::from(bytes_per_pixel))
                .ok_or(GpuLayoutError::Overflow {
                    context,
                    width,
                    height: 1,
                    bytes_per_element: u64::from(bytes_per_pixel),
                })?;
            let row_bytes =
                u32::try_from(row_bytes_u64).map_err(|_| GpuLayoutError::ValueTooLarge {
                    context,
                    value: row_bytes_u64,
                    target: "u32",
                })?;
            let padded_row_bytes = checked_aligned_bytes_per_row(
                context,
                width,
                bytes_per_pixel,
                wgpu::COPY_BYTES_PER_ROW_ALIGNMENT,
            )?;
            let buffer_size = u64::from(padded_row_bytes)
                .checked_mul(u64::from(height))
                .ok_or(GpuLayoutError::Overflow {
                    context,
                    width,
                    height,
                    bytes_per_element: u64::from(padded_row_bytes),
                })?;
            let output_size_u64 =
                checked_2d_buffer_size(context, width, height, u64::from(bytes_per_pixel))?;
            let output_size =
                usize::try_from(output_size_u64).map_err(|_| GpuLayoutError::ValueTooLarge {
                    context,
                    value: output_size_u64,
                    target: "usize",
                })?;

            Ok(Self {
                width,
                height,
                row_bytes,
                padded_row_bytes,
                buffer_size,
                output_size,
            })
        }

        pub fn rgba8(width: u32, height: u32) -> Result<Self, GpuLayoutError> {
            Self::new("RGBA8 texture readback", width, height, 4)
        }

        pub fn row_bytes(&self) -> u32 {
            self.row_bytes
        }

        pub fn padded_row_bytes(&self) -> u32 {
            self.padded_row_bytes
        }

        pub fn buffer_size(&self) -> u64 {
            self.buffer_size
        }

        pub fn output_size(&self) -> usize {
            self.output_size
        }
    }

    /// Grow-only staging allocation plus layout for the most recent texture copy.
    ///
    /// Recurrent render paths keep one instance. A larger frame reallocates once;
    /// smaller frames reuse the existing mapped-read buffer.
    #[derive(Default)]
    pub struct TextureReadback {
        buffer: Option<wgpu::Buffer>,
        capacity: u64,
        layout: Option<TextureReadbackLayout>,
    }

    impl TextureReadback {
        pub fn capacity(&self) -> u64 {
            self.capacity
        }
    }

    /// Encode a tightly packed pixel texture into reusable staging storage.
    pub fn readback_texture_bytes(
        ctx: &GpuContext,
        encoder: &mut wgpu::CommandEncoder,
        texture: &wgpu::Texture,
        width: u32,
        height: u32,
        bytes_per_pixel: u32,
        label: &'static str,
        staging: &mut TextureReadback,
    ) -> Result<(), ReadbackError> {
        let layout = TextureReadbackLayout::new(label, width, height, bytes_per_pixel)?;
        if staging.buffer.is_none() || staging.capacity < layout.buffer_size {
            staging.buffer = Some(make_buffer(
                &ctx.device,
                label,
                layout.buffer_size,
                wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
            ));
            staging.capacity = layout.buffer_size;
        }
        let buffer = staging
            .buffer
            .as_ref()
            .ok_or(ReadbackError::MissingTarget)?;

        encoder.copy_texture_to_buffer(
            wgpu::TexelCopyTextureInfo {
                texture,
                mip_level: 0,
                origin: wgpu::Origin3d::ZERO,
                aspect: wgpu::TextureAspect::All,
            },
            wgpu::TexelCopyBufferInfo {
                buffer,
                layout: wgpu::TexelCopyBufferLayout {
                    offset: 0,
                    bytes_per_row: Some(layout.padded_row_bytes),
                    rows_per_image: Some(layout.height),
                },
            },
            wgpu::Extent3d {
                width: layout.width,
                height: layout.height,
                depth_or_array_layers: 1,
            },
        );
        staging.layout = Some(layout);
        Ok(())
    }

    /// Encode an RGBA8 texture copy into reusable staging storage.
    pub fn readback_texture(
        ctx: &GpuContext,
        encoder: &mut wgpu::CommandEncoder,
        texture: &wgpu::Texture,
        width: u32,
        height: u32,
        staging: &mut TextureReadback,
    ) -> Result<(), ReadbackError> {
        readback_texture_bytes(
            ctx,
            encoder,
            texture,
            width,
            height,
            4,
            "RGBA8 Readback",
            staging,
        )
    }

    /// Map the most recently encoded copy and strip row padding.
    pub fn map_readback(
        ctx: &GpuContext,
        staging: &TextureReadback,
    ) -> Result<Vec<u8>, ReadbackError> {
        let layout = staging.layout.ok_or(ReadbackError::MissingTarget)?;
        let buffer = staging
            .buffer
            .as_ref()
            .ok_or(ReadbackError::MissingTarget)?;
        map_buffer_read(&ctx.device, buffer, |data| {
            let actual = data.len();
            let expected = usize::try_from(layout.buffer_size).map_err(|_| {
                ReadbackError::Layout(GpuLayoutError::ValueTooLarge {
                    context: "RGBA8 mapped range",
                    value: layout.buffer_size,
                    target: "usize",
                })
            })?;
            if actual < expected {
                return Err(ReadbackError::MappedRangeTooSmall { expected, actual });
            }

            let row_bytes = layout.row_bytes as usize;
            let padded_row_bytes = layout.padded_row_bytes as usize;
            let mut pixels = Vec::with_capacity(layout.output_size);
            for row in 0..layout.height as usize {
                let start = row * padded_row_bytes;
                pixels.extend_from_slice(&data[start..start + row_bytes]);
            }
            Ok(pixels)
        })?
    }
}

/// Recoverable error from GPU rendering or buffer/texture readback.
#[derive(Debug)]
pub enum ReadbackError {
    Layout(GpuLayoutError),
    PollFailed(wgpu::PollError),
    CallbackDropped(std::sync::mpsc::RecvError),
    MapFailed(wgpu::BufferAsyncError),
    MissingTarget,
    StagingBufferTooSmall { required: u64, capacity: u64 },
    MappedRangeTooSmall { expected: usize, actual: usize },
    HostAllocation(std::collections::TryReserveError),
    SceneBuild(String),
}

impl From<std::collections::TryReserveError> for ReadbackError {
    fn from(value: std::collections::TryReserveError) -> Self {
        Self::HostAllocation(value)
    }
}

impl From<GpuLayoutError> for ReadbackError {
    fn from(value: GpuLayoutError) -> Self {
        Self::Layout(value)
    }
}

impl std::fmt::Display for ReadbackError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Layout(e) => write!(f, "invalid readback layout: {e}"),
            Self::PollFailed(e) => write!(f, "device poll failed: {e:?}"),
            Self::CallbackDropped(e) => {
                write!(f, "map callback dropped before completion: {e}")
            }
            Self::MapFailed(e) => write!(f, "map_async failed: {e:?}"),
            Self::MissingTarget => write!(f, "readback has no encoded target"),
            Self::StagingBufferTooSmall { required, capacity } => write!(
                f,
                "readback staging buffer too small: requires {required} bytes, capacity {capacity}"
            ),
            Self::MappedRangeTooSmall { expected, actual } => write!(
                f,
                "mapped range too small: expected at least {expected} bytes, got {actual}"
            ),
            Self::HostAllocation(error) => {
                write!(f, "cannot allocate host readback storage: {error}")
            }
            Self::SceneBuild(error) => write!(f, "scene build failed: {error}"),
        }
    }
}

impl std::error::Error for ReadbackError {}

/// Map a buffer exactly once, run `f`, drop the mapped view, then unmap.
///
/// The caller must submit every command that writes `buffer` before calling.
pub fn map_buffer_read<R, F>(
    device: &wgpu::Device,
    buffer: &wgpu::Buffer,
    f: F,
) -> Result<R, ReadbackError>
where
    F: FnOnce(&[u8]) -> R,
{
    let slice = buffer.slice(..);
    let (tx, rx) = std::sync::mpsc::channel();
    slice.map_async(wgpu::MapMode::Read, move |result| {
        let _ = tx.send(result);
    });
    device
        .poll(wgpu::PollType::wait_indefinitely())
        .map_err(ReadbackError::PollFailed)?;
    match rx.recv() {
        Ok(Ok(())) => {}
        Ok(Err(e)) => return Err(ReadbackError::MapFailed(e)),
        Err(e) => return Err(ReadbackError::CallbackDropped(e)),
    }
    let data = slice.get_mapped_range();
    let result = f(&data);
    drop(data);
    buffer.unmap();
    Ok(result)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn checked_layout_rejects_zero_extent() {
        assert!(matches!(
            checked_2d_buffer_size("test", 0, 1, 4),
            Err(GpuLayoutError::ZeroExtent { .. })
        ));
    }

    #[test]
    fn checked_layout_rejects_byte_overflow() {
        assert!(matches!(
            checked_2d_buffer_size("test", u32::MAX, u32::MAX, 32),
            Err(GpuLayoutError::Overflow { .. })
        ));
    }

    #[test]
    fn rgba8_readback_layout_accounts_for_padding() {
        let layout = gpu::TextureReadbackLayout::rgba8(65, 3).unwrap();
        assert_eq!(layout.row_bytes(), 260);
        assert_eq!(layout.padded_row_bytes(), 512);
        assert_eq!(layout.buffer_size(), 1536);
        assert_eq!(layout.output_size(), 780);
    }
}
