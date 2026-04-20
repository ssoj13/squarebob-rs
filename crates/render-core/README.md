# render-core

render-core wraps the low-level GPU context used by the renderers.

## Why this exists
We want a minimal, reusable GPU context (device/queue/surface formats) without dragging in
renderer or app logic.

## What it provides
- `GpuContext` and helper utilities for wgpu setup.
- Shared GPU-related glue used by render-2d/render-3d.

## Where it is used
- `crates/render-3d` and any future GPU renderer crates.
