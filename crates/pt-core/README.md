# pt-core

pt-core is the shared foundation for path tracing.

## Why this exists
Both megakernel and wavefront PT need the same scene representation, BVH data, and GPU buffer
layouts. This crate provides those shared definitions so PT implementations stay consistent.

## What it provides
- `Instance` representation and instance BVH build (CPU SAH).
- GPU data layouts for nodes, instances, and materials.
- BVH builders and helpers used by PT pipelines.

## Where it is used
- `crates/pt-megakernel` and `crates/pt-wavefront`.
- `crates/render-3d`: PT scene upload and BVH build selection.
