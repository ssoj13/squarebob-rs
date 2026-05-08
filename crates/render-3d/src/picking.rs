//! Object ID picking: async GPU readback with 1-frame latency
//! Maps object_id -> file path for hover tooltips and selection

/// Matches `cube_object_id.wgsl` — selected instances OR this into the R32Uint object_id texture.
pub const OBJECT_ID_SELECTED_BIT: u32 = 0x8000_0000;

/// Strip GPU-only bits so lookups match `id_map` keys (allocated without SELECTED_BIT).
#[inline]
pub fn canonical_object_id(id: u32) -> u32 {
    id & !OBJECT_ID_SELECTED_BIT
}

use std::collections::hash_map::Entry;
use std::collections::HashMap;
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
    pub fn ensure_readback(&mut self, device: &wgpu::Device, width: u32) {
        // Buffer size = aligned row (256 bytes alignment for wgpu)
        let bytes_per_row = (width * 4 + 255) & !255;
        if self.buffer.is_none() || self.buffer_size < bytes_per_row {
            self.buffer = Some(device.create_buffer(&wgpu::BufferDescriptor {
                label: Some("ID Readback"),
                size: bytes_per_row as u64,
                usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
                mapped_at_creation: false,
            }));
            self.buffer_size = bytes_per_row;
        }
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
    ) {
        log::trace!(
            "picking::submit_readback pending={:?} tex_size={:?}",
            self.pending_pick,
            tex_size
        );
        let (px, py) = match self.pending_pick.take() {
            Some(coords) => coords,
            None => {
                log::trace!("picking::submit_readback - no pending pick");
                return;
            }
        };
        if px >= tex_size.0 || py >= tex_size.1 {
            log::warn!("picking::submit_readback - coords out of bounds");
            return;
        }
        let Some(buf) = &self.buffer else {
            log::warn!("picking::submit_readback - no buffer");
            return;
        };

        self.texture_width = tex_size.0;
        self.pending_px = Some(px);
        let bytes_per_row = (tex_size.0 * 4 + 255) & !255;

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
    }

    /// Read pick result (call AFTER `queue.submit` for the encoder that included `submit_readback`).
    /// Waits for the copy, then maps — same ordering contract as `render_core::map_readback`.
    pub fn poll_result(&mut self, device: &wgpu::Device) {
        let Some(px) = self.pending_px else {
            log::trace!("picking::poll_result - no pending_px");
            return;
        };
        let Some(buf) = self.buffer.as_ref() else {
            log::warn!("picking::poll_result - no buffer");
            self.pending_px = None;
            return;
        };

        // Ensure the copy command has finished before mapping (map_async before submit caused BufferStillMapped)
        let _ = device.poll(wgpu::PollType::wait_indefinitely());

        let buffer_slice = buf.slice(..);
        let (tx, rx) = std::sync::mpsc::channel();
        buffer_slice.map_async(wgpu::MapMode::Read, move |result| {
            let _ = tx.send(result);
        });
        let _ = device.poll(wgpu::PollType::wait_indefinitely());
        if let Err(e) = rx.recv().unwrap() {
            log::warn!("picking::poll_result - map_async failed: {e:?}");
            self.pending_px = None;
            return;
        }

        let data = buffer_slice.get_mapped_range();
        let offset = (px as usize) * 4;
        if offset + 4 <= data.len() {
            let raw = u32::from_le_bytes([
                data[offset],
                data[offset + 1],
                data[offset + 2],
                data[offset + 3],
            ]);
            // Texture encodes selected instances as id | SELECTED_BIT; id_map uses canonical ids only.
            self.hovered_id = canonical_object_id(raw);
            log::trace!(
                "picking::poll_result raw={raw:#x} canonical={}",
                self.hovered_id
            );
        }
        drop(data);
        buf.unmap();
        self.pending_px = None;
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
