//! Sparse Voxel Octree for radiance storage.

use bytemuck::{Pod, Zeroable};

/// SVO node storing child pointers and radiance.
/// Each octree node has 8 children and stores accumulated radiance.
#[repr(C)]
#[derive(Debug, Clone, Copy, Pod, Zeroable)]
pub struct SvoNode {
    /// Child indices (0 = empty, non-zero = valid)
    pub children: [u32; 8],
    /// Accumulated radiance for this voxel
    pub radiance: [f32; 3],
    /// Sample count for averaging
    pub count: u32,
}

impl SvoNode {
    pub const SIZE: usize = std::mem::size_of::<Self>();
}

/// SVO configuration.
#[allow(dead_code)]
#[derive(Debug, Clone)]
pub struct SvoConfig {
    /// Grid resolution (power of 2, e.g. 64)
    pub resolution: u32,
    /// Maximum octree depth
    pub max_depth: u32,
    /// Decay factor for temporal smoothing (0-1)
    pub decay: f32,
}

impl Default for SvoConfig {
    fn default() -> Self {
        Self {
            resolution: 64,
            max_depth: 6, // 2^6 = 64
            decay: 0.95,
        }
    }
}
