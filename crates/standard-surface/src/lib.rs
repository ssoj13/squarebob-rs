//! Autodesk Standard Surface — parameter struct + WGSL shader sources.
//!
//! This is a **stripped** copy of the upstream `ssoj13/alembic-rs`
//! `standard-surface` crate. The upstream crate also ships a full
//! `wgpu`-based rasterizer pipeline (`create_pipeline`,
//! `create_skybox_pipeline`, `create_shadow_pipeline`,
//! `BindGroupLayouts`, etc.), which is not needed by the squarebob
//! path-tracer — squarebob consumes only the parameter struct +
//! WGSL shader-lib snippets and runs everything ray-traced.
//!
//! Stripping out the rasterizer eliminates the dependency on a
//! specific `wgpu` API version (the upstream crate was on wgpu 27;
//! squarebob runs on wgpu 29). If you ever want the rasterizer back,
//! re-vendor the upstream `lib.rs` body and port the breaking
//! `RenderPipelineDescriptor` field renames (`multiview` →
//! `multiview_mask`, etc.).
//!
//! ## What's exposed
//!
//! * [`StandardSurfaceParams`] — GPU-ready material parameter struct
//!   (`Pod + Zeroable`, `vec4`-packed). Same memory layout as the
//!   WGSL `Material` struct the squarebob shaders read.
//! * Auxiliary uniform types ([`CameraUniform`] / [`Light`] /
//!   [`LightRig`] / [`ModelUniform`] / [`ShadowUniform`]) — kept
//!   because they don't pull in `wgpu`, only `bytemuck` + `glam`.
//!   They're unused by squarebob today; trim later if they grow
//!   wgpu-dependent fields upstream.
//! * [`SHADER_SOURCE`] and [`shader_lib`] — WGSL shader source
//!   strings, suitable for `include_str!`-style usage in callers
//!   that compose their own shader modules.

mod params;

pub use params::{
    CameraUniform, Light, LightRig, LightUniform, ModelUniform, ShadowUniform,
    StandardSurfaceParams,
};

/// Embedded main shader source (full Standard Surface BRDF in WGSL).
pub const SHADER_SOURCE: &str = include_str!("shaders/standard_surface.wgsl");

/// Per-section WGSL helper sources. Suitable for include-style
/// composition in callers that build their own shader modules.
pub mod shader_lib {
    pub const COMMON: &str = include_str!("shaders/lib/common.wgsl");
    pub const FRESNEL: &str = include_str!("shaders/lib/fresnel.wgsl");
    pub const MICROFACET: &str = include_str!("shaders/lib/microfacet.wgsl");
    pub const DIFFUSE: &str = include_str!("shaders/lib/diffuse.wgsl");
}
