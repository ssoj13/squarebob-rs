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
- `MaterialDistribution`: shaping curve applied to the source value
  before bucketing (direct, quantised, gradient, spatial noise, bands).
- `MaterializeMode`: legacy preset shortcut for `MaterialSource`.
- `MaterializeSettings`: full classification knob bundle (seed,
  source/distribution choice, quant levels, palette pin, etc.).
- `MaterialInput`: per-cube inputs handed to `classify_to_index`.
- `classify_to_index(input, settings, library_size) -> u32`: pick a
  material index in `0..library_size`.
- Palette helpers (`Palette`, `sample_palette`,
  `auto_palette_for_source`, `hierarchical_path_value`) — used by
  upstream colour-ramp consumers, not by the classifier itself.

## Where it is used

- `crates/render-3d/src/renderer3d/material_cache.rs`: calls
  `classify_to_index` once per unique path; results are cached per
  PBR/PT bucket and invalidated on `MaterializeSettings` change or
  library identity change.
- `crates/render-3d/src/renderer3d/instance_collect.rs`: consumes
  palette + `MaterialDistribution` for per-cube colour ramps.

## What lives elsewhere

- Material slots, JSON serialisation, per-cube variance: `pt-material`.
- GPU material layout (`GpuMaterial` / `StandardSurfaceParams`):
  `pt-core` and `standard-surface`.
- Glass / light overrides (legacy `MaterialClass` slot routing): gone.
  Glass-vs-emissive is now a property of the `StandardSurface` params
  themselves (transmission weight / emission weight), edited per
  library slot.
