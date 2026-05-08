//! BVH node and primitive types for GPU path tracing.
//!
//! Flat array layout optimized for GPU traversal:
//! - 32-byte nodes (cache-line friendly)
//! - Triangles packed with vertex data for coherent access

use bytemuck::{Pod, Zeroable};
use glam::{Mat4, Vec3};

/// Axis-aligned bounding box.
#[derive(Debug, Clone, Copy)]
pub struct Aabb {
    pub min: [f32; 3],
    pub max: [f32; 3],
}

impl Aabb {
    pub const EMPTY: Self = Self {
        min: [f32::INFINITY; 3],
        max: [f32::NEG_INFINITY; 3],
    };

    /// Grow to include a point.
    #[inline]
    #[allow(clippy::needless_range_loop)]
    pub fn grow_point(&mut self, p: [f32; 3]) {
        for i in 0..3 {
            self.min[i] = self.min[i].min(p[i]);
            self.max[i] = self.max[i].max(p[i]);
        }
    }

    /// Grow to include another AABB.
    #[inline]
    pub fn grow(&mut self, other: &Aabb) {
        for i in 0..3 {
            self.min[i] = self.min[i].min(other.min[i]);
            self.max[i] = self.max[i].max(other.max[i]);
        }
    }

    /// Surface area (for SAH cost).
    #[inline]
    pub fn area(&self) -> f32 {
        let dx = self.max[0] - self.min[0];
        let dy = self.max[1] - self.min[1];
        let dz = self.max[2] - self.min[2];
        2.0 * (dx * dy + dy * dz + dz * dx)
    }

    /// Centroid of the AABB.
    #[inline]
    pub fn centroid(&self) -> [f32; 3] {
        [
            (self.min[0] + self.max[0]) * 0.5,
            (self.min[1] + self.max[1]) * 0.5,
            (self.min[2] + self.max[2]) * 0.5,
        ]
    }
}

/// GPU-friendly BVH node (32 bytes, matches WGSL struct).
///
/// Internal node: left_or_first = left child index, count = 0
/// Leaf node: left_or_first = first triangle index, count > 0
#[repr(C)]
#[derive(Debug, Clone, Copy, Pod, Zeroable)]
pub struct BvhNode {
    pub aabb_min: [f32; 3],
    pub left_or_first: u32,
    pub aabb_max: [f32; 3],
    pub count: u32,
}

/// Triangle primitive for GPU storage (112 bytes).
/// Packed: 3 vertices × (pos + normal) = 3 × 2 × vec3, plus material and object IDs.
#[repr(C)]
#[derive(Debug, Clone, Copy, Pod, Zeroable)]
pub struct GpuTriangle {
    pub v0: [f32; 3],
    pub material_id: u32,
    pub v1: [f32; 3],
    pub object_id: u32,
    pub v2: [f32; 3],
    pub _pad1: u32,
    pub n0: [f32; 3],
    pub _pad2: u32,
    pub n1: [f32; 3],
    pub _pad3: u32,
    pub n2: [f32; 3],
    pub _pad4: u32,
}

/// Standard Surface material params for GPU (144 bytes).
///
/// Matches StandardSurfaceParams layout from the rasterizer.
/// All colors use vec4 packing: rgb = color, a = weight.
#[repr(C)]
#[derive(Debug, Clone, Copy, Pod, Zeroable)]
pub struct GpuMaterial {
    /// Base color (rgb) and weight (a)
    pub base_color_weight: [f32; 4],
    /// Specular color (rgb) and weight (a)
    pub specular_color_weight: [f32; 4],
    /// Transmission color (rgb) and weight (a)
    pub transmission_color_weight: [f32; 4],
    /// Subsurface color (rgb) and weight (a)
    pub subsurface_color_weight: [f32; 4],
    /// Coat color (rgb) and weight (a)
    pub coat_color_weight: [f32; 4],
    /// Emission color (rgb) and weight (a)
    pub emission_color_weight: [f32; 4],
    /// Opacity (rgb), a unused
    pub opacity: [f32; 4],
    /// x=diffuse_roughness, y=metalness, z=specular_roughness, w=specular_IOR
    pub params1: [f32; 4],
    /// x=specular_anisotropy, y=coat_roughness, z=coat_IOR, w=visible (0=hidden, 1=visible)
    pub params2: [f32; 4],
}

// ---- Instance-based path tracing (ray-box intersection) ----

/// Unit cube corners for AABB computation.
const UNIT_CUBE_CORNERS: [Vec3; 8] = [
    Vec3::new(-0.5, -0.5, -0.5),
    Vec3::new(0.5, -0.5, -0.5),
    Vec3::new(0.5, 0.5, -0.5),
    Vec3::new(-0.5, 0.5, -0.5),
    Vec3::new(-0.5, -0.5, 0.5),
    Vec3::new(0.5, -0.5, 0.5),
    Vec3::new(0.5, 0.5, 0.5),
    Vec3::new(-0.5, 0.5, 0.5),
];

/// CPU-side cube instance for BVH building.
#[derive(Debug, Clone)]
pub struct Instance {
    pub model_inv: Mat4,
    pub color: [f32; 4],
    pub object_id: u32,
    pub material_id: u32,
    pub aabb: Aabb,
}

impl Instance {
    /// Create from a model matrix (T * S), color, and object_id.
    /// Computes world-space AABB from transformed unit cube corners.
    pub fn from_cube(model: Mat4, color: [f32; 4], object_id: u32, material_id: u32) -> Self {
        let mut aabb = Aabb::EMPTY;
        for &corner in &UNIT_CUBE_CORNERS {
            let wp = model.transform_point3(corner);
            aabb.grow_point(wp.to_array());
        }
        Self {
            model_inv: model.inverse(),
            color,
            object_id,
            material_id,
            aabb,
        }
    }

    /// Centroid of the instance AABB.
    pub fn centroid(&self) -> [f32; 3] {
        self.aabb.centroid()
    }
}

/// GPU-packed instance (96 bytes). Matches WGSL Instance struct.
#[repr(C)]
#[derive(Debug, Clone, Copy, Pod, Zeroable)]
pub struct GpuInstance {
    pub model_inv: [[f32; 4]; 4], // 64B
    pub color: [f32; 4],          // 16B
    pub object_id: u32,           //  4B
    pub material_id: u32,         //  4B
    pub _padding: [u32; 2],       //  8B
} // Total: 96B

/// GPU-friendly AABB (32 bytes).
#[repr(C)]
#[derive(Debug, Clone, Copy, Pod, Zeroable)]
pub struct GpuAabb {
    pub min: [f32; 4],
    pub max: [f32; 4],
}

impl Instance {
    /// Convert to GPU format.
    pub fn to_gpu(&self) -> GpuInstance {
        GpuInstance {
            model_inv: self.model_inv.to_cols_array_2d(),
            color: self.color,
            object_id: self.object_id,
            material_id: self.material_id,
            _padding: [0; 2],
        }
    }

    /// Convert instance AABB to GPU-friendly format.
    pub fn aabb_to_gpu(&self) -> GpuAabb {
        GpuAabb {
            min: [self.aabb.min[0], self.aabb.min[1], self.aabb.min[2], 0.0],
            max: [self.aabb.max[0], self.aabb.max[1], self.aabb.max[2], 0.0],
        }
    }
}
