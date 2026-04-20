//! Serialize BVH + primitives into GPU storage buffers.

use super::bvh::{BvhNode, GpuInstance, GpuMaterial, Instance};
use super::build::Bvh;

// ---- Instance-based scene data ----

/// GPU scene data for instance-based path tracing (no triangles/materials).
pub struct GpuInstanceSceneData {
    pub nodes: Vec<BvhNode>,
    pub instances: Vec<GpuInstance>,
    pub materials: Vec<GpuMaterial>,
}

/// Build GPU data from BVH + instances (reordered by BVH leaf order).
pub fn build_instance_gpu_data(
    bvh: &Bvh,
    instances: &[Instance],
    materials: &[GpuMaterial],
) -> GpuInstanceSceneData {
    let gpu_instances: Vec<GpuInstance> = bvh
        .tri_indices
        .iter()
        .map(|&idx| instances[idx].to_gpu())
        .collect();

    GpuInstanceSceneData {
        nodes: bvh.nodes.clone(),
        instances: gpu_instances,
        materials: materials.to_vec(),
    }
}

impl GpuInstanceSceneData {
    pub fn nodes_bytes(&self) -> &[u8] {
        bytemuck::cast_slice(&self.nodes)
    }
    pub fn instances_bytes(&self) -> &[u8] {
        bytemuck::cast_slice(&self.instances)
    }
    pub fn materials_bytes(&self) -> &[u8] {
        bytemuck::cast_slice(&self.materials)
    }
}

/// Build GPU data from pre-built BVH nodes and sorted indices.
/// Used with GpuBvhBuilder output.
pub fn build_gpu_data_from_nodes(
    nodes: Vec<BvhNode>,
    sorted_indices: &[u32],
    instances: &[Instance],
    materials: &[GpuMaterial],
) -> GpuInstanceSceneData {
    let gpu_instances: Vec<GpuInstance> = sorted_indices
        .iter()
        .map(|&idx| instances[idx as usize].to_gpu())
        .collect();

    GpuInstanceSceneData {
        nodes,
        instances: gpu_instances,
        materials: materials.to_vec(),
    }
}
