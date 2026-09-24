# pt-mats

Classification-only helper crate for the squarebob-rs materialize
pipeline.

## Why this exists

After the Phase 4 split, `pt-mats` no longer owns the material data
model — `pt-material::MaterialLibrary` is the single source of truth
for per-scene material slots, with user-editable
`StandardSurfaceParams` + per-attribute variance. `pt-mats` keeps the
*classification* side: given a cube's metadata, pick a `u32` material
index into the caller-supplied library.

## What it provides

- `MaterialSource`: scalar dimension to classify on (extension, path,
  size, age, depth, random).
- `MaterialDistribution`: source-value shaping before weighted slot selection
  (`Direct`, `Stratified`, `Spatial`, `Perlin`, `Gradient`).
- `MaterializeMode`: legacy preset shortcut for `MaterialSource`.
- `MaterializeSettings`: classification settings (seed, source, distribution,
  band count, spatial scale) plus palette settings consumed by color ramps.
- `MaterialInput`: per-cube inputs handed to `classify_to_index`.
- `classify_to_index(input, settings, weights) -> u32`: pick a material
  index in `0..weights.len()` using non-negative per-slot weights.
  An empty or all-zero weight slice, or `MaterialSource::None`, selects slot 0.
- `override_picker`: a separate seeded vote for material overrides using
  the same distribution modes.
- Palette helpers (`Palette`, `sample_palette`,
  `auto_palette_for_source`, `hierarchical_path_value`) — used by
  upstream colour-ramp consumers, not by the classifier itself.

## Where it is used

- `crates/render-3d/src/renderer3d/material_cache.rs`: calls
  `classify_to_index` with the current library weights. Cache entries are
  invalidated when settings or library identity change; spatial and Perlin
  distributions are evaluated for each cube.
- `crates/render-3d/src/renderer3d/instance_collect.rs`: consumes
  palette + `MaterialDistribution` for per-cube colour ramps.

## What lives elsewhere

- Material slots, JSON serialisation, per-cube variance: `pt-material`.
- GPU material layout (`GpuMaterial` / `StandardSurfaceParams`):
  `pt-core` and `standard-surface`.
- Glass and emissive behavior is represented by the transmission and
  emission weights in each library slot's `StandardSurfaceParams`.
