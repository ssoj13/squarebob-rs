//! Wavefront path tracing pipelines.

pub mod wavefront;

pub use wavefront::{
    WavefrontConfig, WavefrontPipeline, WfDims, WfHit, WfRay, TILE_SLOT_STRIDE, WF_COUNTS_SIZE,
    WF_DIMS_SIZE,
};
