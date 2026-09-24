# render-core

render-core holds shared viewport, GPU setup, checked GPU layouts, and readback utilities.

## Why this exists
The app creates one `GpuContext` and passes its instance, device, and queue to eframe and the renderers. Layout and readback checks live here so callers use the same error handling.

## What it provides
- `gpu::GpuContext` for wgpu instance, adapter, device, queue, and format selection.
- `Viewport` pan and zoom state.
- Checked buffer and texture layout helpers with `GpuLayoutError`.
- Texture readback and buffer mapping helpers with `ReadbackError`.

## Where it is used
- `src/main.rs`: creates the shared GPU context for eframe.
- `crates/treemap`, `crates/render-3d`, and PT/BVH crates: layout and readback helpers.
