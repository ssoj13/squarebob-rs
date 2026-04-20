//! Object ID picking: async GPU readback with 1-frame latency
//! Maps object_id -> file path for hover tooltips and selection

use std::collections::hash_map::Entry;
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

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
    /// Pending pixel X for deferred read (after async map)
    pending_px: Option<u32>,
    /// Flag set by map_async callback when buffer is ready
    map_ready: Arc<AtomicBool>,
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
            map_ready: Arc::new(AtomicBool::new(false)),
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
                e.insert(PickInfo { path: path.to_path_buf(), size, is_dir });
            }
            Entry::Vacant(e) => {
                e.insert(PickInfo { path: path.to_path_buf(), size, is_dir });
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
            // Reset map_ready when buffer is recreated
            self.map_ready.store(false, Ordering::SeqCst);
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
        log::trace!("picking::submit_readback pending={:?} tex_size={:?}", self.pending_pick, tex_size);
        let (px, py) = match self.pending_pick.take() {
            Some(coords) => coords,
            None => { log::trace!("picking::submit_readback - no pending pick"); return; },
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
        self.map_ready.store(false, Ordering::SeqCst);
        let bytes_per_row = (tex_size.0 * 4 + 255) & !255;

        // Copy entire row containing our pixel
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
            wgpu::Extent3d { width: tex_size.0, height: 1, depth_or_array_layers: 1 },
        );

        // Start async mapping with callback
        let ready = Arc::clone(&self.map_ready);
        buf.slice(..).map_async(wgpu::MapMode::Read, move |result| {
            if result.is_ok() {
                ready.store(true, Ordering::SeqCst);
            }
        });
    }

    /// Non-blocking poll to read pick result (call AFTER queue.submit).
    /// Uses 1-frame latency: submit in frame N, read result in frame N+1.
    pub fn poll_result(&mut self, device: &wgpu::Device) {
        let Some(px) = self.pending_px else {
            log::trace!("picking::poll_result - no pending_px");
            return;
        };
        let Some(buf) = &self.buffer else {
            log::warn!("picking::poll_result - no buffer");
            self.pending_px = None;
            return;
        };

        // Non-blocking poll to progress GPU work
        let _ = device.poll(wgpu::PollType::Poll);

        // Check if map_async callback has been called
        if !self.map_ready.load(Ordering::SeqCst) {
            log::trace!("picking::poll_result - buffer not ready, retry next frame");
            return;
        }

        // Buffer is ready - read the data
        self.pending_px = None;
        let slice = buf.slice(..);
        let data = slice.get_mapped_range();
        let offset = (px as usize) * 4;
        if offset + 4 <= data.len() {
            let new_id = u32::from_le_bytes([
                data[offset],
                data[offset+1],
                data[offset+2],
                data[offset+3]
            ]);
            self.hovered_id = new_id;
            log::trace!("picking::poll_result read id={}", new_id);
        }
        drop(data);
        buf.unmap();
        self.map_ready.store(false, Ordering::SeqCst);
    }

    /// Look up path for an object ID
    pub fn path_for_id(&self, id: u32) -> Option<&PathBuf> {
        if id == 0 { return None; }
        let result = self.id_map.get(&id).map(|info| &info.path);
        if result.is_none() {
            log::debug!("path_for_id({}): not found in id_map (map has {} entries)", id, self.id_map.len());
        }
        result
    }

    /// Look up object ID for a path (reverse lookup)
    pub fn id_for_path(&self, path: &std::path::Path) -> Option<u32> {
        self.id_map.iter()
            .find(|(_, info)| info.path.as_path() == path)
            .map(|(id, _)| *id)
    }

    /// Look up file size for an object ID
    pub fn size_for_id(&self, id: u32) -> Option<u64> {
        self.id_map.get(&id).map(|info| info.size)
    }

    /// Look up directory flag for an object ID
    pub fn is_dir_for_id(&self, id: u32) -> Option<bool> {
        self.id_map.get(&id).map(|info| info.is_dir)
    }

    /// Get full pick info for an object ID
    pub fn info_for_id(&self, id: u32) -> Option<&PickInfo> {
        if id == 0 { return None; }
        self.id_map.get(&id)
    }
}
