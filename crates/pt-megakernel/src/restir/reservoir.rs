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

/// Reservoir for Resampled Importance Sampling (RIS).
/// Stores weighted random samples for combination.
#[repr(C)]
#[derive(Debug, Clone, Copy, Pod, Zeroable)]
pub struct Reservoir {
    /// Currently selected sample
    pub sample: Sample,
    /// Sum of weights seen so far
    pub w_sum: f32,
    /// Number of samples seen (M)
    pub m: u32,
    /// Final weight for unbiased contribution
    pub w: f32,
    pub _pad: u32,
}

impl Reservoir {
    /// Size in bytes for GPU buffer allocation.
    pub const SIZE: usize = std::mem::size_of::<Self>();
}

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
