# pt-core

pt-core is the shared foundation for path tracing.

## Why this exists
The megakernel path tracer and GPU BVH builder share instance and BVH layouts. This crate defines those types and the CPU BVH builder without depending on a renderer.

## What it provides
- `Instance`, `BvhNode`, and AABB representations shared with the path tracer.
- GPU data layouts for nodes and instances; `GpuMaterial` re-exports `standard_surface::StandardSurfaceParams`.
- CPU SAH instance BVH builder and GPU upload helpers.

## Where it is used
- `crates/pt-megakernel`: scene upload and CPU BVH fallback.
- `crates/bvh-gpu`: shared instance and node types for GPU BVH construction.
- `crates/render-3d`: PT scene preparation.
