//! Wavefront path tracing pipelines.

pub mod wavefront;

pub use wavefront::{
    DEFAULT_TILE_CAPACITY, MAX_TILE_CAPACITY, TILE_SLOT_STRIDE, WF_COUNTS_SIZE, WF_DIMS_SIZE,
    WavefrontConfig, WavefrontPipeline, WfDims, WfHit, WfRay, pack_tile_slots,
};
