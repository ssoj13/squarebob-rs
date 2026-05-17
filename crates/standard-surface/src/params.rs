//! Standard Surface material parameters
//!
//! Based on Autodesk Standard Surface specification:
//! https://autodesk.github.io/standard-surface/

use bytemuck::{Pod, Zeroable};
use glam::{Vec3, Vec4};
use serde::{Deserialize, Serialize};

/// Standard Surface material parameters
///
/// Maps directly to the WGSL uniform buffer layout.
/// All color values are linear (not sRGB).
/// 
/// Uses vec4 packing for proper GPU alignment:
/// - Colors use rgb, weight uses alpha
/// - Scalar params packed into vec4
#[repr(C)]
#[derive(Clone, Copy, Debug, Pod, Zeroable, Serialize, Deserialize)]
pub struct StandardSurfaceParams {
    /// Base color (rgb) and weight (a)
    pub base_color_weight: Vec4,
    /// Specular color (rgb) and weight (a)
    pub specular_color_weight: Vec4,
    /// Transmission color (rgb) and weight (a)
    pub transmission_color_weight: Vec4,
    /// Subsurface color (rgb) and weight (a)
    pub subsurface_color_weight: Vec4,
    /// Coat color (rgb) and weight (a)
    pub coat_color_weight: Vec4,
    /// Emission color (rgb) and weight (a)
    pub emission_color_weight: Vec4,
    /// Opacity (rgb), alpha unused
    pub opacity: Vec4,
    /// Packed params: x=diffuse_roughness, y=metalness, z=specular_roughness, w=specular_IOR
    pub params1: Vec4,
    /// Packed params: x=specular_anisotropy, y=coat_roughness, z=coat_IOR, w=unused
    pub params2: Vec4,
}

impl Default for StandardSurfaceParams {
    fn default() -> Self {
        Self {
            base_color_weight: Vec4::new(0.8, 0.8, 0.8, 1.0),
            specular_color_weight: Vec4::new(1.0, 1.0, 1.0, 1.0),
            transmission_color_weight: Vec4::new(1.0, 1.0, 1.0, 0.0),
            subsurface_color_weight: Vec4::new(1.0, 1.0, 1.0, 0.0),
            coat_color_weight: Vec4::new(1.0, 1.0, 1.0, 0.0),
            emission_color_weight: Vec4::new(1.0, 1.0, 1.0, 0.0),
            opacity: Vec4::new(1.0, 1.0, 1.0, 1.0),
            // x=diffuse_roughness, y=metalness, z=specular_roughness, w=specular_IOR
            params1: Vec4::new(0.0, 0.0, 0.2, 1.5),
            // x=specular_anisotropy, y=coat_roughness, z=coat_IOR, w=unused
            params2: Vec4::new(0.0, 0.1, 1.5, 0.0),
        }
    }
}

impl StandardSurfaceParams {
    /// Create a simple diffuse material
    pub fn diffuse(color: Vec3) -> Self {
        Self {
            base_color_weight: color.extend(1.0),
            specular_color_weight: Vec4::new(1.0, 1.0, 1.0, 0.0), // disable specular
            ..Self::default()
        }
    }

    /// Create a plastic-like material
    pub fn plastic(color: Vec3, roughness: f32) -> Self {
        let mut p = Self {
            base_color_weight: color.extend(1.0),
            ..Self::default()
        };
        p.params1.z = roughness; // specular_roughness
        p
    }

    /// Create a metallic material
    pub fn metal(color: Vec3, roughness: f32) -> Self {
        let mut p = Self {
            base_color_weight: color.extend(1.0),
            ..Self::default()
        };
        p.params1.y = 1.0; // metalness
        p.params1.z = roughness; // specular_roughness
        p
    }

    /// Create a glass-like material
    pub fn glass(color: Vec3, ior: f32) -> Self {
        let mut p = Self::default();
        p.base_color_weight.w = 0.0; // disable base
        p.transmission_color_weight = color.extend(1.0);
        p.params1.w = ior; // specular_IOR
        p.params1.z = 0.0; // specular_roughness
        p
    }

    /// Create an emissive material
    pub fn emissive(color: Vec3, intensity: f32) -> Self {
        let mut p = Self::default();
        p.base_color_weight.w = 0.0;
        p.specular_color_weight.w = 0.0;
        p.emission_color_weight = color.extend(intensity);
        p
    }

    /// Add coat layer
    pub fn with_coat(mut self, weight: f32, roughness: f32) -> Self {
        self.coat_color_weight.w = weight;
        self.params2.y = roughness;
        self
    }

    /// Set opacity
    pub fn with_opacity(mut self, opacity: f32) -> Self {
        self.opacity = Vec4::splat(opacity);
        self
    }
    
    // Convenience setters
    
    /// Set base color
    pub fn set_base_color(&mut self, color: Vec3) {
        self.base_color_weight.x = color.x;
        self.base_color_weight.y = color.y;
        self.base_color_weight.z = color.z;
    }
    
    /// Set base weight
    pub fn set_base(&mut self, weight: f32) {
        self.base_color_weight.w = weight;
    }
    
    /// Set metalness
    pub fn set_metalness(&mut self, metalness: f32) {
        self.params1.y = metalness;
    }
    
    /// Set specular roughness
    pub fn set_roughness(&mut self, roughness: f32) {
        self.params1.z = roughness;
    }
    
    /// Set specular weight
    pub fn set_specular(&mut self, weight: f32) {
        self.specular_color_weight.w = weight;
    }
}

/// Camera uniform data
#[repr(C)]
#[derive(Clone, Copy, Debug, Pod, Zeroable)]
pub struct CameraUniform {
    /// Combined view-projection matrix
    pub view_proj: [[f32; 4]; 4],
    /// View matrix only
    pub view: [[f32; 4]; 4],
    /// Inverse view-projection (for screen-space reconstruction)
    pub inv_view_proj: [[f32; 4]; 4],
    /// Camera world position
    pub position: Vec3,
    /// X-Ray mode alpha override (1.0 = normal, 0.5 = x-ray)
    pub xray_alpha: f32,
    /// Flat shading mode (1.0 = flat, 0.0 = smooth)
    pub flat_shading: f32,
    /// Auto-flip normals for backfaces (1.0 = enabled, 0.0 = disabled)
    pub auto_normals: f32,
    /// Padding for alignment
    pub _pad2: f32,
    pub _pad3: f32,
}

/// Directional light
#[repr(C)]
#[derive(Clone, Copy, Debug, Pod, Zeroable)]
pub struct Light {
    /// Light direction (normalized, pointing toward light source)
    pub direction: Vec3,
    pub _pad1: f32,
    /// Light color (linear RGB)
    pub color: Vec3,
    /// Light intensity
    pub intensity: f32,
}

impl Light {
    pub fn new(direction: Vec3, color: Vec3, intensity: f32) -> Self {
        Self {
            direction: direction.normalize(),
            _pad1: 0.0,
            color,
            intensity,
        }
    }
    
    /// Create a light pointing in direction
    pub fn directional(dir: Vec3, intensity: f32) -> Self {
        Self::new(dir, Vec3::ONE, intensity)
    }
    
    /// Disabled light
    pub fn off() -> Self {
        Self {
            direction: Vec3::NEG_Y,
            _pad1: 0.0,
            color: Vec3::ZERO,
            intensity: 0.0,
        }
    }
}

impl Default for Light {
    fn default() -> Self {
        Self::directional(Vec3::new(0.0, -1.0, -1.0), 1.0)
    }
}

/// 3-point lighting rig (key, fill, rim)
#[repr(C)]
#[derive(Clone, Copy, Debug, Pod, Zeroable)]
pub struct LightRig {
    /// Key light - main light, typically 45° above and to the side
    pub key: Light,
    /// Fill light - softer light from opposite side
    pub fill: Light,
    /// Rim light - back light for edge definition
    pub rim: Light,
    /// Ambient light color
    pub ambient: Vec3,
    pub _pad: f32,
}

impl Default for LightRig {
    fn default() -> Self {
        Self::three_point()
    }
}

impl LightRig {
    /// Classic 3-point lighting setup
    pub fn three_point() -> Self {
        Self {
            // Key: 45° front-right, above
            key: Light::new(
                Vec3::new(-0.5, -0.7, -0.5),
                Vec3::new(1.0, 0.98, 0.95), // Slightly warm
                1.2,
            ),
            // Fill: 45° front-left, at eye level (softer)
            fill: Light::new(
                Vec3::new(0.6, -0.3, -0.6),
                Vec3::new(0.9, 0.95, 1.0), // Slightly cool
                0.4,
            ),
            // Rim: from behind, above
            rim: Light::new(
                Vec3::new(0.0, -0.5, 0.8),
                Vec3::new(1.0, 1.0, 1.0),
                0.6,
            ),
            ambient: Vec3::splat(0.08),
            _pad: 0.0,
        }
    }
    
    /// Simple single key light
    pub fn single_key() -> Self {
        Self {
            key: Light::new(
                Vec3::new(-0.5, -0.8, -0.3),
                Vec3::ONE,
                1.0,
            ),
            fill: Light::off(),
            rim: Light::off(),
            ambient: Vec3::splat(0.15),
            _pad: 0.0,
        }
    }
    
    /// Studio lighting (flat, even)
    pub fn studio() -> Self {
        Self {
            key: Light::new(
                Vec3::new(0.0, -1.0, 0.0),
                Vec3::ONE,
                0.8,
            ),
            fill: Light::new(
                Vec3::new(0.0, 0.0, -1.0),
                Vec3::ONE,
                0.5,
            ),
            rim: Light::off(),
            ambient: Vec3::splat(0.2),
            _pad: 0.0,
        }
    }
    
    /// Dramatic lighting (high contrast)
    pub fn dramatic() -> Self {
        Self {
            key: Light::new(
                Vec3::new(-0.8, -0.5, -0.2),
                Vec3::new(1.0, 0.9, 0.8),
                1.5,
            ),
            fill: Light::off(),
            rim: Light::new(
                Vec3::new(0.3, -0.2, 0.9),
                Vec3::new(0.8, 0.9, 1.0),
                0.8,
            ),
            ambient: Vec3::splat(0.03),
            _pad: 0.0,
        }
    }
}

// Legacy alias
pub type LightUniform = Light;

/// Model transform uniform
#[repr(C)]
#[derive(Clone, Copy, Debug, Pod, Zeroable)]
pub struct ModelUniform {
    /// Model matrix (world transform)
    pub model: [[f32; 4]; 4],
    /// Normal matrix (inverse transpose of model)
    pub normal_matrix: [[f32; 4]; 4],
    /// Object ID for picking/hover (0 = background/none)
    pub object_id: u32,
    /// Padding to 16-byte alignment
    pub _pad: [u32; 3],
}

impl Default for ModelUniform {
    fn default() -> Self {
        let identity = [
            [1.0, 0.0, 0.0, 0.0],
            [0.0, 1.0, 0.0, 0.0],
            [0.0, 0.0, 1.0, 0.0],
            [0.0, 0.0, 0.0, 1.0],
        ];
        Self {
            model: identity,
            normal_matrix: identity,
            object_id: 0,
            _pad: [0; 3],
        }
    }
}

/// Shadow mapping uniform
#[repr(C)]
#[derive(Clone, Copy, Debug, Pod, Zeroable)]
pub struct ShadowUniform {
    /// Light view-projection matrix
    pub light_view_proj: [[f32; 4]; 4],
}

impl Default for ShadowUniform {
    fn default() -> Self {
        Self {
            light_view_proj: [
                [1.0, 0.0, 0.0, 0.0],
                [0.0, 1.0, 0.0, 0.0],
                [0.0, 0.0, 1.0, 0.0],
                [0.0, 0.0, 0.0, 1.0],
            ],
        }
    }
}
