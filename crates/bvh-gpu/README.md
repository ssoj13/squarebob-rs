# bvh-gpu

bvh-gpu provides the GPU-side LBVH (linear BVH) build pipeline used by the path tracer.

## Why this exists
GPU BVH build is a specialized pipeline (Morton codes, radix sort, LBVH topology, AABB reduction)
that we want to reuse across PT implementations and keep isolated from renderer/app code.

## What it provides

`GpuBvhConfig::gpu_threshold` defaults to 512 instances. Its `high_quality` and `wide_bvh` fields are reserved for future work; they do not enable those algorithms today.

- Morton-code generation and GPU radix sort.
- LBVH topology build (Karras-style split selection).
- AABB reduction, BVH refit, and validation helpers.
- Readback/linearization utilities for PT consumption.

## Integration

- `crates/pt-megakernel`: `PathTraceCompute` owns `GpuBvhBuilder` and selects GPU build, refit, or CPU fallback for instance BVHs.
- `crates/pt-core`: this crate consumes its shared instance and BVH node layouts.
