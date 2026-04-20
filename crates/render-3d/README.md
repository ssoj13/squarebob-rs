# render-3d

render-3d is the 3D treemap renderer (PBR + PT integration).

## Why this exists
The 3D renderer is a substantial subsystem (GPU pipelines, picking, PT dispatch) and needs to be
reusable by the app and any future tools without pulling in UI code.

## What it provides
- PBR raster pipeline (instanced cubes, hover/selection).
- PT integration (megakernel or wavefront) with BVH build selection.
- Environment map handling and render target management.
- CPU picking that matches render placement.

## Where it is used
- `src/app`: 3D treemap view and PT mode.
