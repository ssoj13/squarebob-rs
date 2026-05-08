//! Shared path tracing core types and CPU BVH builder.

pub mod build;
pub mod bvh;
pub mod gpu_data;

pub use build::build_instance_bvh;
pub use bvh::{BvhNode, GpuAabb, GpuMaterial, Instance};
pub use gpu_data::{build_gpu_data_from_nodes, build_instance_gpu_data};
