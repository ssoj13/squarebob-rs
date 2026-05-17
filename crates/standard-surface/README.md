# standard-surface

Autodesk Standard Surface shader for wgpu, ported from MaterialX GLSL to WGSL.

## What is Standard Surface?

[Autodesk Standard Surface](https://autodesk.github.io/standard-surface/) is an uber-shader
designed to represent the vast majority of materials used in VFX and animation production.
It's the default shader in Maya, Arnold, and many other tools.

This crate provides a WGSL implementation for use with wgpu.

## Features

- Full Standard Surface parameter set (base, specular, metalness, coat, emission, etc.)
- GGX microfacet BRDF with Smith height-correlated masking-shadowing
- Oren-Nayar diffuse
- Fresnel: dielectric, conductor, and Schlick approximations
- Energy compensation for multiple scattering
- Thin film iridescence (optional)

## Usage

```rust
use standard_surface::{StandardSurfaceMaterial, StandardSurfacePipeline};

let material = StandardSurfaceMaterial {
    base: 1.0,
    base_color: [0.8, 0.2, 0.1],
    metalness: 0.0,
    specular: 1.0,
    specular_roughness: 0.3,
    ..Default::default()
};

let pipeline = StandardSurfacePipeline::new(&device, surface_format);
pipeline.draw(&mut render_pass, &camera, &mesh, &material);
```

## Credits

- [MaterialX](https://github.com/AcademySoftwareFoundation/MaterialX) - Original GLSL implementation
- [Autodesk Standard Surface](https://github.com/Autodesk/standard-surface) - Specification
- Academy Software Foundation, Sony Pictures Imageworks, Autodesk

## License

Apache-2.0 (same as MaterialX)
