# pt-mats

pt-mats is a small, reusable material classification and preset library for squarebob-rs.

## Why this exists
The renderer needs deterministic, data-driven materials for two different pipelines:

- **PBR (raster)**: per-instance colors that can be blended with procedural/materialized palettes.
- **PT (path tracing)**: physically-inspired materials (metal/glass/emissive, etc.) packed into GPU buffers.

Keeping this logic in a dedicated crate makes it reusable across projects and keeps the app layer free of
material-model details. It also makes it possible to maintain a consistent material taxonomy between
PBR and PT while still allowing pipeline-specific behavior (e.g., lights only in PT).

## What it provides
- `MaterialClass`: material taxonomy (including light/glass variants).
- `MaterialLibrary`: CPU-side material presets -> `GpuMaterial` array.
- `MaterializeMode`: classification modes (by extension, path, size, age, random).
- `MaterializeSettings`: flags and probabilities for light/glass categories.
- `classify_path_filtered(...)`: deterministic classifier with filtering & remapping rules.

## Where it is used
- `crates/render-3d/src/lib.rs`: PBR color blending and PT material assignment.
- PT shaders (`crates/pt-megakernel` / `crates/pt-wavefront`): consume `GpuMaterial` arrays.

This crate does **not** implement a full Standard Surface model. It uses a compact custom material
layout that matches the existing PT shaders and is sufficient for PBR tinting and PT shading.
