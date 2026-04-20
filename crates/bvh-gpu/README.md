# bvh-gpu

bvh-gpu provides the GPU-side LBVH (linear BVH) build pipeline used by the path tracer.

## Why this exists
GPU BVH build is a specialized pipeline (Morton codes, radix sort, LBVH topology, AABB reduction)
that we want to reuse across PT implementations and keep isolated from renderer/app code.

## What it provides
- GPU radix sort for Morton codes.
- LBVH topology build (Karras-style split selection).
- AABB reduction and validation helpers.
- Readback/linearization utilities for PT consumption.

## Where it is used
- `crates/pt-core` and `crates/pt-megakernel`: building per-frame instance BVHs.
- `crates/render-3d/src/lib.rs`: PT scene upload when GPU BVH build is enabled.
