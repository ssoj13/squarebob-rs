# pt-wavefront

pt-wavefront implements the wavefront path tracing pipeline (staged raygen/intersect/shade).

## Why this exists
Wavefront PT allows better scheduling, tiling, and data-driven control compared to a single
megakernel. It is also a good base for experiments like ReSTIR and adaptive sampling.

## What it provides
- Wavefront PT pipelines and buffer orchestration.
- Per-stage dispatch (raygen, intersect, shade, resolve).
- Tile-based execution and accumulation controls.

## Where it is used
- `crates/render-3d/src/lib.rs`: optional PT pipeline when wavefront is enabled.
