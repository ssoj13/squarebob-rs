//! Path Guiding module using Sparse Voxel Octree (SVO).
//!
//! Stores incident radiance distribution in 3D grid.
//! Used to guide sampling toward high-energy directions.
//!
//! Based on: "Practical Path Guiding for Efficient Light-Transport Simulation"
//! (Muller et al., 2017)

mod config;
mod pipeline;
mod svo;

#[allow(unused_imports)]
pub use config::PathGuideConfig;
#[allow(unused_imports)]
pub use pipeline::PathGuidePipeline;
#[allow(unused_imports)]
pub use svo::{SvoConfig, SvoNode};
