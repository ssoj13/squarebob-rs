//! Object ID picking: async GPU readback with 1-frame latency
//! Maps object_id -> file path for hover tooltips and selection

/// Matches `cube_object_id.wgsl` — selected instances OR this into the R32Uint object_id texture.
pub const OBJECT_ID_SELECTED_BIT: u32 = 0x8000_0000;

/// Strip GPU-only bits so lookups match `id_map` keys (allocated without SELECTED_BIT).
#[inline]
pub fn canonical_object_id(id: u32) -> u32 {
    id & !OBJECT_ID_SELECTED_BIT
}

use std::collections::HashMap;
use std::collections::hash_map::Entry;
use std::path::PathBuf;

/// Combined pick info for a single object ID
#[derive(Clone, Debug)]
pub struct PickInfo {
    pub path: PathBuf,
    pub size: u64,
    pub is_dir: bool,
}

/// Object ID picking state (async readback with 1-frame latency)
pub struct PickingState {
    /// Readback buffer (copies entire row for alignment)
    buffer: Option<wgpu::Buffer>,
    buffer_size: u32,
    /// Pending pick request (pixel coords)
    pub pending_pick: Option<(u32, u32)>,
    /// Pending pixel X — read in poll_result after submit + GPU copy completes
    pending_px: Option<u32>,
    /// Last texture width (for reading correct pixel)
    texture_width: u32,
    /// Last successfully read ID
    pub hovered_id: u32,
    /// object_id -> pick info (path, size, is_dir) - rebuilt each frame
    pub id_map: HashMap<u32, PickInfo>,
    /// Next available object ID (0 = background)
    pub next_id: u32,
}

impl Default for PickingState {
    fn default() -> Self {
        Self::new()
    }
}

impl PickingState {
    pub fn new() -> Self {
        Self {
            buffer: None,
            buffer_size: 0,
            pending_pick: None,
            pending_px: None,
            texture_width: 0,
            hovered_id: 0,
            id_map: HashMap::new(),
            next_id: 1,
        }
    }

    /// Reset ID counter for new frame.
    /// Keeps id_map entries — reused by `alloc_id` when traversal order is stable (animation).
    /// Stale entries (id >= next allocation) are harmless: never looked up from current scene.
    pub fn reset_frame(&mut self) {
        log::trace!("picking.reset_frame: {} entries (reuse)", self.id_map.len());
        self.next_id = 1;
    }

    /// Allocate a new object ID and map it to a path.
    /// Skips PathBuf clone if existing entry already matches (common during animation).
    pub fn alloc_id(&mut self, path: &std::path::Path, size: u64, is_dir: bool) -> u32 {
        let id = self.next_id;
        self.next_id += 1;
        match self.id_map.entry(id) {
            Entry::Occupied(ref e) if e.get().path.as_path() == path => {}
            Entry::Occupied(mut e) => {
                e.insert(PickInfo {
                    path: path.to_path_buf(),
                    size,
                    is_dir,
                });
            }
            Entry::Vacant(e) => {
                e.insert(PickInfo {
                    path: path.to_path_buf(),
                    size,
                    is_dir,
                });
            }
        }
        id
    }

    /// Ensure readback buffer exists and is large enough
    pub fn ensure_readback(
        &mut self,
        device: &wgpu::Device,
        width: u32,
    ) -> Result<(), render_core::GpuLayoutError> {
        let layout =
            render_core::gpu::TextureReadbackLayout::new("object ID readback", width, 1, 4)?;
        let bytes_per_row = layout.padded_row_bytes();
        if self.buffer.is_none() || self.buffer_size < bytes_per_row {
            self.buffer = Some(render_core::gpu::make_buffer(
                device,
                "ID Readback",
                u64::from(bytes_per_row),
                wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
            ));
            self.buffer_size = bytes_per_row;
        }
        Ok(())
    }

    /// Request hover pick at pixel coords (call on mouse move)
    pub fn request_pick(&mut self, x: u32, y: u32) {
        log::trace!("picking::request_pick({}, {})", x, y);
        self.pending_pick = Some((x, y));
    }

    /// Submit readback copy command (call during render, after object_id pass)
    pub fn submit_readback(
        &mut self,
        encoder: &mut wgpu::CommandEncoder,
        id_texture: &wgpu::Texture,
        tex_size: (u32, u32),
    ) -> Result<(), render_core::ReadbackError> {
        log::trace!(
            "picking::submit_readback pending={:?} tex_size={:?}",
            self.pending_pick,
            tex_size
        );
        let (px, py) = match self.pending_pick.take() {
            Some(coords) => coords,
            None => {
                log::trace!("picking::submit_readback - no pending pick");
                return Ok(());
            }
        };
        if px >= tex_size.0 || py >= tex_size.1 {
            log::warn!("picking::submit_readback - coords out of bounds");
            return Ok(());
        }
        let layout =
            render_core::gpu::TextureReadbackLayout::new("object ID readback", tex_size.0, 1, 4)?;
        let bytes_per_row = layout.padded_row_bytes();
        if self.buffer_size < bytes_per_row {
            return Err(render_core::ReadbackError::StagingBufferTooSmall {
                required: u64::from(bytes_per_row),
                capacity: u64::from(self.buffer_size),
            });
        }
        let buf = self
            .buffer
            .as_ref()
            .ok_or(render_core::ReadbackError::MissingTarget)?;

        self.texture_width = tex_size.0;
        self.pending_px = Some(px);

        // Copy entire row containing our pixel (must be submitted before map_async — see poll_result)
        encoder.copy_texture_to_buffer(
            wgpu::TexelCopyTextureInfo {
                texture: id_texture,
                mip_level: 0,
                origin: wgpu::Origin3d { x: 0, y: py, z: 0 },
                aspect: wgpu::TextureAspect::All,
            },
            wgpu::TexelCopyBufferInfo {
                buffer: buf,
                layout: wgpu::TexelCopyBufferLayout {
                    offset: 0,
                    bytes_per_row: Some(bytes_per_row),
                    rows_per_image: Some(1),
                },
            },
            wgpu::Extent3d {
                width: tex_size.0,
                height: 1,
                depth_or_array_layers: 1,
            },
        );
        Ok(())
    }

    /// Read pick result (call AFTER `queue.submit` for the encoder that included `submit_readback`).
    /// Waits for the copy, then maps — same ordering contract as `render_core::map_readback`.
    pub fn poll_result(&mut self, device: &wgpu::Device) -> Result<(), render_core::ReadbackError> {
        let Some(px) = self.pending_px else {
            log::trace!("picking::poll_result - no pending_px");
            return Ok(());
        };

        let result = (|| {
            let buf = self
                .buffer
                .as_ref()
                .ok_or(render_core::ReadbackError::MissingTarget)?;
            let offset_u64 = u64::from(px) * 4;
            let offset = usize::try_from(offset_u64).map_err(|_| {
                render_core::GpuLayoutError::ValueTooLarge {
                    context: "object ID readback offset",
                    value: offset_u64,
                    target: "usize",
                }
            })?;
            let end = offset
                .checked_add(4)
                .ok_or(render_core::GpuLayoutError::ValueTooLarge {
                    context: "object ID readback end",
                    value: offset_u64,
                    target: "usize",
                })?;

            render_core::map_buffer_read(device, buf, |data| {
                let bytes = data.get(offset..end).ok_or(
                    render_core::ReadbackError::MappedRangeTooSmall {
                        expected: end,
                        actual: data.len(),
                    },
                )?;
                Ok(u32::from_le_bytes([bytes[0], bytes[1], bytes[2], bytes[3]]))
            })?
        })();
        self.pending_px = None;
        let raw = result?;

        // Texture encodes selected instances as id | SELECTED_BIT; id_map uses canonical ids only.
        self.hovered_id = canonical_object_id(raw);
        log::trace!(
            "picking::poll_result raw={raw:#x} canonical={}",
            self.hovered_id
        );
        Ok(())
    }

    /// Look up path for an object ID
    pub fn path_for_id(&self, id: u32) -> Option<&PathBuf> {
        let id = canonical_object_id(id);
        if id == 0 {
            return None;
        }
        let result = self.id_map.get(&id).map(|info| &info.path);
        if result.is_none() {
            log::debug!(
                "path_for_id({}): not found in id_map (map has {} entries)",
                id,
                self.id_map.len()
            );
        }
        result
    }

    /// Look up object ID for a path (reverse lookup)
    pub fn id_for_path(&self, path: &std::path::Path) -> Option<u32> {
        self.id_map
            .iter()
            .find(|(_, info)| info.path.as_path() == path)
            .map(|(id, _)| *id)
    }

    /// Look up file size for an object ID
    pub fn size_for_id(&self, id: u32) -> Option<u64> {
        self.id_map
            .get(&canonical_object_id(id))
            .map(|info| info.size)
    }

    /// Look up directory flag for an object ID
    pub fn is_dir_for_id(&self, id: u32) -> Option<bool> {
        self.id_map
            .get(&canonical_object_id(id))
            .map(|info| info.is_dir)
    }

    /// Get full pick info for an object ID
    pub fn info_for_id(&self, id: u32) -> Option<&PickInfo> {
        let id = canonical_object_id(id);
        if id == 0 {
            return None;
        }
        self.id_map.get(&id)
    }
}
