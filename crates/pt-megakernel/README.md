# pt-megakernel

pt-megakernel implements the monolithic (megakernel) path tracing pipeline.

## Why this exists
The megakernel approach is the most compact PT implementation and is useful for testing,
feature parity, and performance comparisons with the wavefront pipeline.

## What it provides
- `PathTraceCompute`: PT compute pipeline, accumulation, and render target management.
- ReSTIR DI/GI integration and history management.
- Path guiding and adaptive sampling hooks.

## Where it is used
- `crates/render-3d/src/lib.rs`: PT rendering in 3D mode.
