# pt-wavefront

pt-wavefront implements the wavefront path tracing pipeline (staged raygen/intersect/shade).

## Why this exists
The staged pipeline provides tiled ray queues, intersection, shading, and finalization. `pt-megakernel::PathTraceCompute` owns its scene and dispatch integration.

## What it provides
- Wavefront PT pipelines and buffer orchestration.
- Ray generation, intersection, shading, count swap, and finalization pipelines.
- Tile preparation and queue buffer management; `PathTraceCompute` owns accumulation.

## Where it is used
- `crates/pt-megakernel/src/compute.rs`: creates and dispatches `WavefrontPipeline` when wavefront mode is enabled.
- `crates/render-3d/src/pt`: selects wavefront mode through render options.
