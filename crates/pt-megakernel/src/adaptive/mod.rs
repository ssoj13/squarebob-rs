//! Adaptive Sampling module.
//!
//! Estimates variance per pixel and allocates more samples
//! to high-variance (noisy) regions.
//!
//! Uses running mean/variance estimation with Welford's algorithm.

mod config;
mod pipeline;

#[allow(unused_imports)]
pub use config::AdaptiveConfig;
pub(crate) use pipeline::VARIANCE_DATA_WGSL;
#[allow(unused_imports)]
pub use pipeline::{AdaptivePipeline, VarianceData};
