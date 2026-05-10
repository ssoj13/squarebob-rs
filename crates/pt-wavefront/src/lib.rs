//! Wavefront path tracing pipelines.

pub mod wavefront;

pub use wavefront::{
    pack_tile_slots, WavefrontConfig, WavefrontPipeline, WfDims, WfHit, WfRay,
    DEFAULT_TILE_CAPACITY, MAX_TILE_CAPACITY, TILE_SLOT_STRIDE, WF_COUNTS_SIZE, WF_DIMS_SIZE,
};
