# standard-surface

Vendored Autodesk Standard Surface parameters and WGSL shader sources. This crate does not provide a wgpu render pipeline.

## What is Standard Surface?

[Autodesk Standard Surface](https://autodesk.github.io/standard-surface/) is an uber-shader
designed to represent the vast majority of materials used in VFX and animation production.
It's the default shader in Maya, Arnold, and many other tools.

The squarebob material library and path tracer use `StandardSurfaceParams`. This crate also exports WGSL source strings for callers that compose shader modules. The upstream raster pipeline was removed from this vendored crate; callers create their own pipelines if needed.

## Features

- `StandardSurfaceParams`: a nine-`vec4` GPU material layout shared with `pt-core::GpuMaterial` and `pt-material`.
- Material constructors and setters for diffuse, plastic, metal, glass, emission, coat, and opacity.
- `sanitized_for_gpu()` to bound specular and coat IOR before GPU upload.
- `SHADER_SOURCE` and `shader_lib::{COMMON, FRESNEL, MICROFACET, DIFFUSE}` for WGSL composition.
- Camera, light, model, and shadow uniform structs retained from the upstream crate.
- WGSL sources for GGX/Smith specular and Oren-Nayar diffuse shading, plus dielectric, conductor, and Schlick Fresnel helpers.
- Multiple-scattering energy-compensation helpers in the microfacet WGSL library.

## Usage

```rust
use standard_surface::{SHADER_SOURCE, StandardSurfaceParams};

let mut material = StandardSurfaceParams::default();
material.set_metalness(1.0);
material.set_roughness(0.3);
let gpu_material = material.sanitized_for_gpu();

assert!(!SHADER_SOURCE.is_empty());
assert!(gpu_material.params1.w >= StandardSurfaceParams::MIN_IOR);
```

`gpu_material` uses the same packed layout as `pt-core::GpuMaterial`. The caller supplies wgpu buffers and pipelines.

## Credits

- [MaterialX](https://github.com/AcademySoftwareFoundation/MaterialX) - Original GLSL implementation
- [Autodesk Standard Surface](https://github.com/Autodesk/standard-surface) - Specification
- Academy Software Foundation, Sony Pictures Imageworks, Autodesk

## License

Apache-2.0 (same as MaterialX)
