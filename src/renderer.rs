/// Renderer abstraction: CPU (rayon) or GPU (wgpu) backends.
///
/// Most shared renderer types live in the render-shared crate. This module
/// re-exports them and keeps the app-local CPU treemap wrapper.
pub use render_shared::*;

pub mod cpu {
    use dirstat_core::DirEntry;
    use render_core::Viewport;
    use treemap::TreeMapOptions;

    pub fn render(root: &DirEntry, viewport: &Viewport, opts: &TreeMapOptions) -> Vec<u8> {
        let w = viewport.width;
        let h = viewport.height;

        // For now, render at 1:1 scale - pan/zoom will be added later
        // Layout in world space
        let world_w = w as f32 / viewport.zoom;
        let world_h = h as f32 / viewport.zoom;

        treemap::layout(root, -viewport.pan[0], -viewport.pan[1], world_w, world_h, opts);
        treemap::render(root, w, h, opts)
    }
}
