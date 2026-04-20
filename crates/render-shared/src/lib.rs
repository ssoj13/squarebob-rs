/// Renderer abstraction: CPU (rayon) or GPU (wgpu) backends.
use bytemuck::{Pod, Zeroable};
use glam::{Mat4, Vec3};
use serde::{Deserialize, Serialize};
use pt_mats::{MaterialClass, MaterializeMode, MaterialSource, MaterialDistribution};

/// Available rendering backends
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
pub enum RenderBackend {
    #[default]
    Cpu,
    Gpu,
}

impl RenderBackend {
    pub fn name(&self) -> &'static str {
        match self {
            RenderBackend::Cpu => "CPU (Rayon)",
            RenderBackend::Gpu => "GPU (wgpu)",
        }
    }
    
    pub fn all() -> &'static [RenderBackend] {
        &[RenderBackend::Cpu, RenderBackend::Gpu]
    }
}

/// Render mode: 2D treemap or 3D cubes
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
pub enum RenderMode {
    #[default]
    Mode2D,
    Mode3D,
}

#[allow(dead_code)]
impl RenderMode {
    pub fn name(&self) -> &'static str {
        match self {
            RenderMode::Mode2D => "2D",
            RenderMode::Mode3D => "3D",
        }
    }
    
    pub fn all() -> &'static [RenderMode] {
        &[RenderMode::Mode2D, RenderMode::Mode3D]
    }
}

/// What determines cube height in 3D mode
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
pub enum CubeHeightMode {
    #[default]
    FileSize,
    OwnSize,
    FileCount,
    DirCount,
    Age,
    Depth,
    DepthSquared,
    Constant,
}

impl CubeHeightMode {
    pub fn name(&self) -> &'static str {
        match self {
            CubeHeightMode::FileSize => "File Size",
            CubeHeightMode::OwnSize => "Own Size",
            CubeHeightMode::FileCount => "File Count",
            CubeHeightMode::DirCount => "Dir Count",
            CubeHeightMode::Age => "Age",
            CubeHeightMode::Depth => "Depth",
            CubeHeightMode::DepthSquared => "Depth^2",
            CubeHeightMode::Constant => "Constant",
        }
    }
}

/// Color mode for 3D cubes
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
pub enum ColorMode {
    #[default]
    Treemap,      // Use treemap-assigned colors (depth-based)
    FileType,     // Color by file extension category
    FileAge,      // Color by modification time (old->new gradient)
    FileSize,     // Color by file size (small->large gradient)
    Depth,        // Color by directory depth (rainbow gradient)
}

impl ColorMode {
    pub fn name(&self) -> &'static str {
        match self {
            ColorMode::Treemap => "Treemap",
            ColorMode::FileType => "File Type",
            ColorMode::FileAge => "File Age",
            ColorMode::FileSize => "File Size",
            ColorMode::Depth => "Depth",
        }
    }
    pub fn all() -> &'static [ColorMode] {
        &[
            ColorMode::Treemap,
            ColorMode::FileType,
            ColorMode::FileAge,
            ColorMode::FileSize,
            ColorMode::Depth,
        ]
    }
}

/// Folder tint color source for files
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
pub enum FolderColorMode {
    #[default]
    Depth,      // Depth-based rainbow gradient
    NameHash,   // Hash of folder name
    PathHash,   // Hash of full folder path
}

impl FolderColorMode {
    pub fn name(&self) -> &'static str {
        match self {
            FolderColorMode::Depth => "Depth",
            FolderColorMode::NameHash => "Name",
            FolderColorMode::PathHash => "Path",
        }
    }
    pub fn all() -> &'static [FolderColorMode] {
        &[
            FolderColorMode::Depth,
            FolderColorMode::NameHash,
            FolderColorMode::PathHash,
        ]
    }
}

/// Adaptive sampling preset (UI helper).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
pub enum AdaptivePreset {
    #[default]
    Custom,
    Conservative,
    Balanced,
    Aggressive,
}

impl AdaptivePreset {
    pub fn name(&self) -> &'static str {
        match self {
            AdaptivePreset::Custom => "Custom",
            AdaptivePreset::Conservative => "Low",
            AdaptivePreset::Balanced => "Medium",
            AdaptivePreset::Aggressive => "High",
        }
    }
    pub fn all() -> &'static [AdaptivePreset] {
        &[
            AdaptivePreset::Custom,
            AdaptivePreset::Conservative,
            AdaptivePreset::Balanced,
            AdaptivePreset::Aggressive,
        ]
    }
}

/// Spectral rendering mode (PT only).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
pub enum SpectralMode {
    #[default]
    Off,
    Hero,
    Multi,
}

impl SpectralMode {
    pub fn name(&self) -> &'static str {
        match self {
            SpectralMode::Off => "Off",
            SpectralMode::Hero => "Hero",
            SpectralMode::Multi => "Multi",
        }
    }

    pub fn all() -> &'static [SpectralMode] {
        &[
            SpectralMode::Off,
            SpectralMode::Hero,
            SpectralMode::Multi,
        ]
    }
}

/// Preset glass variants for global PT transparency
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
pub enum GlassPreset {
    #[default]
    Clear,
    Blue,
    Green,
    Amber,
    Pink,
}

impl GlassPreset {
    pub fn name(self) -> &'static str {
        match self {
            GlassPreset::Clear => "Clear",
            GlassPreset::Blue => "Blue",
            GlassPreset::Green => "Green",
            GlassPreset::Amber => "Amber",
            GlassPreset::Pink => "Pink",
        }
    }

    pub fn all() -> &'static [GlassPreset] {
        &[
            GlassPreset::Clear,
            GlassPreset::Blue,
            GlassPreset::Green,
            GlassPreset::Amber,
            GlassPreset::Pink,
        ]
    }

    pub fn to_material_class(self) -> MaterialClass {
        match self {
            GlassPreset::Clear => MaterialClass::GlassClear,
            GlassPreset::Blue => MaterialClass::GlassBlue,
            GlassPreset::Green => MaterialClass::GlassGreen,
            GlassPreset::Amber => MaterialClass::GlassAmber,
            GlassPreset::Pink => MaterialClass::GlassPink,
        }
    }
}

/// Color gradient for depth (rainbow: red->orange->yellow->green->cyan->blue->magenta)
pub fn color_for_depth(depth: u32, max_depth: u32) -> [f32; 4] {
    let t = if max_depth > 0 { (depth as f32 / max_depth as f32).clamp(0.0, 1.0) } else { 0.0 };
    // HSV-like rainbow: hue from 0 (red) to 270 (violet)
    let hue = t * 270.0;
    let (r, g, b) = hsv_to_rgb(hue, 0.8, 0.9);
    [r, g, b, 1.0]
}

/// Convert HSV (hue 0-360, sat/val 0-1) to RGB
fn hsv_to_rgb(h: f32, s: f32, v: f32) -> (f32, f32, f32) {
    let c = v * s;
    let x = c * (1.0 - ((h / 60.0) % 2.0 - 1.0).abs());
    let m = v - c;
    let (r, g, b) = match (h / 60.0) as u32 {
        0 => (c, x, 0.0),
        1 => (x, c, 0.0),
        2 => (0.0, c, x),
        3 => (0.0, x, c),
        4 => (x, 0.0, c),
        _ => (c, 0.0, x),
    };
    (r + m, g + m, b + m)
}

pub fn color_for_hash(hash: u32) -> [f32; 4] {
    let h = (hash as f32) / (u32::MAX as f32);
    let hue = h * 360.0;
    let (r, g, b) = hsv_to_rgb(hue, 0.55, 0.90);
    [r, g, b, 1.0]
}

/// Get color for file type based on extension
pub fn color_for_extension(ext: &str) -> [f32; 4] {
    let ext_lower = ext.to_lowercase();
    match ext_lower.as_str() {
        // Code files - blue family
        "rs" | "py" | "js" | "ts" | "c" | "cpp" | "h" | "hpp" | "java" | "go" | "rb" | "php" | "swift" | "kt" =>
            [0.3, 0.5, 0.9, 1.0],
        // Web files - orange
        "html" | "htm" | "css" | "scss" | "sass" | "vue" | "jsx" | "tsx" =>
            [0.95, 0.6, 0.2, 1.0],
        // Data files - green
        "json" | "xml" | "yaml" | "yml" | "toml" | "csv" | "sql" =>
            [0.4, 0.8, 0.4, 1.0],
        // Documents - warm yellow
        "md" | "txt" | "doc" | "docx" | "pdf" | "rtf" | "odt" =>
            [0.95, 0.85, 0.4, 1.0],
        // DCC scene files - distinct per DCC
        "mb" =>
            [0.85, 0.65, 0.25, 1.0],
        "hou" =>
            [0.9, 0.5, 0.15, 1.0],
        // HDR images - cyan/teal
        "exr" =>
            [0.2, 0.8, 0.9, 1.0],
        // Film/RAW images - distinct per format
        "dpx" =>
            [0.6, 0.45, 0.95, 1.0],
        "raf" =>
            [0.45, 0.75, 0.35, 1.0],
        "nef" =>
            [0.35, 0.55, 0.9, 1.0],
        // Images - purple/magenta (keep TIFF/TIF distinct)
        "tif" | "tiff" =>
            [0.75, 0.35, 0.7, 1.0],
        "png" | "jpg" | "jpeg" | "gif" | "bmp" | "svg" | "webp" | "ico" =>
            [0.8, 0.4, 0.8, 1.0],
        // Audio - cyan
        "mp3" | "wav" | "ogg" | "flac" | "aac" | "m4a" =>
            [0.3, 0.8, 0.85, 1.0],
        // Video - red
        "mp4" | "avi" | "mkv" | "mov" | "wmv" | "flv" | "webm" =>
            [0.9, 0.3, 0.3, 1.0],
        // Archives - brown
        "zip" | "tar" | "gz" | "7z" | "rar" | "bz2" | "xz" =>
            [0.7, 0.5, 0.3, 1.0],
        // Executables - dark red
        "exe" | "dll" | "so" | "dylib" | "bin" | "app" =>
            [0.7, 0.2, 0.2, 1.0],
        // Config files - teal
        "ini" | "conf" | "cfg" | "env" | "lock" =>
            [0.3, 0.7, 0.65, 1.0],
        // Default - gray
        _ => [0.6, 0.6, 0.6, 1.0],
    }
}

/// Get color for file age (heat map: 0.0 = newest/hot, 1.0 = oldest/cold)
pub fn color_for_age(normalized_age: f32) -> [f32; 4] {
    // Vibrant heat map: Magenta (recent) -> Red -> Orange -> Yellow -> Cyan -> Blue (old)
    let t = (1.0 - normalized_age).clamp(0.0, 1.0);
    if t < 0.2 {
        // Old files: Deep Blue
        let s = t / 0.2;
        [0.1, 0.2 + s * 0.3, 0.95, 1.0]
    } else if t < 0.4 {
        // Blue -> Cyan
        let s = (t - 0.2) / 0.2;
        [0.1, 0.5 + s * 0.45, 0.95 - s * 0.35, 1.0]
    } else if t < 0.6 {
        // Cyan -> Yellow/Green
        let s = (t - 0.4) / 0.2;
        [0.1 + s * 0.85, 0.95, 0.6 - s * 0.5, 1.0]
    } else if t < 0.8 {
        // Yellow -> Orange
        let s = (t - 0.6) / 0.2;
        [0.95, 0.95 - s * 0.45, 0.1, 1.0]
    } else {
        // Recent files: Orange -> Red/Magenta
        let s = (t - 0.8) / 0.2;
        [1.0, 0.5 - s * 0.3, 0.1 + s * 0.5, 1.0]
    }
}

/// Get color for file size (0.0 = smallest, 1.0 = largest)
pub fn color_for_size(normalized_size: f32) -> [f32; 4] {
    // Green (small) -> Yellow -> Orange -> Red (large) - distinct from age gradient
    let t = normalized_size.clamp(0.0, 1.0);
    if t < 0.33 {
        // Green -> Yellow-green
        let s = t / 0.33;
        [0.2 + s * 0.6, 0.8, 0.2, 1.0]
    } else if t < 0.66 {
        // Yellow-green -> Orange
        let s = (t - 0.33) / 0.33;
        [0.8 + s * 0.15, 0.8 - s * 0.35, 0.2, 1.0]
    } else {
        // Orange -> Red
        let s = (t - 0.66) / 0.34;
        [0.95, 0.45 - s * 0.35, 0.2, 1.0]
    }
}

/// Hash-based transform effect for cubes
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
pub enum HashTransformEffect {
    #[default]
    None,
    Wave,           // Sine wave based on hash
    RandomHeight,   // Pulsing random height
    RandomOffset,   // Drifting 3D offset
    Explode,        // Pulsing explosion outward
    Noise,          // Smooth noise drift
    Pulse,          // Radial breathing
    Spiral,         // Spiral swirl around center
    Ocean,          // Large slow waves like ocean surface
    Rotate3D,       // 3D rotation around center
    Twist,          // Twisting tower effect
    Breathe,        // Synchronized breathing
    Swarm,          // Insect swarm movement
    Earthquake,     // Shaking/trembling
    Ripple,         // Concentric ripples from center
    Vortex,         // Rotating vortex pulling inward
    Glitch,         // Digital glitch displacement
    Echo,          // Pulsing outward bloom
}

impl HashTransformEffect {
    pub fn name(&self) -> &'static str {
        match self {
            HashTransformEffect::None => "None",
            HashTransformEffect::Wave => "Wave",
            HashTransformEffect::RandomHeight => "Random Height",
            HashTransformEffect::RandomOffset => "Random Offset",
            HashTransformEffect::Explode => "Explode",
            HashTransformEffect::Noise => "Noise",
            HashTransformEffect::Pulse => "Pulse",
            HashTransformEffect::Spiral => "Spiral",
            HashTransformEffect::Ocean => "Ocean",
            HashTransformEffect::Rotate3D => "Rotate 3D",
            HashTransformEffect::Twist => "Twist",
            HashTransformEffect::Breathe => "Breathe",
            HashTransformEffect::Swarm => "Swarm",
            HashTransformEffect::Earthquake => "Earthquake",
            HashTransformEffect::Ripple => "Ripple",
            HashTransformEffect::Vortex => "Vortex",
            HashTransformEffect::Glitch => "Glitch",
            HashTransformEffect::Echo => "Echo",
        }
    }

    pub fn all() -> &'static [HashTransformEffect] {
        &[
            HashTransformEffect::None,
            HashTransformEffect::Wave,
            HashTransformEffect::RandomHeight,
            HashTransformEffect::RandomOffset,
            HashTransformEffect::Explode,
            HashTransformEffect::Noise,
            HashTransformEffect::Pulse,
            HashTransformEffect::Spiral,
            HashTransformEffect::Ocean,
            HashTransformEffect::Rotate3D,
            HashTransformEffect::Twist,
            HashTransformEffect::Breathe,
            HashTransformEffect::Swarm,
            HashTransformEffect::Earthquake,
            HashTransformEffect::Ripple,
            HashTransformEffect::Vortex,
            HashTransformEffect::Glitch,
            HashTransformEffect::Echo,
        ]
    }
}


/// Hover highlight mode
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
pub enum HoverMode {
    #[default]
    None,
    Outline,
    Tint,
    Both,
}

impl HoverMode {
    pub fn name(&self) -> &'static str {
        match self {
            HoverMode::None => "None",
            HoverMode::Outline => "Outline",
            HoverMode::Tint => "Tint",
            HoverMode::Both => "Both",
        }
    }

    pub fn all() -> &'static [HoverMode] {
        &[HoverMode::None, HoverMode::Outline, HoverMode::Tint, HoverMode::Both]
    }

    /// WGSL mode value: 0=none, 1=outline, 2=tint, 3=both
    pub fn to_u32(self) -> u32 {
        match self {
            HoverMode::None => 0,
            HoverMode::Outline => 1,
            HoverMode::Tint => 2,
            HoverMode::Both => 3,
        }
    }
}

fn default_animation_speed() -> f32 { 1.0 }

/// Options for 3D rendering
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct Render3DOptions {
    pub height_mode: CubeHeightMode,
    #[serde(default)]
    pub height_power_enabled: bool,
    #[serde(default = "default_height_power")]
    pub height_power: f32,
    pub height_scale: f32,
    pub color_mode: ColorMode,
    #[serde(default)]
    pub folder_color_mode: FolderColorMode,
    #[serde(default = "default_folder_tint")]
    pub folder_tint: f32,
    pub hash_effect: HashTransformEffect,
    pub hash_effect_strength: f32,
    pub animation_time: f32,
    #[serde(default = "default_animation_speed")]
    pub animation_speed: f32,
    pub animate: bool,
    pub show_wireframe: bool,
    pub hover_mode: HoverMode,
    pub hover_outline_width: f32,
    pub hover_outline_alpha: f32,
    pub roughness: f32,
    pub metalness: f32,
    pub specular_ior: f32,
    pub xray_alpha: f32,
    pub flat_shading: bool,
    pub double_sided: bool,
    pub materialize_mode: MaterializeMode,  // Legacy, kept for compatibility
    #[serde(default)]
    pub mat_source: MaterialSource,
    #[serde(default)]
    pub mat_distribution: MaterialDistribution,
    #[serde(default = "default_quant_levels")]
    pub mat_quant_levels: u32,
    #[serde(default = "default_band_count")]
    pub mat_band_count: u32,
    #[serde(default = "default_spatial_scale")]
    pub mat_spatial_scale: f32,
    #[serde(default = "default_materialize_mix")]
    pub materialize_mix: f32,     // 0=use color_mode, 1=use materialize color
    #[serde(default = "default_true")]
    pub mat_allow_lights: bool,  // Allow emissive/neon materials
    #[serde(default = "default_prob")]
    pub mat_light_prob: f32,     // Probability of assigning light material (0.0-1.0)
    #[serde(default = "default_light_warm")]
    pub mat_light_warm: f32,     // Warm light bias (0-1)
    #[serde(default = "default_light_cool")]
    pub mat_light_cool: f32,     // Cool light bias (0-1)
    #[serde(default = "default_light_intensity")]
    pub mat_light_intensity: f32, // Global light intensity multiplier
    #[serde(default = "default_light_color_randomness")]
    pub mat_light_color_randomness: f32, // Per-light color randomness (0-1)
    #[serde(default = "default_false")]
    pub mat_allow_glass: bool,   // Allow glass/transparent materials
    #[serde(default = "default_prob")]
    pub mat_glass_prob: f32,     // Probability of assigning glass material (0.0-1.0)
    #[serde(default)]
    pub mat_include_dirs: bool,  // Allow materialization for directories
    #[serde(default = "default_mat_seed")]
    pub mat_seed: u32,           // Seed for random material assignment
    #[serde(default = "default_transparency")]
    pub pt_global_transparency: f32, // 0=opaque, 1=all glass
    #[serde(default)]
    pub pt_global_glass: GlassPreset,
    #[serde(default = "default_glass_specular")]
    pub pt_glass_specular: f32,
    #[serde(default = "default_glass_base")]
    pub pt_glass_base: f32,
    #[serde(default = "default_glass_roughness")]
    pub pt_glass_roughness: f32,
    #[serde(default = "default_glass_ior")]
    pub pt_glass_ior: f32,
    #[serde(default = "default_glass_dispersion")]
    pub pt_glass_dispersion: f32,
    #[serde(default = "default_glass_temp")]
    pub pt_glass_temp: f32,
    #[serde(default = "default_false")]
    pub pt_glass_thin: bool,
    pub env_map_intensity: f32,
    pub env_map_rotation: f32,
    pub env_map_enabled: bool,
    pub env_map_visible: bool,
    pub env_map_path: Option<std::path::PathBuf>,
    pub env_animate: bool,
    #[serde(default = "default_env_speed")]
    pub env_speed: f32,
    pub background_color: [f32; 3],
    // Path tracing
    pub path_tracing: bool,
    pub pt_max_bounces: u32,
    pub pt_max_samples: u32,
    pub pt_samples_per_update: u32,
    pub pt_max_transmission_depth: u32,
    pub pt_dof_enabled: bool,
    pub pt_aperture: f32,
    pub pt_focus_distance: f32,
    pub pt_env_importance_sampling: bool,
    pub pt_target_fps: f32,
    pub pt_auto_spp: bool,
    pub pt_camera_snap: bool,
    #[serde(default)]
    pub pt_spectral_mode: SpectralMode,
    #[serde(default = "default_spectral_samples")]
    pub pt_spectral_samples: u32,
    #[serde(default)]
    pub pt_spectral_dispersion: bool,
    // GPU acceleration options
    pub pt_gpu_bvh: bool,
    pub pt_bvh_refit: bool,
    pub pt_wavefront: bool,
    pub pt_wavefront_tile_size: u32,
    pub pt_russian_roulette: bool,
    pub pt_adaptive_sampling: bool,
    #[serde(default)]
    pub pt_adaptive_preset: AdaptivePreset,
    #[serde(default = "default_adaptive_min_spp")]
    pub pt_adaptive_min_spp: u32,
    #[serde(default = "default_adaptive_max_spp")]
    pub pt_adaptive_max_spp: u32,
    #[serde(default = "default_adaptive_variance")]
    pub pt_adaptive_variance: f32,
    #[serde(default = "default_adaptive_interval")]
    pub pt_adaptive_interval: u32,
    // ReSTIR options
    pub pt_restir_di: bool,
    pub pt_restir_gi: bool,
    pub pt_restir_temporal: bool,
    pub pt_restir_spatial: bool,
    #[serde(default = "default_restir_m_max")]
    pub pt_restir_m_max: u32,
    // Path Guiding options
    pub pt_path_guiding: bool,
    #[serde(default = "default_svo_resolution")]
    pub pt_svo_resolution: u32,
    // Slice plane (cut through scene)
    pub slice_enabled: bool,
    pub slice_axis: u32,  // 0=X, 1=Y, 2=Z (used when slice_use_vector=false)
    #[serde(default = "default_slice_position")]
    pub slice_position: f32,
    #[serde(default = "default_slice_position_vector")]
    pub slice_position_vector: f32,
    pub slice_invert: bool,
    pub slice_use_vector: bool,  // true = use arbitrary normal, false = use axis
    #[serde(default = "default_slice_normal")]
    pub slice_normal: [f32; 3],  // Arbitrary slice plane normal (normalized)
    // LOD (Level of Detail)
    pub lod_enabled: bool,
    #[serde(default = "default_lod_min_size")]
    pub lod_min_screen_size: f32,  // Min screen size in pixels to render
    // Camera inertia
    #[serde(default = "default_inertia_enabled")]
    pub inertia_enabled: bool,
    #[serde(default = "default_inertia_friction")]
    pub inertia_friction: f32,  // Higher = faster stop (1-10 typical)
    #[serde(default = "default_inertia_cutoff")]
    pub inertia_cutoff: f32,  // Stop inertia when speed is below cutoff
}

fn default_lod_min_size() -> f32 { 2.0 }
fn default_true() -> bool { true }
fn default_false() -> bool { false }
fn default_prob() -> f32 { 0.5 }
fn default_materialize_mix() -> f32 { 1.0 }
fn default_light_warm() -> f32 { 0.5 }
fn default_light_cool() -> f32 { 0.5 }
fn default_light_intensity() -> f32 { 1.0 }
fn default_light_color_randomness() -> f32 { 0.0 }
fn default_mat_seed() -> u32 { 2654435761 }
fn default_quant_levels() -> u32 { 5 }
fn default_band_count() -> u32 { 8 }
fn default_spatial_scale() -> f32 { 0.01 }
fn default_transparency() -> f32 { 0.0 }
fn default_folder_tint() -> f32 { 0.0 }
fn default_glass_specular() -> f32 { 1.0 }
fn default_glass_base() -> f32 { 0.0 }
fn default_glass_roughness() -> f32 { 0.02 }
fn default_glass_ior() -> f32 { 1.52 }
fn default_glass_dispersion() -> f32 { 0.0 }
fn default_glass_temp() -> f32 { 6500.0 }
fn default_inertia_enabled() -> bool { true }
fn default_inertia_friction() -> f32 { 5.0 }
fn default_inertia_cutoff() -> f32 { 0.001 }
fn default_height_power() -> f32 { 2.0 }
fn default_restir_m_max() -> u32 { 30 }
fn default_env_speed() -> f32 { 1.0 }
fn default_svo_resolution() -> u32 { 64 }
fn default_slice_position() -> f32 { 0.0 }
fn default_slice_position_vector() -> f32 { 0.0 }
fn default_slice_normal() -> [f32; 3] { [0.0, 1.0, 0.0] }  // Default: Y-up
fn default_adaptive_min_spp() -> u32 { 1 }
fn default_adaptive_max_spp() -> u32 { 64 }
fn default_adaptive_variance() -> f32 { 0.001 }
fn default_adaptive_interval() -> u32 { 4 }
fn default_spectral_samples() -> u32 { 2 }

impl Default for Render3DOptions {
    fn default() -> Self {
        Self {
            height_mode: CubeHeightMode::FileSize,
            height_power_enabled: false,
            height_power: 2.0,
            height_scale: 1.0,
            color_mode: ColorMode::FileType,
            folder_color_mode: FolderColorMode::Depth,
            folder_tint: default_folder_tint(),
            hash_effect: HashTransformEffect::Pulse,
            hash_effect_strength: 2.0,
            animation_time: 0.0,
            animation_speed: 3.0,
            animate: true,
            show_wireframe: false,
            hover_mode: HoverMode::Both,
            hover_outline_width: 2.0,
            hover_outline_alpha: 1.0,
            roughness: 0.5,
            metalness: 0.0,
            specular_ior: 1.5,
            xray_alpha: 1.0,
            flat_shading: false,
            double_sided: false,
            materialize_mode: MaterializeMode::ByExtension,
            mat_source: MaterialSource::Extension,
            mat_distribution: MaterialDistribution::Direct,
            mat_quant_levels: default_quant_levels(),
            mat_band_count: default_band_count(),
            mat_spatial_scale: default_spatial_scale(),
            materialize_mix: 1.0,
            mat_allow_lights: true,
            mat_light_prob: 0.15,
            mat_light_warm: 0.5,
            mat_light_cool: 0.5,
            mat_light_intensity: default_light_intensity(),
            mat_light_color_randomness: default_light_color_randomness(),
            mat_allow_glass: false,
            mat_glass_prob: 0.61,
            mat_include_dirs: false,
            mat_seed: default_mat_seed(),
            pt_global_transparency: 0.0,
            pt_global_glass: GlassPreset::Clear,
            pt_glass_specular: default_glass_specular(),
            pt_glass_base: default_glass_base(),
            pt_glass_roughness: default_glass_roughness(),
            pt_glass_ior: default_glass_ior(),
            pt_glass_dispersion: default_glass_dispersion(),
            pt_glass_temp: default_glass_temp(),
            pt_glass_thin: false,
            env_map_intensity: 1.0,
            env_map_rotation: 0.0,
            env_map_enabled: true,
            env_map_visible: true,
            env_map_path: Some(std::path::PathBuf::from("data/uffizi-large.hdr")),
            env_animate: true,
            env_speed: 1.0,
            background_color: [0.1, 0.1, 0.1],
            path_tracing: true,
            pt_max_bounces: 4,
            pt_max_samples: 3500,
            pt_samples_per_update: 25,
            pt_max_transmission_depth: 8,
            pt_dof_enabled: false,
            pt_aperture: 2.0,
            pt_focus_distance: 500.0,
            pt_env_importance_sampling: true,
            pt_target_fps: 30.0,
            pt_auto_spp: false,
            pt_camera_snap: false,
            pt_spectral_mode: SpectralMode::Off,
            pt_spectral_samples: default_spectral_samples(),
            pt_spectral_dispersion: false,
            // GPU acceleration
            pt_gpu_bvh: true,
            pt_bvh_refit: true,
            pt_wavefront: false,
            pt_wavefront_tile_size: 1024,
            pt_russian_roulette: true,
            pt_adaptive_sampling: true,
            pt_adaptive_preset: AdaptivePreset::Custom,
            pt_adaptive_min_spp: default_adaptive_min_spp(),
            pt_adaptive_max_spp: default_adaptive_max_spp(),
            pt_adaptive_variance: default_adaptive_variance(),
            pt_adaptive_interval: default_adaptive_interval(),
            // ReSTIR
            pt_restir_di: true,
            pt_restir_gi: true,
            pt_restir_temporal: true,
            pt_restir_spatial: true,
            pt_restir_m_max: 30,
            // Path Guiding
            pt_path_guiding: true,
            pt_svo_resolution: 64,
            // Slice plane
            slice_enabled: false,
            slice_axis: 1,
            slice_position: -500.0,
            slice_position_vector: 0.0,
            slice_invert: false,
            slice_use_vector: false,
            slice_normal: [0.0, 1.0, 0.0],
            // LOD
            lod_enabled: false,
            lod_min_screen_size: 1.0,
            // Inertia
            inertia_enabled: true,
            inertia_friction: 5.0,
            inertia_cutoff: 0.001,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::Render3DOptions;

    #[test]
    fn render_3d_options_deserialize_defaults() {
        let json = "{}";
        let opts: Render3DOptions = serde_json::from_str(json).expect("deserialize");
        let defaults = Render3DOptions::default();
        assert_eq!(opts.pt_max_bounces, defaults.pt_max_bounces);
        assert_eq!(opts.pt_max_samples, defaults.pt_max_samples);
        assert_eq!(opts.pt_gpu_bvh, defaults.pt_gpu_bvh);
        assert_eq!(opts.pt_spectral_mode, defaults.pt_spectral_mode);
        assert_eq!(opts.pt_spectral_samples, defaults.pt_spectral_samples);
        assert_eq!(opts.pt_spectral_dispersion, defaults.pt_spectral_dispersion);
    }
}

/// Orbit camera for 3D view (Houdini-style controls)
#[derive(Debug, Clone)]
pub struct OrbitCamera {
    /// Horizontal rotation angle (radians)
    pub yaw: f32,
    /// Vertical rotation angle (radians)
    pub pitch: f32,
    /// Distance from target
    pub distance: f32,
    /// Look-at target position
    pub target: Vec3,
    /// Field of view in radians
    pub fov: f32,
    /// Near clip plane
    pub near: f32,
    /// Far clip plane
    pub far: f32,
    // Inertia velocities
    yaw_velocity: f32,
    pitch_velocity: f32,
    distance_velocity: f32,
    target_velocity: Vec3,
    // Animation targets
    yaw_target: f32,
    pitch_target: f32,
    distance_target: f32,
    target_target: Vec3,
    animating: bool,
}

impl Default for OrbitCamera {
    fn default() -> Self {
        Self {
            yaw: 0.0,                                // Front view (matches 2D)
            pitch: 0.0,                              // Looking straight ahead
            distance: 500.0,
            target: Vec3::ZERO,
            fov: std::f32::consts::FRAC_PI_4,
            near: 0.1,
            far: 100000.0,
            yaw_velocity: 0.0,
            pitch_velocity: 0.0,
            distance_velocity: 0.0,
            target_velocity: Vec3::ZERO,
            yaw_target: 0.0,
            pitch_target: 0.0,
            distance_target: 500.0,
            target_target: Vec3::ZERO,
            animating: false,
        }
    }
}

impl OrbitCamera {
    /// Orbit the camera (left mouse drag) - non-inertia version
    #[allow(dead_code)]
    pub fn orbit(&mut self, delta_x: f32, delta_y: f32) {
        let sensitivity = 0.005;
        self.yaw += delta_x * sensitivity;
        self.pitch = (self.pitch + delta_y * sensitivity)
            .clamp(-std::f32::consts::FRAC_PI_2 + 0.1, std::f32::consts::FRAC_PI_2 - 0.1);
    }
    
    /// Pan the camera (middle mouse drag) - non-inertia version
    #[allow(dead_code)]
    pub fn pan(&mut self, delta_x: f32, delta_y: f32) {
        let sensitivity = self.distance * 0.001;
        
        // Calculate right and up vectors in world space
        let right = Vec3::new(self.yaw.cos(), 0.0, -self.yaw.sin());
        let up = Vec3::Y;
        
        self.target -= right * delta_x * sensitivity;
        self.target += up * delta_y * sensitivity;
    }
    
    /// Zoom the camera (right mouse drag or scroll) - non-inertia version
    #[allow(dead_code)]
    pub fn zoom(&mut self, delta: f32) {
        let factor = 1.0 + delta * 0.001;
        self.distance = (self.distance * factor).clamp(10.0, 5000.0);
    }
    
    /// Get camera position in world space
    pub fn position(&self) -> Vec3 {
        let x = self.distance * self.pitch.cos() * self.yaw.sin();
        let y = self.distance * self.pitch.sin();
        let z = self.distance * self.pitch.cos() * self.yaw.cos();
        self.target + Vec3::new(x, y, z)
    }
    
    /// Get view matrix
    pub fn view_matrix(&self) -> Mat4 {
        Mat4::look_at_rh(self.position(), self.target, Vec3::Y)
    }
    
    /// Get projection matrix
    pub fn projection_matrix(&self, aspect: f32) -> Mat4 {
        Mat4::perspective_rh(self.fov, aspect, self.near, self.far)
    }
    
    /// Get combined view-projection matrix
    pub fn view_projection_matrix(&self, aspect: f32) -> Mat4 {
        self.projection_matrix(aspect) * self.view_matrix()
    }

    /// Orbit with inertia (adds to velocity)
    pub fn orbit_inertia(&mut self, delta_x: f32, delta_y: f32) {
        let sensitivity = 0.005;
        self.yaw_velocity += delta_x * sensitivity;
        self.pitch_velocity += delta_y * sensitivity;
    }

    /// Pan with inertia (adds to velocity)
    pub fn pan_inertia(&mut self, delta_x: f32, delta_y: f32) {
        let sensitivity = self.distance * 0.001;
        let right = Vec3::new(self.yaw.cos(), 0.0, -self.yaw.sin());
        let up = Vec3::Y;
        self.target_velocity -= right * delta_x * sensitivity;
        self.target_velocity += up * delta_y * sensitivity;
    }

    /// Zoom with inertia
    pub fn zoom_inertia(&mut self, delta: f32) {
        self.distance_velocity += delta * self.distance * 0.0005;
    }

    /// Update camera with inertia (call each frame)
    /// Returns true if camera is still moving
    pub fn update_inertia(&mut self, dt: f32, friction: f32, cutoff: f32) -> bool {
        let decay = (-friction * dt).exp();
        let threshold = cutoff.max(0.000001);

        // Apply velocities
        self.yaw += self.yaw_velocity * dt;
        self.pitch = (self.pitch + self.pitch_velocity * dt)
            .clamp(-std::f32::consts::FRAC_PI_2 + 0.1, std::f32::consts::FRAC_PI_2 - 0.1);
        self.distance = (self.distance + self.distance_velocity * dt).clamp(10.0, 5000.0);
        self.target += self.target_velocity * dt;

        // Apply friction
        self.yaw_velocity *= decay;
        self.pitch_velocity *= decay;
        self.distance_velocity *= decay;
        self.target_velocity *= decay;

        // Snap to rest below threshold to avoid jitter
        if self.yaw_velocity.abs() < threshold { self.yaw_velocity = 0.0; }
        if self.pitch_velocity.abs() < threshold { self.pitch_velocity = 0.0; }
        if self.distance_velocity.abs() < threshold { self.distance_velocity = 0.0; }
        if self.target_velocity.length() < threshold { self.target_velocity = Vec3::ZERO; }

        // Check if still moving
        self.yaw_velocity != 0.0
            || self.pitch_velocity != 0.0
            || self.distance_velocity != 0.0
            || self.target_velocity != Vec3::ZERO
    }

    /// Stop all inertia immediately
    pub fn stop_inertia(&mut self) {
        self.yaw_velocity = 0.0;
        self.pitch_velocity = 0.0;
        self.distance_velocity = 0.0;
        self.target_velocity = Vec3::ZERO;
    }

    /// Check if camera has inertia (alternative to update_inertia return value)
    #[allow(dead_code)]
    pub fn has_inertia(&self) -> bool {
        let threshold = 0.0001;
        self.yaw_velocity.abs() > threshold
            || self.pitch_velocity.abs() > threshold
            || self.distance_velocity.abs() > threshold
            || self.target_velocity.length() > threshold
    }

    /// Check if camera is animating
    pub fn is_animating(&self) -> bool {
        self.animating
    }

    /// Cancel any active animation (user took control)
    pub fn cancel_animation(&mut self) {
        self.animating = false;
    }

    /// Reset to default view (with animation)
    pub fn reset(&mut self) {
        let def = Self::default();
        self.yaw_target = def.yaw;
        self.pitch_target = def.pitch;
        self.distance_target = def.distance;
        self.target_target = def.target;
        self.animating = true;
        self.stop_inertia();
    }

    /// Animate to a specific state
    pub fn animate_to(&mut self, yaw: f32, pitch: f32, distance: f32, target: Vec3) {
        self.yaw_target = yaw;
        self.pitch_target = pitch;
        self.distance_target = distance;
        self.target_target = target;
        self.animating = true;
        self.stop_inertia();
    }

    /// Animate zoom only (keep current yaw/pitch)
    pub fn animate_zoom_to(&mut self, distance: f32, target: Vec3) {
        self.yaw_target = self.yaw;
        self.pitch_target = self.pitch;
        self.distance_target = distance;
        self.target_target = target;
        self.animating = true;
        self.stop_inertia();
    }

    /// Update animation (call each frame, returns true if still animating)
    pub fn update_animation(&mut self, dt: f32) -> bool {
        if !self.animating { return false; }

        let speed = 8.0 * dt; // Animation speed
        let t = speed.min(1.0);

        self.yaw = self.yaw + (self.yaw_target - self.yaw) * t;
        self.pitch = self.pitch + (self.pitch_target - self.pitch) * t;
        self.distance = self.distance + (self.distance_target - self.distance) * t;
        self.target = self.target + (self.target_target - self.target) * t;

        // Check if close enough to stop
        let threshold = 0.001;
        if (self.yaw - self.yaw_target).abs() < threshold
            && (self.pitch - self.pitch_target).abs() < threshold
            && (self.distance - self.distance_target).abs() < threshold
            && (self.target - self.target_target).length() < threshold
        {
            self.yaw = self.yaw_target;
            self.pitch = self.pitch_target;
            self.distance = self.distance_target;
            self.target = self.target_target;
            self.animating = false;
        }

        true
    }

    /// Set front-view matching 2D layout (looking along +Z at XY wall)
    pub fn set_front_view(&mut self, width: f32, height: f32) {
        self.yaw = 0.0;
        self.pitch = 0.0;
        self.target = Vec3::new(width / 2.0, -(height / 2.0), 0.0);
        // Distance so treemap fills view (approx)
        let half_h = height / 2.0;
        let fov_half = self.fov / 2.0;
        self.distance = half_h / fov_half.tan();
    }

    /// Set front-view with animation (full reset including rotation)
    pub fn animate_to_front_view(&mut self, width: f32, height: f32) {
        let target = Vec3::new(width / 2.0, -(height / 2.0), 0.0);
        let half_h = height / 2.0;
        let fov_half = self.fov / 2.0;
        let distance = half_h / fov_half.tan();
        self.animate_to(0.0, 0.0, distance, target);
    }

    /// Zoom to fit scene without changing rotation
    pub fn zoom_to_fit_scene(&mut self, width: f32, height: f32) {
        let target = Vec3::new(width / 2.0, -(height / 2.0), 0.0);
        let half_h = height / 2.0;
        let fov_half = self.fov / 2.0;
        let distance = half_h / fov_half.tan();
        self.animate_zoom_to(distance, target);
    }
}

/// Compute a deterministic hash from a string (for per-cube transforms)
pub fn name_hash(name: &str) -> u32 {
    name.bytes().fold(0u32, |acc, b| acc.wrapping_mul(31).wrapping_add(b as u32))
}

/// Derive secondary hash without string allocation
#[inline]
fn hash_derive(hash: u32, salt: u32) -> u32 {
    hash.wrapping_mul(1664525).wrapping_add(salt).wrapping_mul(1013904223)
}

/// Transform result with offset and rotation
pub struct CubeTransform {
    pub offset: Vec3,
    pub rotation: glam::Quat,
}

impl Default for CubeTransform {
    fn default() -> Self {
        Self {
            offset: Vec3::ZERO,
            rotation: glam::Quat::IDENTITY,
        }
    }
}

/// Compute hash-based transform (offset + rotation) for a cube
/// Optimized: only computes rotation for effects that need it
#[inline]
pub fn hash_transform(
    name: &str,
    base_pos: Vec3,
    center: Vec3,
    effect: HashTransformEffect,
    strength: f32,
    time: f32,
) -> CubeTransform {
    // Fast path: effects without rotation
    let needs_rotation = matches!(
        effect,
        HashTransformEffect::Rotate3D
            | HashTransformEffect::Spiral
            | HashTransformEffect::Twist
            | HashTransformEffect::Swarm
            | HashTransformEffect::Earthquake
            | HashTransformEffect::Ocean
            | HashTransformEffect::Echo
    );

    if !needs_rotation {
        return CubeTransform {
            offset: hash_transform_offset(name, base_pos, center, effect, strength, time),
            rotation: glam::Quat::IDENTITY,
        };
    }

    // Compute hash once, reuse for both offset and rotation
    let hash = name_hash(name);
    let hash_f = (hash as f32) / (u32::MAX as f32);
    let tau = std::f32::consts::TAU;
    let phase = hash_f * tau;

    let (offset, rotation) = match effect {
        HashTransformEffect::Rotate3D => {
            let px = base_pos.x * 0.03 + time * 0.2;
            let py = base_pos.y * 0.03 + time * 0.15;
            let pz = base_pos.z * 0.03 + time * 0.1;
            let ax = (px.sin() + (py * 1.3).cos()) * strength * 0.3 + phase * 0.2;
            let ay = (py.sin() + (pz * 1.5).cos()) * strength * 0.3 + phase * 0.25;
            let az = (pz.sin() + (px * 1.2).cos()) * strength * 0.3 + phase * 0.15;
            // Small displacement from rotation effect
            let disp = Vec3::new(ax.sin() * strength * 3.0, ay.sin() * strength * 3.0, az.cos() * strength * 2.0);
            (disp, glam::Quat::from_euler(glam::EulerRot::XYZ, ax, ay, az))
        }

        HashTransformEffect::Spiral => {
            let rel = base_pos - center;
            let angle = (time * 0.5 + phase) * strength;
            let rot_mat = Mat4::from_rotation_z(angle);
            let rotated = rot_mat.transform_point3(rel);
            let offset = (rotated - rel) * 0.6;
            (offset, glam::Quat::from_rotation_z(angle * 0.3))
        }

        HashTransformEffect::Twist => {
            let rel = base_pos - center;
            let height_factor = (-base_pos.z / 50.0).clamp(0.0, 2.0);
            let angle = (time * 0.08 + phase * 0.3) * height_factor * strength;
            let rot_mat = Mat4::from_rotation_z(angle);
            let rotated = rot_mat.transform_point3(rel);
            let offset = (rotated - rel) * 0.8;
            (offset, glam::Quat::from_rotation_z(angle * 0.4))
        }

        HashTransformEffect::Swarm => {
            let h2 = hash_derive(hash, 0x73737373);
            let h3 = hash_derive(hash, 0x77777777);
            let p2 = (h2 as f32) / (u32::MAX as f32) * tau;
            let p3 = (h3 as f32) / (u32::MAX as f32) * tau;
            let jx = (time * 0.6 + phase).sin() + (time * 1.4 + p2).sin() * 0.3;
            let jy = (time * 0.5 + p2).cos() + (time * 1.1 + phase).cos() * 0.4;
            let jz = (time * 0.4 + p3).sin() + (time * 0.8 + p3).cos() * 0.2;
            let offset = Vec3::new(jx, jy, jz) * strength * 5.0;
            let ax = (time * 0.4 + phase).sin() * strength * 0.15;
            let ay = (time * 0.5 + p2).cos() * strength * 0.15;
            (offset, glam::Quat::from_euler(glam::EulerRot::XYZ, ax, ay, 0.0))
        }

        HashTransformEffect::Earthquake => {
            let intensity = ((time * 0.5).sin() * 0.5 + 0.5).powf(2.0);
            let shake_x = (time * 15.0 + phase).sin() * intensity;
            let shake_y = (time * 17.0 + phase * 1.3).cos() * intensity;
            let shake_z = (time * 12.0 + phase * 0.7).sin() * intensity * 0.5;
            let offset = Vec3::new(shake_x, shake_y, shake_z) * strength * 8.0;
            let ax = (time * 12.0 + phase).sin() * intensity * strength * 0.1;
            let ay = (time * 14.0 + phase * 1.2).cos() * intensity * strength * 0.1;
            (offset, glam::Quat::from_euler(glam::EulerRot::XYZ, ax, ay, 0.0))
        }

        HashTransformEffect::Ocean => {
            let rel = base_pos - center;
            let dist = (rel.x * rel.x + rel.y * rel.y).sqrt();
            let wave1 = (time * 0.3 + dist * 0.02 + phase * 0.3).sin();
            let wave2 = (time * 0.2 - dist * 0.015 + phase * 0.7).cos() * 0.5;
            let wave3 = (time * 0.4 + rel.x * 0.01 + rel.y * 0.008).sin() * 0.3;
            let offset = Vec3::new(0.0, 0.0, -(wave1 + wave2 + wave3) * strength * 15.0);
            let tilt = wave1 * strength * 0.05;
            (offset, glam::Quat::from_euler(glam::EulerRot::XYZ, tilt, tilt * 0.5, 0.0))
        }

        HashTransformEffect::Echo => {
            let rel = base_pos - center;
            let dist = rel.length();
            // Phase offset based on distance - creates wave-like delay
            let phase_offset = dist * 0.05;
            let master_angle = time * 0.3;
            let delayed = master_angle - phase_offset;
            // Offset: circular orbit
            let orbit_radius = strength * 3.0;
            let ox = delayed.cos() * orbit_radius;
            let oy = delayed.sin() * orbit_radius;
            let oz = (delayed * 2.0).sin() * strength * 2.0;
            let offset = Vec3::new(ox, oy, oz);
            // Rotation: follow master rotation with phase delay
            let rot_angle = delayed * 0.5;
            let rot = glam::Quat::from_euler(glam::EulerRot::XYZ, rot_angle.sin() * 0.3, rot_angle.cos() * 0.3, rot_angle * 0.2);
            (offset, rot)
        }

        _ => (Vec3::ZERO, glam::Quat::IDENTITY),
    };

    CubeTransform { offset, rotation }
}

/// Compute hash-based transform offset for a cube
pub fn hash_transform_offset(
    name: &str,
    base_pos: Vec3,
    center: Vec3,
    effect: HashTransformEffect,
    strength: f32,
    time: f32,
) -> Vec3 {
    let hash = name_hash(name);
    let hash_f = (hash as f32) / (u32::MAX as f32);  // 0.0 to 1.0
    let tau = std::f32::consts::TAU;
    let phase = hash_f * tau;  // unique phase per cube

    match effect {
        HashTransformEffect::None => Vec3::ZERO,

        // Vertical sine wave with per-cube phase
        HashTransformEffect::Wave => {
            let wave = ((time * 2.0 + phase).sin() * 0.5 + 0.5) * strength * 20.0;
            Vec3::new(0.0, 0.0, -wave)
        }

        // Pulsing random heights - floats up and down
        HashTransformEffect::RandomHeight => {
            let base_offset = (hash_f - 0.5) * 2.0;
            let pulse = (time * 0.8 + phase).sin() * 0.3 + 0.7;
            Vec3::new(0.0, 0.0, -base_offset * pulse * strength * 30.0)
        }

        // Drifting 3D positions - slow organic movement
        HashTransformEffect::RandomOffset => {
            let h2 = hash_derive(hash, 0x78787878);
            let h3 = hash_derive(hash, 0x79797979);
            let p2 = (h2 as f32) / (u32::MAX as f32) * tau;
            let p3 = (h3 as f32) / (u32::MAX as f32) * tau;
            let hx = (time * 0.4 + phase).sin();
            let hy = (time * 0.5 + p2).sin();
            let hz = (time * 0.3 + p3).cos();
            Vec3::new(hx, hy, hz) * strength * 10.0
        }

        // Pulsing explosion - breathes in and out
        HashTransformEffect::Explode => {
            let dir = (base_pos - center).normalize_or_zero();
            let pulse = (time * 0.6 + phase * 0.5).sin() * 0.4 + 0.6;
            dir * hash_f * pulse * strength * 50.0
        }

        // Smooth noise drift
        HashTransformEffect::Noise => {
            let t = time * 0.6 + phase;
            let n = (t.sin() + (t * 1.7).cos()) * 0.5;
            Vec3::new(n, (t * 1.3).sin() * 0.5, (t * 0.7).cos() * 0.5) * strength * 8.0
        }

        // Radial breathing pulse
        HashTransformEffect::Pulse => {
            let dir = (base_pos - center).normalize_or_zero();
            let pulse = (time * 1.5 + phase).sin() * 0.5 + 0.5;
            dir * pulse * strength * 25.0
        }

        // Spiral swirl around Z axis
        HashTransformEffect::Spiral => {
            let rel = base_pos - center;
            let angle = (time * 0.5 + phase) * strength;
            let rot = Mat4::from_rotation_z(angle);
            let rotated = rot.transform_point3(rel);
            (rotated - rel) * 0.6
        }

        // Large slow ocean waves
        HashTransformEffect::Ocean => {
            let rel = base_pos - center;
            let dist = (rel.x * rel.x + rel.y * rel.y).sqrt();
            let wave1 = (time * 0.3 + dist * 0.02 + phase * 0.3).sin();
            let wave2 = (time * 0.2 - dist * 0.015 + phase * 0.7).cos() * 0.5;
            let wave3 = (time * 0.4 + rel.x * 0.01 + rel.y * 0.008).sin() * 0.3;
            Vec3::new(0.0, 0.0, -(wave1 + wave2 + wave3) * strength * 15.0)
        }

        // 3D noise-based rotation - cubes tumble in place
        HashTransformEffect::Rotate3D => {
            // Use position for spatial coherence (neighbors rotate similarly)
            let px = base_pos.x * 0.03 + time * 0.2;
            let py = base_pos.y * 0.03 + time * 0.15;
            let pz = base_pos.z * 0.03 + time * 0.1;
            // Rotation angles based on position noise + per-cube phase
            let ax = (px.sin() + (py * 1.3).cos()) * strength * 0.2 + phase * 0.1;
            let ay = (py.sin() + (pz * 1.5).cos()) * strength * 0.2 + phase * 0.15;
            let az = (pz.sin() + (px * 1.2).cos()) * strength * 0.2 + phase * 0.08;
            // Small displacement based on rotation (tumbling effect)
            let disp_x = ax.sin() * strength * 3.0;
            let disp_y = ay.sin() * strength * 3.0;
            let disp_z = az.cos() * strength * 2.0;
            Vec3::new(disp_x, disp_y, disp_z)
        }

        // Twisting tower - rotation based on height
        HashTransformEffect::Twist => {
            let rel = base_pos - center;
            let height_factor = (-base_pos.z / 50.0).clamp(0.0, 2.0);
            let angle = (time * 0.08 + phase * 0.3) * height_factor * strength;
            let rot = Mat4::from_rotation_z(angle);
            let rotated = rot.transform_point3(rel);
            (rotated - rel) * 0.8
        }

        // Synchronized breathing - all cubes scale together with slight offset
        HashTransformEffect::Breathe => {
            let dir = (base_pos - center).normalize_or_zero();
            let dist = (base_pos - center).length();
            let breath = (time * 0.8).sin() * 0.5 + 0.5;
            let local_offset = (phase * 0.2).sin() * 0.1;
            dir * (breath + local_offset) * (dist * 0.01).min(1.0) * strength * 20.0
        }

        // Insect swarm - jittery random movement (slowed down 5x)
        HashTransformEffect::Swarm => {
            let h2 = hash_derive(hash, 0x73737373);
            let h3 = hash_derive(hash, 0x77777777);
            let p2 = (h2 as f32) / (u32::MAX as f32) * tau;
            let p3 = (h3 as f32) / (u32::MAX as f32) * tau;
            let jx = (time * 0.6 + phase).sin() + (time * 1.4 + p2).sin() * 0.3;
            let jy = (time * 0.5 + p2).cos() + (time * 1.1 + phase).cos() * 0.4;
            let jz = (time * 0.4 + p3).sin() + (time * 0.8 + p3).cos() * 0.2;
            Vec3::new(jx, jy, jz) * strength * 5.0
        }

        // Earthquake - shaking with aftershocks
        HashTransformEffect::Earthquake => {
            let intensity = ((time * 0.5).sin() * 0.5 + 0.5).powf(2.0);
            let shake_x = (time * 15.0 + phase).sin() * intensity;
            let shake_y = (time * 17.0 + phase * 1.3).cos() * intensity;
            let shake_z = (time * 12.0 + phase * 0.7).sin() * intensity * 0.5;
            Vec3::new(shake_x, shake_y, shake_z) * strength * 8.0
        }

        // Ripple - concentric water ripples from center
        HashTransformEffect::Ripple => {
            let rel = base_pos - center;
            let dist = (rel.x * rel.x + rel.y * rel.y).sqrt();
            // Multiple ripple frequencies for natural look
            let ripple1 = (time * 2.0 - dist * 0.08).sin();
            let ripple2 = (time * 3.5 - dist * 0.12 + 1.0).sin() * 0.5;
            let ripple3 = (time * 1.2 - dist * 0.05 + phase * 0.3).sin() * 0.3;
            // Amplitude decreases with distance
            let falloff = 1.0 / (1.0 + dist * 0.02);
            let height = (ripple1 + ripple2 + ripple3) * falloff;
            Vec3::new(0.0, 0.0, -height * strength * 12.0)
        }

        // Vortex - rotating pull toward center
        HashTransformEffect::Vortex => {
            let rel = base_pos - center;
            let dist = rel.length().max(1.0);
            let angle = time * 1.5 + dist * 0.03 + phase * 0.5;
            // Spiral inward
            let pull = (1.0 / dist.sqrt()) * strength * 15.0;
            let rot = Mat4::from_rotation_z(angle);
            let spiral_dir = rot.transform_point3(rel.normalize_or_zero());
            let inward = -rel.normalize_or_zero() * pull * 0.3;
            let tangent = Vec3::new(-spiral_dir.y, spiral_dir.x, 0.0) * pull;
            // Sink toward center with height oscillation
            let sink = (time * 2.0 + phase).sin() * strength * 3.0;
            inward + tangent * 0.5 + Vec3::new(0.0, 0.0, -sink)
        }

        // Glitch - digital artifact displacement (slowed down 5x)
        HashTransformEffect::Glitch => {
            // Random glitch timing based on time quantization
            let glitch_time = (time * 1.6).floor();
            let glitch_hash = hash_derive(hash, glitch_time as u32);
            let glitch_f = (glitch_hash as f32) / (u32::MAX as f32);
            // Only glitch sometimes
            let active = if glitch_f > 0.7 { 1.0 } else { 0.0 };
            // Quantized displacement (digital look)
            let h2 = hash_derive(hash, 0x99999999);
            let h3 = hash_derive(hash, 0xAAAAAAAA);
            let dx = ((h2 as f32 / u32::MAX as f32) - 0.5) * 2.0;
            let dy = ((h3 as f32 / u32::MAX as f32) - 0.5) * 2.0;
            // Horizontal bands effect
            let band = ((base_pos.y * 0.1 + time * 0.6).floor() % 3.0 == 0.0) as i32 as f32;
            Vec3::new(dx * band, dy * (1.0 - band), 0.0) * active * strength * 20.0
        }

        // Echo - handled in hash_transform (with rotation)
        HashTransformEffect::Echo => Vec3::ZERO
    }
}

// ============================================================================
// GPU uniform structs (must match WGSL shader layouts exactly)
// ============================================================================

/// Camera uniform (256 bytes, matches Camera in cube_pbr.wgsl)
#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
pub struct CameraUniform {
    pub view_proj: [[f32; 4]; 4],
    pub view: [[f32; 4]; 4],
    pub inv_view_proj: [[f32; 4]; 4],
    pub position: [f32; 3],
    pub xray_alpha: f32,
    pub flat_shading: f32,
    pub slice_enabled: f32,
    pub slice_position: f32,
    pub slice_invert: f32,
    pub slice_normal: [f32; 3],  // Slice plane normal (normalized)
    pub _pad: [f32; 5], // Pad to 256 bytes total
}

/// Single directional light (32 bytes)
#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
pub struct LightUniform {
    pub direction: [f32; 3],
    pub _pad: f32,
    pub color: [f32; 3],
    pub intensity: f32,
}

/// 3-point lighting rig (112 bytes)
#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
pub struct LightRigUniform {
    pub key: LightUniform,
    pub fill: LightUniform,
    pub rim: LightUniform,
    pub ambient: [f32; 3],
    pub _pad: f32,
}

impl Default for LightRigUniform {
    fn default() -> Self {
        Self {
            key: LightUniform {
                direction: [-0.5, -0.7, -0.5],  // Top-left front
                _pad: 0.0,
                color: [1.0, 0.98, 0.95],        // Warm white
                intensity: 1.2,
            },
            fill: LightUniform {
                direction: [0.7, -0.3, 0.5],     // Right side, softer
                _pad: 0.0,
                color: [0.7, 0.8, 1.0],          // Cool blue fill
                intensity: 0.5,
            },
            rim: LightUniform {
                direction: [0.0, -0.2, 1.0],     // Behind, edge light
                _pad: 0.0,
                color: [1.0, 1.0, 1.0],
                intensity: 0.3,
            },
            ambient: [0.15, 0.15, 0.18],
            _pad: 0.0,
        }
    }
}

/// Global PBR material params (16 bytes)
#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
pub struct MaterialParamsUniform {
    pub roughness: f32,
    pub metalness: f32,
    pub specular_ior: f32,
    pub specular_weight: f32,
}

impl Default for MaterialParamsUniform {
    fn default() -> Self {
        Self {
            roughness: 0.5,
            metalness: 0.0,
            specular_ior: 1.5,
            specular_weight: 1.0,
        }
    }
}

/// Environment map params (16 bytes)
#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
pub struct EnvParamsUniform {
    pub intensity: f32,
    pub rotation: f32,
    pub enabled: f32,
    pub _pad: f32,
}

impl Default for EnvParamsUniform {
    fn default() -> Self {
        Self {
            intensity: 1.0,
            rotation: 0.0,
            enabled: 0.0,
            _pad: 0.0,
        }
    }
}

/// Hover highlight params (64 bytes)
#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
pub struct HoverParamsUniform {
    pub hovered_id: u32,
    pub mode: u32,
    pub outline_width: f32,
    pub _pad0: f32,
    pub outline_color: [f32; 4],
    pub tint_color: [f32; 4],
    pub viewport_size: [f32; 2],
    pub _pad1: [f32; 2],
}

impl Default for HoverParamsUniform {
    fn default() -> Self {
        Self {
            hovered_id: 0,
            mode: 0,
            outline_width: 2.0,
            _pad0: 0.0,
            outline_color: [1.0, 0.5, 0.0, 1.0],  // Orange
            tint_color: [1.0, 0.7, 0.2, 0.15],     // Warm tint
            viewport_size: [0.0, 0.0],
            _pad1: [0.0; 2],
        }
    }
}
