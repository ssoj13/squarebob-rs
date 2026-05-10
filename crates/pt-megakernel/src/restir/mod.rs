//! ReSTIR (Reservoir-based Spatiotemporal Importance Resampling) module.
//!
//! Provides massive quality improvements at low SPP (1-4) through:
//! - Initial candidate sampling with target function
//! - Temporal resampling (reuse previous frames)
//! - Spatial resampling (reuse neighbor pixels)
//!
//! Based on: "Spatiotemporal reservoir resampling for real-time ray tracing
//! with dynamic direct lighting" (Bitterli et al., 2020)

mod config;
mod pipeline;
mod reservoir;

// Infrastructure for ReSTIR - in development
#[allow(unused_imports)]
pub use config::ReSTIRConfig;
#[allow(unused_imports)]
pub use pipeline::{
    ReSTIRPipeline, RESTIR_INITIAL_PARAMS_SIZE, RESTIR_SHADE_PARAMS_SIZE,
    RESTIR_SPATIAL_PARAMS_SIZE, RESTIR_TEMPORAL_PARAMS_SIZE,
};
#[allow(unused_imports)]
pub use reservoir::{Reservoir, Sample};
