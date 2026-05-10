//! Wavefront Path Tracing module.
//!
//! Splits megakernel into separate passes for better GPU occupancy:
//! - Ray Generation: Generate camera rays
//! - Intersection: BVH traversal for all rays
//! - Shading: Evaluate BSDF, generate next rays
//! - Compaction: Remove terminated rays

mod buffers;
mod config;
mod pipeline;

// Infrastructure for wavefront PT - currently in development
#[allow(unused_imports)]
pub use buffers::{WfHit, WfRay};
#[allow(unused_imports)]
pub use config::WavefrontConfig;
#[allow(unused_imports)]
pub use pipeline::WavefrontPipeline;
#[allow(unused_imports)]
pub use pipeline::{
    pack_tile_slots, WfDims, DEFAULT_TILE_CAPACITY, MAX_TILE_CAPACITY, TILE_SLOT_STRIDE,
    WF_COUNTS_SIZE, WF_DIMS_SIZE,
};
