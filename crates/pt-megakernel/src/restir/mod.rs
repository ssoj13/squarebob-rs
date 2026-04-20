//! ReSTIR (Reservoir-based Spatiotemporal Importance Resampling) module.
//!
//! Provides massive quality improvements at low SPP (1-4) through:
//! - Initial candidate sampling with target function
//! - Temporal resampling (reuse previous frames)
//! - Spatial resampling (reuse neighbor pixels)
//!
//! Based on: "Spatiotemporal reservoir resampling for real-time ray tracing
//! with dynamic direct lighting" (Bitterli et al., 2020)

mod reservoir;
mod config;
mod pipeline;

// Infrastructure for ReSTIR - in development
#[allow(unused_imports)]
pub use reservoir::{Reservoir, Sample};
#[allow(unused_imports)]
pub use config::ReSTIRConfig;
#[allow(unused_imports)]
pub use pipeline::ReSTIRPipeline;
