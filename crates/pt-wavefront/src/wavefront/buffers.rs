//! Wavefront buffer structures.

use bytemuck::{Pod, Zeroable};

/// Ray buffer element for wavefront processing.
#[repr(C)]
#[derive(Debug, Clone, Copy, Pod, Zeroable)]
pub struct WfRay {
    pub origin: [f32; 3],
    pub pixel_id: u32,      // Which pixel this ray belongs to
    pub dir: [f32; 3],
    pub bounce: u32,        // Current bounce depth
    pub throughput: [f32; 3],
    pub flags: u32,         // Bit 0: active, Bit 1: shadow ray, etc
}

/// Hit result for intersection pass.
/// Layout must match WGSL: vec3<f32> requires 16-byte alignment.
#[repr(C)]
#[derive(Debug, Clone, Copy, Pod, Zeroable)]
pub struct WfHit {
    pub t: f32,             // offset 0
    pub instance_id: u32,   // offset 4
    pub _pad: [u32; 2],     // offset 8, padding to align normal to 16
    pub normal: [f32; 3],   // offset 16 (vec3<f32> requires 16-byte alignment)
    pub hit: u32,           // offset 28
}

