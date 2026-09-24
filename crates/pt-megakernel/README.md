# pt-megakernel

pt-megakernel owns the path tracing renderer, including megakernel dispatch and integration of the optional wavefront pipeline.

## Why this exists
`PathTraceCompute` keeps scene upload, BVH build/refit, accumulation, sampling controls, and output management in one place for both dispatch modes.

## What it provides
- `PathTraceCompute`: PT compute pipeline, accumulation, and render target management.
- ReSTIR DI/GI integration and history management.
- Path guiding and adaptive sampling pipelines and controls.
- Optional wavefront dispatch through `pt-wavefront`, selected by the renderer's options.
- GPU BVH build/refit through `bvh-gpu`, with CPU fallback through `pt-core`.

## Where it is used
- `crates/render-3d/src/pt`: PT rendering in 3D mode.
