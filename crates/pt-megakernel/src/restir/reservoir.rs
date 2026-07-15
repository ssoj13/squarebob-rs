//! ReSTIR reservoir and sample structures.

use bytemuck::{Pod, Zeroable};

/// Light sample for ReSTIR.
/// Stores the sampled light/path contribution.
#[repr(C)]
#[derive(Debug, Clone, Copy, Pod, Zeroable)]
pub struct Sample {
    /// Light/hit position
    pub position: [f32; 3],
    /// Sample validity (0 = invalid)
    pub valid: u32,
    /// Incoming direction (from hit to light)
    pub wi: [f32; 3],
    /// Light type (0=env, 1=emissive)
    pub light_type: u32,
    /// Radiance estimate
    pub radiance: [f32; 3],
    /// Distance to light
    pub dist: f32,
    /// Normal at sample point
    pub normal: [f32; 3],
    pub _pad: u32,
}

/// Complete receiving-surface state used by every ReSTIR target evaluation.
/// Layout mirrors WGSL `RestirSurface` exactly.
#[repr(C)]
#[derive(Debug, Clone, Copy, Pod, Zeroable)]
pub struct RestirSurface {
    pub position: [f32; 3],
    pub instance_id: u32,
    pub normal: [f32; 3],
    pub material_id: u32,
    pub view: [f32; 3],
    pub valid: u32,
    pub diffuse_color: [f32; 3],
    pub roughness: f32,
    pub f0: [f32; 3],
    pub specular_weight: f32,
    pub opacity: f32,
    pub _pad: [f32; 3],
}

/// Reservoir for Resampled Importance Sampling (RIS).
/// Stores one selected light sample plus the receiver state defining its target.
#[repr(C)]
#[derive(Debug, Clone, Copy, Pod, Zeroable)]
pub struct Reservoir {
    pub sample: Sample,
    pub w_sum: f32,
    pub m: u32,
    pub w: f32,
    pub _pad: u32,
    pub surface: RestirSurface,
}

impl Reservoir {
    /// Size in bytes for GPU buffer allocation.
    pub const SIZE: usize = std::mem::size_of::<Self>();
}

const _: () = {
    assert!(std::mem::size_of::<Sample>() == 64);
    assert!(std::mem::align_of::<Sample>() == 4);
    assert!(std::mem::size_of::<RestirSurface>() == 96);
    assert!(std::mem::align_of::<RestirSurface>() == 4);
    assert!(std::mem::offset_of!(RestirSurface, position) == 0);
    assert!(std::mem::offset_of!(RestirSurface, normal) == 16);
    assert!(std::mem::offset_of!(RestirSurface, view) == 32);
    assert!(std::mem::offset_of!(RestirSurface, diffuse_color) == 48);
    assert!(std::mem::offset_of!(RestirSurface, f0) == 64);
    assert!(std::mem::offset_of!(RestirSurface, opacity) == 80);
    assert!(std::mem::size_of::<Reservoir>() == 176);
    assert!(std::mem::align_of::<Reservoir>() == 4);
    assert!(std::mem::offset_of!(Reservoir, surface) == 80);
};

/// Motion vector for temporal reprojection.
#[repr(C)]
#[derive(Debug, Clone, Copy, Pod, Zeroable)]
pub struct MotionVector {
    /// Screen-space motion (pixels)
    pub motion: [f32; 2],
    /// Depth at current frame
    pub depth: f32,
    /// Valid flag (for disocclusion)
    pub valid: u32,
}
