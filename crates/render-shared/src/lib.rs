/// Renderer abstraction: CPU (rayon) or GPU (wgpu) backends.
use bytemuck::{Pod, Zeroable};
use glam::{Mat4, Vec3};
use pt_mats::{MaterialDistribution, MaterialSource, MaterializeMode, Palette};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

pub mod viz;
pub use viz::{
    AnimationState, CurveParams, EffectsState, HashEffectParams, Mapping, RampParams,
    N_COLOR_MODES, N_FOLDER_COLOR_MODES, N_HASH_EFFECTS, N_HEIGHT_MODES,
};

pub mod physical_camera;
pub use physical_camera::{
    CameraType, PhysicalCamera, FOCAL_LENGTH_PRESETS_MM, F_NUMBER_PRESETS,
    SENSOR_WIDTH_PRESETS_MM,
};

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
    Treemap, // Use treemap-assigned colors (depth-based)
    FileType, // Color by file extension category
    FileAge,  // Color by modification time (old->new gradient)
    FileSize, // Color by file size (small->large gradient)
    Depth,    // Color by directory depth (rainbow gradient)
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
    Depth, // Depth-based rainbow gradient
    NameHash, // Hash of folder name
    PathHash, // Hash of full folder path
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

/// Path tracing pixel sampler.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
pub enum PtSamplerMode {
    /// Per-sample PCG random numbers.
    #[default]
    Pcg,
    /// R2 low-discrepancy pixel jitter with per-pixel scrambling.
    R2,
}

impl PtSamplerMode {
    pub fn name(&self) -> &'static str {
        match self {
            PtSamplerMode::Pcg => "PCG",
            PtSamplerMode::R2 => "R2",
        }
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
        &[SpectralMode::Off, SpectralMode::Hero, SpectralMode::Multi]
    }
}

/// Color gradient for depth (rainbow: red->orange->yellow->green->cyan->blue->magenta)
pub fn color_for_depth(depth: u32, max_depth: u32) -> [f32; 4] {
    let t = if max_depth > 0 {
        (depth as f32 / max_depth as f32).clamp(0.0, 1.0)
    } else {
        0.0
    };
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
        "rs" | "py" | "js" | "ts" | "c" | "cpp" | "h" | "hpp" | "java" | "go" | "rb" | "php"
        | "swift" | "kt" => [0.3, 0.5, 0.9, 1.0],
        // Web files - orange
        "html" | "htm" | "css" | "scss" | "sass" | "vue" | "jsx" | "tsx" => [0.95, 0.6, 0.2, 1.0],
        // Data files - green
        "json" | "xml" | "yaml" | "yml" | "toml" | "csv" | "sql" => [0.4, 0.8, 0.4, 1.0],
        // Documents - warm yellow
        "md" | "txt" | "doc" | "docx" | "pdf" | "rtf" | "odt" => [0.95, 0.85, 0.4, 1.0],
        // DCC scene files - distinct per DCC
        "mb" => [0.85, 0.65, 0.25, 1.0],
        "hou" => [0.9, 0.5, 0.15, 1.0],
        // HDR images - cyan/teal
        "exr" => [0.2, 0.8, 0.9, 1.0],
        // Film/RAW images - distinct per format
        "dpx" => [0.6, 0.45, 0.95, 1.0],
        "raf" => [0.45, 0.75, 0.35, 1.0],
        "nef" => [0.35, 0.55, 0.9, 1.0],
        // Images - purple/magenta (keep TIFF/TIF distinct)
        "tif" | "tiff" => [0.75, 0.35, 0.7, 1.0],
        "png" | "jpg" | "jpeg" | "gif" | "bmp" | "svg" | "webp" | "ico" => [0.8, 0.4, 0.8, 1.0],
        // Audio - cyan
        "mp3" | "wav" | "ogg" | "flac" | "aac" | "m4a" => [0.3, 0.8, 0.85, 1.0],
        // Video - red
        "mp4" | "avi" | "mkv" | "mov" | "wmv" | "flv" | "webm" => [0.9, 0.3, 0.3, 1.0],
        // Archives - brown
        "zip" | "tar" | "gz" | "7z" | "rar" | "bz2" | "xz" => [0.7, 0.5, 0.3, 1.0],
        // Executables - dark red
        "exe" | "dll" | "so" | "dylib" | "bin" | "app" => [0.7, 0.2, 0.2, 1.0],
        // Config files - teal
        "ini" | "conf" | "cfg" | "env" | "lock" => [0.3, 0.7, 0.65, 1.0],
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
    Wave,         // Sine wave based on hash
    RandomHeight, // Pulsing random height
    RandomOffset, // Drifting 3D offset
    Explode,      // Pulsing explosion outward
    Noise,        // Smooth noise drift
    Pulse,        // Radial breathing
    Spiral,       // Spiral swirl around center
    Ocean,        // Large slow waves like ocean surface
    Rotate3D,     // 3D rotation around center
    Twist,        // Twisting tower effect
    Breathe,      // Synchronized breathing
    Swarm,        // Insect swarm movement
    Earthquake,   // Shaking/trembling
    Ripple,       // Concentric ripples from center
    Vortex,       // Rotating vortex pulling inward
    Glitch,       // Digital glitch displacement
    Echo,         // Pulsing outward bloom
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
        &[
            HoverMode::None,
            HoverMode::Outline,
            HoverMode::Tint,
            HoverMode::Both,
        ]
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

fn default_animation_speed() -> f32 {
    1.0
}

/// One of `Render3DOptions::material_overrides` — a post-classify
/// hook that re-points a fraction of cubes at a specific library
/// material, regardless of what the global Source / Distribution
/// path picked. Two overrides live on `Render3DOptions`; each gets
/// its own `MaterialDistribution` + seed so they fire on *different*
/// random subsets of cubes (e.g. "5 % of cubes become glass" *and*
/// independently "10 % become emissive").
///
/// Replaces the legacy `mat_allow_lights` / `mat_allow_glass` flags
/// and their warm/cool/intensity satellites — light / glass are
/// just materials in the library now, so the user controls them
/// by aiming an override at the right slot.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct MaterialOverride {
    /// When false the override is bypassed; the base classification
    /// stands.
    pub enabled: bool,
    /// UUID of the library material to paint onto claimed cubes.
    /// UUIDs survive reorder / rename; the resolver falls back to a
    /// no-op when the UUID is missing from the active library.
    /// `None` ⇒ the override is inactive even if `enabled`.
    pub material_uuid: Option<Uuid>,
    /// Fraction of cubes the override claims (0..=1). Combined with
    /// `distribution` to pick which cubes specifically.
    pub probability: f32,
    /// Per-cube voting shape (Direct / Stratified / Spatial /
    /// Perlin / Gradient). Same enum the global classify uses, but
    /// evaluated independently here so two overrides can use
    /// different shapes — e.g. emissive in 3D clusters via Spatial
    /// and glass in stratified bands via Stratified.
    pub distribution: MaterialDistribution,
    pub band_count: u32,
    pub spatial_scale: f32,
    /// Seed for the voting. Two overrides with the same `enabled`,
    /// `material_uuid`, `probability`, and `distribution` but
    /// different seeds land on *disjoint* cube sets — that's what
    /// makes "two random distributions, one per override" not
    /// collide on the same cubes.
    pub seed: u32,
}

impl Default for MaterialOverride {
    fn default() -> Self {
        Self {
            enabled: false,
            material_uuid: None,
            probability: 0.1,
            distribution: MaterialDistribution::Direct,
            band_count: 8,
            spatial_scale: 0.01,
            seed: 2_654_435_761,
        }
    }
}

/// Options for 3D rendering
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct Render3DOptions {
    pub height_mode: CubeHeightMode,
    /// Per-mode scale + exponent. Switching modes preserves each mode's
    /// own values so "Size" length doesn't bleed into "Const".
    #[serde(default)]
    pub height_curves: Mapping<CurveParams, N_HEIGHT_MODES>,
    pub color_mode: ColorMode,
    /// Per-`ColorMode` palette + distribution + curve. Drives per-cube
    /// base color (tint), blended with material via `materialize_mix`.
    #[serde(default)]
    pub color_ramps: Mapping<RampParams, N_COLOR_MODES>,
    #[serde(default)]
    pub folder_color_mode: FolderColorMode,
    #[serde(default = "default_folder_tint")]
    pub folder_tint: f32,
    /// Per-`FolderColorMode` palette + distribution + curve. Drives
    /// folder-level tint that blends with file color.
    #[serde(default)]
    pub folder_ramps: Mapping<RampParams, N_FOLDER_COLOR_MODES>,
    pub hash_effect: HashTransformEffect,
    /// Per-`HashTransformEffect` strength. Switching effects preserves
    /// each effect's own intensity.
    #[serde(default)]
    pub effects: EffectsState,
    /// Polar-coords layout: when on, each cube's `(x, y)` is remapped
    /// to `(r·cosθ, r·sinθ)` around `world_center`. `polar_strength` is
    /// a 0..1 lerp between the original rectangular layout (0) and the
    /// fully wrapped polar interpretation (1). `polar_wrap_scale` is
    /// the world distance along the X axis that maps to one full
    /// 360° revolution. Independent of `hash_effect`: any effect
    /// (Ocean, Vortex, …) layers on top of the polar position.
    #[serde(default = "default_false")]
    pub polar_layout: bool,
    #[serde(default = "default_polar_strength")]
    pub polar_strength: f32,
    #[serde(default = "default_polar_wrap_scale")]
    pub polar_wrap_scale: f32,
    /// Object-side animation time (cube transforms, hash effects).
    /// Advances by `animation_speed * dt` when `animate` is true.
    pub animation_time: f32,
    #[serde(default = "default_animation_speed")]
    pub animation_speed: f32,
    /// Env-side animation time (sky time-of-day, daylight cycle).
    /// Advances by `env_speed * dt` when `env_animate` is true.
    /// Independent from `animation_time` so the user can pause object
    /// animation without stopping the sky, or vice versa.
    #[serde(default)]
    pub env_time: f32,
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
    pub materialize_mode: MaterializeMode, // Legacy, kept for compatibility
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
    /// `Some(p)` pins the palette; `None` lets `pt-mats` auto-route from
    /// the active `mat_source`.
    #[serde(default)]
    pub mat_palette: Option<Palette>,
    /// When true, the `Path` source uses hierarchical hashing so siblings
    /// cluster into nearby colors. When false, flat hash → scatter.
    #[serde(default = "default_true")]
    pub mat_path_hierarchical: bool,
    #[serde(default = "default_materialize_mix")]
    pub materialize_mix: f32, // 0=use color_mode, 1=use materialize color
    /// Post-classify overrides — two independent slots that re-point
    /// a configurable fraction of cubes at a chosen library
    /// material. Each slot has its own [`MaterialDistribution`] +
    /// seed, so the two random subsets are disjoint. Replaces the
    /// legacy `mat_allow_lights` / `mat_allow_glass` family.
    #[serde(default = "default_material_overrides")]
    pub material_overrides: [MaterialOverride; 2],
    #[serde(default)]
    pub mat_include_dirs: bool, // Allow materialization for directories
    #[serde(default = "default_mat_seed")]
    pub mat_seed: u32, // Seed for random material assignment
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
    pub pt_samples: u32,
    pub pt_samples_per_update: u32,
    pub pt_max_transmission_depth: u32,
    pub pt_dof_enabled: bool,
    pub pt_aperture: f32,
    pub pt_focus_distance: f32,
    pub pt_env_importance_sampling: bool,
    #[serde(default)]
    pub pt_sampler_mode: PtSamplerMode,
    #[serde(default = "default_true")]
    pub pt_emissive_sampling: bool,
    #[serde(default = "default_emissive_samples")]
    pub pt_emissive_samples: u32,
    #[serde(default = "default_emissive_min_weight")]
    pub pt_emissive_min_weight: f32,
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
    // Per-pixel SPP range used by the adaptive sampler is now *derived* from
    // `pt_samples` rather than configured independently:
    //   min_spp = max(pt_samples / 16, 8)
    //   max_spp = pt_samples
    // This keeps one global samples knob in charge of everything. See
    // `pt-megakernel/src/compute.rs::set_adaptive_config` callers in
    // `render-3d`.
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
    pub slice_axis: u32, // 0=X, 1=Y, 2=Z (used when slice_use_vector=false)
    #[serde(default = "default_slice_position")]
    pub slice_position: f32,
    #[serde(default = "default_slice_position_vector")]
    pub slice_position_vector: f32,
    pub slice_invert: bool,
    pub slice_use_vector: bool, // true = use arbitrary normal, false = use axis
    #[serde(default = "default_slice_normal")]
    pub slice_normal: [f32; 3], // Arbitrary slice plane normal (normalized)
    // LOD (Level of Detail)
    pub lod_enabled: bool,
    #[serde(default = "default_lod_min_size")]
    pub lod_min_screen_size: f32, // Min screen size in pixels to render
    // Camera inertia
    #[serde(default = "default_inertia_enabled")]
    pub inertia_enabled: bool,
    #[serde(default = "default_inertia_friction")]
    pub inertia_friction: f32, // Higher = faster stop (1-10 typical)
    #[serde(default = "default_inertia_cutoff")]
    pub inertia_cutoff: f32, // Stop inertia when speed is below cutoff
    // OIDN denoiser (replaces the previous à-trous filter — `pt-denoise-oidn`).
    // Mirrors `pt_denoise_oidn::OidnMode` / `pt_denoise_oidn::Quality` but kept
    // here as a string-serialised enum so `render-shared` need not depend on
    // Burn or oidn-rs.
    #[serde(default = "default_oidn_mode")]
    pub pt_oidn_mode: OidnModeOption,
    #[serde(default = "default_oidn_quality")]
    pub pt_oidn_quality: OidnQualityOption,
    /// Run the denoiser automatically once `current_spp >= target_spp` for
    /// the accumulating render.
    #[serde(default = "default_oidn_auto")]
    pub pt_oidn_auto: bool,
    /// Re-run OIDN every N accumulated samples (in addition to the
    /// target-spp auto trigger). `0` disables the periodic re-run and only
    /// the final-spp fire remains.
    #[serde(default = "default_oidn_interval")]
    pub pt_oidn_interval: u32,
    /// Firefly clamp applied to the OIDN color input. Each RGB channel
    /// is clamped to this value before the input bridge feeds OIDN.
    /// `0.0` disables clamping (raw HDR input).
    ///
    /// Why this exists: PT path-tracing produces rare extreme samples
    /// (a glancing specular bounce that catches the brightest part of
    /// the env map) that survive sample-normalization and end up in
    /// the accumulator as fireflies. OIDN's albedo+normal-guided UNet
    /// keeps high-frequency content intact, which means it smears each
    /// firefly across a halo of pixels instead of suppressing it —
    /// the splotchy "noise that grows with samples" everyone gets to
    /// hate at low SPP. Clamping just the OIDN input (not the PT
    /// accumulator) gives the denoiser a temperate signal while the
    /// underlying PT mean stays physically correct.
    ///
    /// Production default `10.0` matches Arnold's `indirect_clamp` /
    /// V-Ray's secondary GI clamp — bright enough to keep area-light
    /// + skybox contributions, low enough to suppress fireflies.
    #[serde(default = "default_oidn_clamp")]
    pub pt_oidn_clamp: f32,

    /// Replace non-finite (`NaN` / ±`Inf`) samples on the colour /
    /// albedo / normal inputs to OIDN with `0` before clamp +
    /// transfer. Mirrors the reference C++ OIDN `nan_to_zero`
    /// pre-step in every input kernel. Default `true` — strongly
    /// recommended: without it, a single bad path-tracer sample can
    /// poison the entire denoised output through the PU/exp
    /// expansion in the inverse transfer.
    #[serde(default = "default_oidn_nan_protect")]
    pub pt_oidn_nan_protect: bool,

    /// Adaptive firefly clamp: when on, the effective clamp ceiling
    /// smooth-steps from a tight `EARLY_CLAMP_FLOOR` at spp=1 up to
    /// `pt_oidn_clamp` at `ADAPTIVE_CLAMP_SPP` (currently 256). Cuts
    /// halos around lights in early previews without sacrificing
    /// dynamic range on the converged image. Default `true`.
    #[serde(default = "default_oidn_adaptive_clamp")]
    pub pt_oidn_adaptive_clamp: bool,

    /// Camera model selector: `Manual` reads the legacy raw-aperture
    /// and orbit-fov pair; `Physical` swaps in derived values from
    /// [`Self::pt_physical_camera`] (F-stop, focal length, sensor
    /// width, ISO, shutter).
    #[serde(default)]
    pub pt_camera_type: CameraType,

    /// Photographer-style camera parameters. Read when
    /// [`Self::pt_camera_type`] is `Physical`. See
    /// [`PhysicalCamera`] for the full field semantics.
    #[serde(default)]
    pub pt_physical_camera: PhysicalCamera,

    /// Display-side colour pipeline — sits between the PT accumulator
    /// and the egui texture. All knobs are pure post-process: changing
    /// them MUST fire `dirty.preset()` only, never restart PT
    /// accumulation (mirror of the denoise-interval lesson).
    ///
    /// Default = `TonemapKind::AcesFilmic`, which matches the current
    /// blit-shader behaviour bit-exactly. The IDT / LMT / RRT / ODT
    /// lanes are dormant until `tonemap == AcesFull` — they round-trip
    /// through presets immediately so saved configs survive across the
    /// later phases that actually wire them to the GPU.
    /// See `docs/aces-color-pipeline-plan.md`.
    /// OCIO-backed colour pipeline. Replaces the legacy
    /// `color_idt/lmt/rrt/odt/working/tonemap/exposure/wb/gamut`
    /// cascade that lived here before phase 11. See
    /// `crates/color-pipeline/src/lib.rs` for the data model.
    #[serde(default)]
    pub color_pipeline: color_pipeline::ColorPipelineSettings,

    /// Per-scene material library — the single source of truth for
    /// cube materials. Cubes get a `material_index` chosen by
    /// `mat_source` / `mat_distribution` (via `pt_mats::classify_to_index`)
    /// and per-cube variance is resolved at materialize-time via
    /// [`pt_material::Material::resolve_for_cube`].
    #[serde(default)]
    pub material_library: pt_material::MaterialLibrary,
}

impl Render3DOptions {
    /// World-space aperture radius the renderer should use this
    /// frame — derived from the physical camera in `Physical` mode,
    /// or the raw `pt_aperture` slider in `Manual` mode.
    pub fn effective_aperture(&self) -> f32 {
        match self.pt_camera_type {
            CameraType::Manual => self.pt_aperture,
            CameraType::Physical => self.pt_physical_camera.aperture_world(),
        }
    }

    /// Focus distance shared by both camera models. Kept as a single
    /// field on `Render3DOptions` because Ctrl-click DoF pick writes
    /// here directly, and the value's meaning is identical across
    /// modes (world-space distance from camera).
    pub fn effective_focus_distance(&self) -> f32 {
        self.pt_focus_distance
    }

    /// Horizontal field-of-view in radians. `Physical` mode derives
    /// from focal length + sensor width; `Manual` mode returns
    /// `None` so the caller falls back to its own FOV source (the
    /// orbit camera).
    pub fn effective_fov_override(&self) -> Option<f32> {
        match self.pt_camera_type {
            CameraType::Manual => None,
            CameraType::Physical => Some(self.pt_physical_camera.fov_radians()),
        }
    }

    /// Stub identity matrices for the unused `BlitParams.aces_pre /
    /// aces_post` GPU lanes. Phase 11 removed the legacy
    /// `Idt/Lmt/Rrt/Odt/Working` matrix bake — the new OCIO pipeline
    /// drives the colour transform either via the GPU LUT (mode =
    /// Ocio, tag 6) or via a shader-side curve (mode = BuiltIn,
    /// tags 0–2 / 5), neither of which reads pre/post. A follow-up
    /// commit will drop the uniforms entirely.
    pub fn aces_full_matrices(&self) -> ([[f32; 4]; 3], [[f32; 4]; 3]) {
        let id3 = [
            [1.0, 0.0, 0.0, 0.0],
            [0.0, 1.0, 0.0, 0.0],
            [0.0, 0.0, 1.0, 0.0],
        ];
        (id3, id3)
    }

    /// Pack the colour-pipeline knobs into the 4-tuple the blit
    /// shader's second `vec4` consumes (see `compute.rs::set_blit_color`).
    ///
    /// Returns `(tonemap_tag, display_exposure_ev, white_balance_norm,
    /// gamut_compress)` where:
    /// - `tonemap_tag` is `TonemapKind::gpu_tag()` (3 = AcesFilmic, the
    ///   bit-exact pre-C-2 default).
    /// - `white_balance_norm` is `target_K / 6500.0`.
    ///
    /// The C-2 GPU lane treats `AcesFull` as `AcesFilmic` for now — the
    /// IDT/LMT/RRT/ODT matrices are baked in by C-3.
    pub fn blit_color_lane(&self) -> (u32, f32, f32, f32) {
        // Phase 11 deletion: legacy exposure / white-balance /
        // gamut-compress lanes are gone. Callers that want
        // exposure or WB now stage them inside the OCIO pipeline
        // (CDL / GradingPrimary / ExposureContrastTransform). The
        // shader's color.{y, z, w} channels are passed through as
        // identity defaults (EV = 0, WB norm = 1.0, gc = 0.0) so
        // the existing uniform layout stays binary-compatible.
        (
            self.color_pipeline.resolved_tonemap_tag(),
            0.0,
            1.0,
            0.0,
        )
    }

    /// Scene-linear exposure multiplier applied at display + as the
    /// OIDN `input_scale` override. `1.0` in `Manual` mode so the
    /// existing autoexposure / display behaviour is preserved bit-
    /// exactly.
    pub fn effective_exposure_multiplier(&self) -> f32 {
        match self.pt_camera_type {
            CameraType::Manual => 1.0,
            CameraType::Physical => self.pt_physical_camera.exposure_multiplier(),
        }
    }
}

/// String-serialised mirror of `pt_denoise_oidn::OidnMode`. Default is the
/// production target (color+albedo+normal) so a fresh install denoises out
/// of the box once accumulation completes.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
pub enum OidnModeOption {
    Off,
    Color,
    ColorAlbedo,
    #[default]
    ColorAlbedoNormal,
}

/// Model-size selector for OIDN. Maps onto `oidn_rs::Quality` internally:
/// - `Large`  → `Quality::High`  → try `_large` weights, fallback to base
/// - `Base`   → `Quality::Balanced` → base weights only
/// - `Small`  → `Quality::Fast`  → try `_small` weights, fallback to base
///
/// Names match what the user actually controls (which TZA size to load),
/// not abstract "quality". `Large` only matters for prefilter / clean-aux
/// models (Intel doesn't ship `_large` variants of the main color-denoise
/// network); `Small` halves params on the main network.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
pub enum OidnQualityOption {
    Large,
    #[default]
    Base,
    Small,
}

/// Working colour space the display pipeline operates in. Today the
/// path tracer always writes plain linear-sRGB, so anything other than
/// `LinearSRGB` is a forward-looking knob — wiring lands in a later
/// phase (see `docs/aces-color-pipeline-plan.md`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
pub enum ColorWorkingSpace {
    LinearSRGB,
    #[default]
    ACEScg,
    ACES2065_1,
}

/// Display-side tonemap selector. `AcesFilmic` is the default so this
/// whole block is purely additive: nothing about the rendered frame
/// changes until the user actively switches mode.
///
/// - `None`        : clamp [0,1], no curve, no exposure
/// - `Linear`      : exposure + WB only, no curve
/// - `Reinhard`    : `x / (1 + x)` legacy path
/// - `AcesFilmic`  : current Narkowicz fit (DEFAULT — bit-exact today)
/// - `AcesFull`    : unlocks the IDT/LMT/RRT/ODT lanes (vfx-rs backed)
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
pub enum TonemapKind {
    None,
    Linear,
    Reinhard,
    #[default]
    AcesFilmic,
    AcesFull,
}

impl TonemapKind {
    /// Stable numeric tag for the GPU side. WGSL `switch` consumes this
    /// directly. Values are fixed by ABI — never renumber without bumping
    /// the blit shader at the same time.
    pub const fn gpu_tag(self) -> u32 {
        match self {
            TonemapKind::None => 0,
            TonemapKind::Linear => 1,
            TonemapKind::Reinhard => 2,
            TonemapKind::AcesFilmic => 3,
            TonemapKind::AcesFull => 4,
        }
    }
}

impl AcesOdt {
    /// Stable numeric tag for the GPU side. Used by `blit.wgsl` to pick
    /// the right OETF for the active ODT (sRGB 1/2.2 vs PQ for the HDR
    /// Rec.2020 target). Values are ABI — never renumber without bumping
    /// the shader.
    pub const fn gpu_tag(self) -> u32 {
        match self {
            AcesOdt::Srgb100nits => 0,
            AcesOdt::Rec709 => 1,
            AcesOdt::Rec2020_1000nits => 2,
            AcesOdt::P3D65 => 3,
            AcesOdt::DciP3 => 4,
            AcesOdt::SrgbHdrSim => 5,
        }
    }
}

/// ACEScg (AP1) → sRGB matrix with Bradford D60→D65 CAT.
///
/// Row-major. Values are the canonical ACES 1.0 RRT_SAT⊗ODT_sRGB
/// composite — also matches `vfx-color::aces::acescg_to_srgb_matrix()`
/// to <1e-4. Hard-coded for now to avoid pulling vfx-color as a dep
/// just for one matrix pair; the constants are short-lived (C-5+ will
/// either swap on ODT or move to vfx-color for Rec.2020 / P3-D65).
pub const ACESCG_TO_SRGB: [[f32; 3]; 3] = [
    [1.70505, -0.62179, -0.08326],
    [-0.13026, 1.14080, -0.01055],
    [-0.02400, -0.12897, 1.15297],
];

/// sRGB → ACEScg (AP1) matrix with Bradford D65→D60 CAT. Inverse of
/// [`ACESCG_TO_SRGB`] (to numerical precision).
pub const SRGB_TO_ACESCG: [[f32; 3]; 3] = [
    [0.61314, 0.33952, 0.04734],
    [0.07012, 0.91634, 0.01354],
    [0.02061, 0.10957, 0.86983],
];

/// ACEScg (AP1) → Rec.2020 primaries with Bradford D60→D65 CAT.
/// For HDR / wide-gamut displays. Matches `vfx-color` output to <1e-4.
pub const ACESCG_TO_REC2020: [[f32; 3]; 3] = [
    [0.69545, 0.14068, 0.16387],
    [0.04434, 0.85968, 0.09598],
    [-0.00553, 0.00404, 1.00149],
];

/// ACEScg (AP1) → P3-D65 primaries with Bradford D60→D65 CAT.
/// Display P3 — modern wide-gamut desktops (Apple Display P3, OLEDs).
pub const ACESCG_TO_P3D65: [[f32; 3]; 3] = [
    [1.02901, -0.02164, -0.00737],
    [-0.04210, 1.06250, -0.02040],
    [-0.00203, -0.07601, 1.07804],
];

/// ACEScg (AP1) → ACES2065-1 (AP0) primaries, no CAT (both AP1 and AP0
/// share the ACES D60 white point). Canonical ACES constants.
pub const AP1_TO_AP0: [[f32; 3]; 3] = [
    [0.695452, 0.140679, 0.163869],
    [0.044794, 0.859671, 0.095535],
    [-0.005525, 0.004025, 1.001_5],
];

/// ACES2065-1 (AP0) → ACEScg (AP1) primaries, no CAT. Inverse of
/// [`AP1_TO_AP0`].
pub const AP0_TO_AP1: [[f32; 3]; 3] = [
    [1.451439, -0.236_51, -0.214929],
    [-0.076553, 1.176229, -0.099677],
    [0.008316, -0.006032, 0.997716],
];

/// ACEScg (AP1) → DCI-P3 primaries with Bradford D60→DCI CAT.
/// Theatrical projection target. Different white point from Display P3
/// (DCI white ≈ (0.314, 0.351) vs D65 ≈ (0.3127, 0.3290)).
pub const ACESCG_TO_DCIP3: [[f32; 3]; 3] = [
    [1.04391, -0.04437, 0.00045],
    [-0.04158, 1.04408, -0.00250],
    [0.00033, -0.04020, 1.03987],
];

/// Identity 3×3 matrix — used when an IDT or ODT lane is `None`
/// (passthrough) or unimplemented.
pub const MAT3_IDENTITY: [[f32; 3]; 3] =
    [[1.0, 0.0, 0.0], [0.0, 1.0, 0.0], [0.0, 0.0, 1.0]];

/// Build a saturation matrix using Rec.709 luminance coefficients.
/// `s = 1.0` returns identity; `s < 1` desaturates; `s > 1` saturates.
/// The matrix lives in the ACEScg working space — applied after the
/// IDT and before the RRT curve.
pub fn saturation_matrix(s: f32) -> [[f32; 3]; 3] {
    // Rec.709 luma coefficients (Y' = 0.2126 R + 0.7152 G + 0.0722 B).
    // Mixing in ACEScg with Rec.709 luma is an approximation — proper
    // ACEScg uses different coefficients, but the visual difference at
    // these subtle LMT strengths (≤1.2) is below quantisation noise.
    let lr = 0.2126;
    let lg = 0.7152;
    let lb = 0.0722;
    let inv = 1.0 - s;
    [
        [s + inv * lr, inv * lg, inv * lb],
        [inv * lr, s + inv * lg, inv * lb],
        [inv * lr, inv * lg, s + inv * lb],
    ]
}

/// Diagonal per-channel tint as a 3×3 matrix. Tints R/G/B
/// independently — a non-uniform white-point nudge useful as a
/// cheap building block for creative LMTs ("warm", "cool", ...).
pub fn diag_tint(t: [f32; 3]) -> [[f32; 3]; 3] {
    [
        [t[0], 0.0, 0.0],
        [0.0, t[1], 0.0],
        [0.0, 0.0, t[2]],
    ]
}

/// Row-major 3×3 matrix product: `a * b`.
pub fn mat3_mul(a: &[[f32; 3]; 3], b: &[[f32; 3]; 3]) -> [[f32; 3]; 3] {
    let mut out = [[0.0_f32; 3]; 3];
    for (i, row) in out.iter_mut().enumerate() {
        for (j, cell) in row.iter_mut().enumerate() {
            *cell = a[i][0] * b[0][j] + a[i][1] * b[1][j] + a[i][2] * b[2][j];
        }
    }
    out
}

/// Pack a row-major 3×3 matrix into the column-major
/// `array<vec4<f32>, 3>` layout WGSL std140 uniforms consume.
///
/// Each output `vec4` is a column of the matrix with `.w = 0.0` for
/// alignment padding. WGSL's `mat3x3` reads columns sequentially, so
/// CPU stores `[col0.xyzw, col1.xyzw, col2.xyzw]` (12 floats, 48 bytes).
pub fn mat3_to_std140_columns(m: &[[f32; 3]; 3]) -> [[f32; 4]; 3] {
    [
        [m[0][0], m[1][0], m[2][0], 0.0], // column 0
        [m[0][1], m[1][1], m[2][1], 0.0], // column 1
        [m[0][2], m[1][2], m[2][2], 0.0], // column 2
    ]
}

/// Input Device Transform — maps scene-referred RGB into the ACES
/// working space (AP0 / AP1). `SrgbToAp1` is the production default
/// once `AcesFull` is selected; until then this field is dormant.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
pub enum AcesIdt {
    None,
    #[default]
    SrgbToAp1,
    Rec709ToAp1,
    Ap1Passthrough,
}

/// Look Modification Transform — optional creative grade between IDT
/// and RRT. `None` is the neutral default; named looks combine a
/// saturation scalar with an optional per-channel tint to give the
/// user a small catalogue of cinematic looks without committing to
/// the full ACES LMT XML pipeline.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
pub enum AcesLmt {
    #[default]
    None,
    /// +5 % saturation. Subtle bump.
    Neutral,
    /// +15 % saturation. Cinematic punchy look.
    Punchy,
    /// −5 % saturation + warm tint (R↑, B↓). Sunset / candlelight feel.
    Warm,
    /// −5 % saturation + cool tint (B↑, R↓). Moonlight / night feel.
    Cool,
    /// −30 % saturation + slight luma lift. Bleach-bypass / desaturated
    /// high-contrast look.
    Bleach,
    /// −20 % saturation + heavy warm tint + green damp. Pulled-back
    /// vintage film look.
    Vintage,
}

/// Reference Rendering Transform variant. ACES 1.x ships two; `Off`
/// short-circuits the curve, which is useful for debugging the IDT/ODT
/// matrices in isolation.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
pub enum AcesRrt {
    #[default]
    Standard,
    A1_1,
    Off,
}

impl AcesRrt {
    /// Tag value packed into `BlitParams.exposure.z` so the shader
    /// can switch the filmic curve at runtime. The order is pinned —
    /// the WGSL `switch` and the tests in this module both depend on
    /// these exact values, so adding a new variant must extend the
    /// tail rather than reshuffle.
    pub const fn gpu_tag(self) -> u32 {
        match self {
            AcesRrt::Standard => 0,
            AcesRrt::A1_1 => 1,
            AcesRrt::Off => 2,
        }
    }
}

/// Output Device Transform — display-referred target. Must agree with
/// the swapchain surface format (e.g. selecting `Srgb100nits` while the
/// surface is `Rgba8UnormSrgb` means we must skip the final OETF to
/// avoid double-encoding).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
pub enum AcesOdt {
    #[default]
    Srgb100nits,
    Rec709,
    Rec2020_1000nits,
    P3D65,
    DciP3,
    SrgbHdrSim,
}

fn default_oidn_mode() -> OidnModeOption {
    OidnModeOption::ColorAlbedoNormal
}
fn default_oidn_quality() -> OidnQualityOption {
    OidnQualityOption::Base
}
fn default_oidn_auto() -> bool {
    true
}
fn default_oidn_interval() -> u32 {
    128
}
fn default_oidn_clamp() -> f32 {
    10.0
}
fn default_oidn_nan_protect() -> bool {
    true
}
fn default_oidn_adaptive_clamp() -> bool {
    true
}
fn default_polar_strength() -> f32 {
    1.0
}
fn default_polar_wrap_scale() -> f32 {
    1024.0
}

fn default_lod_min_size() -> f32 {
    2.0
}
fn default_true() -> bool {
    true
}
fn default_false() -> bool {
    false
}
fn default_materialize_mix() -> f32 {
    1.0
}
fn default_material_overrides() -> [MaterialOverride; 2] {
    // Two overrides start disabled with disjoint seeds so the
    // moment a user enables both, they don't pile on the same
    // cubes. Seed #2 = #1 ^ 0xDEAD_BEEF — pure bit-twiddle, no
    // semantic meaning.
    let a = MaterialOverride::default();
    let b = MaterialOverride {
        seed: a.seed ^ 0xDEAD_BEEF,
        ..MaterialOverride::default()
    };
    [a, b]
}
fn default_mat_seed() -> u32 {
    2654435761
}
fn default_quant_levels() -> u32 {
    5
}
fn default_band_count() -> u32 {
    8
}
fn default_spatial_scale() -> f32 {
    0.01
}
fn default_folder_tint() -> f32 {
    0.0
}
fn default_inertia_enabled() -> bool {
    true
}
fn default_inertia_friction() -> f32 {
    5.0
}
fn default_inertia_cutoff() -> f32 {
    0.001
}
fn default_restir_m_max() -> u32 {
    30
}
fn default_env_speed() -> f32 {
    1.0
}
fn default_svo_resolution() -> u32 {
    64
}
fn default_slice_position() -> f32 {
    0.0
}
fn default_slice_position_vector() -> f32 {
    0.0
}
fn default_slice_normal() -> [f32; 3] {
    [0.0, 1.0, 0.0]
} // Default: Y-up
fn default_adaptive_variance() -> f32 {
    0.001
}
fn default_adaptive_interval() -> u32 {
    4
}
fn default_spectral_samples() -> u32 {
    2
}
fn default_emissive_samples() -> u32 {
    1
}
fn default_emissive_min_weight() -> f32 {
    0.001
}

impl Render3DOptions {
    /// Strength of the currently selected hash effect. Reads from the
    /// per-variant `effects` map so switching effects preserves each
    /// variant's strength.
    pub fn active_hash_strength(&self) -> f32 {
        self.effects
            .hash_per_variant
            .get(self.hash_effect as usize)
            .strength
    }

    /// Speed multiplier of the currently selected hash effect, applied
    /// on top of `animation_speed`. Per-variant so switching effects
    /// preserves each variant's pace.
    pub fn active_hash_speed(&self) -> f32 {
        self.effects
            .hash_per_variant
            .get(self.hash_effect as usize)
            .speed
    }

    /// Effective animation clock for the current hash effect:
    /// `animation_time * effect_speed`. Consumers that drive cube
    /// transforms read this so each effect has an independent feel.
    pub fn active_hash_time(&self) -> f32 {
        self.animation_time * self.active_hash_speed()
    }
}

impl Default for Render3DOptions {
    fn default() -> Self {
        Self {
            height_mode: CubeHeightMode::FileSize,
            height_curves: Mapping::default(),
            color_mode: ColorMode::FileType,
            color_ramps: Mapping::default(),
            folder_color_mode: FolderColorMode::Depth,
            folder_tint: default_folder_tint(),
            folder_ramps: Mapping::default(),
            hash_effect: HashTransformEffect::Pulse,
            effects: EffectsState::default(),
            polar_layout: false,
            polar_strength: default_polar_strength(),
            polar_wrap_scale: default_polar_wrap_scale(),
            animation_time: 0.0,
            animation_speed: 3.0,
            env_time: 0.0,
            animate: false,
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
            mat_palette: None,
            mat_path_hierarchical: true,
            materialize_mix: 1.0,
            material_overrides: default_material_overrides(),
            mat_include_dirs: false,
            mat_seed: default_mat_seed(),
            env_map_intensity: 1.0,
            env_map_rotation: 0.0,
            env_map_enabled: true,
            env_map_visible: true,
            env_map_path: Some(std::path::PathBuf::from("data/uffizi-large.hdr")),
            env_animate: false,
            env_speed: 1.0,
            background_color: [0.1, 0.1, 0.1],
            path_tracing: true,
            pt_max_bounces: 4,
            pt_samples: 3500,
            pt_samples_per_update: 25,
            pt_max_transmission_depth: 8,
            pt_dof_enabled: true,
            pt_aperture: 2.0,
            pt_focus_distance: 500.0,
            pt_env_importance_sampling: true,
            pt_sampler_mode: PtSamplerMode::default(),
            pt_emissive_sampling: true,
            pt_emissive_samples: default_emissive_samples(),
            pt_emissive_min_weight: default_emissive_min_weight(),
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
            // OIDN denoiser defaults: production-grade out of the box.
            pt_oidn_mode: OidnModeOption::ColorAlbedoNormal,
            pt_oidn_quality: OidnQualityOption::Base,
            pt_oidn_auto: true,
            pt_oidn_interval: 128,
            pt_oidn_clamp: 10.0,
            pt_oidn_nan_protect: true,
            pt_oidn_adaptive_clamp: true,
            pt_camera_type: CameraType::Physical,
            pt_physical_camera: PhysicalCamera::default(),
            // Colour pipeline defaults: tonemap = current blit shader's
            // ACES Filmic so this whole new block is a no-op until the
            // user actually flips a control. Other lanes are seeded with
            // production-grade values so flipping to `AcesFull` lands on
            // a sensible sRGB / 100 nits view without further setup.
            color_pipeline: color_pipeline::ColorPipelineSettings::default(),
            material_library: pt_material::MaterialLibrary::default(),
        }
    }
}

#[cfg(test)]
#[allow(clippy::needless_range_loop)]
mod tests {
    use super::{
        AcesIdt, AcesOdt, PtSamplerMode, Render3DOptions, TonemapKind, ACESCG_TO_SRGB,
        MAT3_IDENTITY, SRGB_TO_ACESCG,
    };

    /// Apply a row-major 3×3 to a column vector.
    fn mul3(m: &[[f32; 3]; 3], v: [f32; 3]) -> [f32; 3] {
        [
            m[0][0] * v[0] + m[0][1] * v[1] + m[0][2] * v[2],
            m[1][0] * v[0] + m[1][1] * v[1] + m[1][2] * v[2],
            m[2][0] * v[0] + m[2][1] * v[1] + m[2][2] * v[2],
        ]
    }

    #[test]
    fn aces_matrices_round_trip_white() {
        // Equal-energy gray (1,1,1) is the most aggressive round-trip
        // test for matrix pairs because chromatic-adaptation errors
        // show up as a tint. Tolerance is loose because the constants
        // are quoted to 5 decimals.
        let v: [f32; 3] = [1.0, 1.0, 1.0];
        let ap1 = mul3(&SRGB_TO_ACESCG, v);
        let rt = mul3(&ACESCG_TO_SRGB, ap1);
        for c in 0..3 {
            assert!(
                (rt[c] - v[c]).abs() < 1e-3,
                "AP1 round-trip drift on channel {c}: {} vs {} (delta {})",
                rt[c],
                v[c],
                rt[c] - v[c]
            );
        }
    }

    #[test]
    fn aces_odt_gpu_tags_are_stable() {
        // ABI-fixed tags consumed by `blit.wgsl` — never renumber.
        assert_eq!(AcesOdt::Srgb100nits.gpu_tag(), 0);
        assert_eq!(AcesOdt::Rec709.gpu_tag(), 1);
        assert_eq!(AcesOdt::Rec2020_1000nits.gpu_tag(), 2);
        assert_eq!(AcesOdt::P3D65.gpu_tag(), 3);
        assert_eq!(AcesOdt::DciP3.gpu_tag(), 4);
        assert_eq!(AcesOdt::SrgbHdrSim.gpu_tag(), 5);
    }

    #[test]
    fn wider_gamut_odt_matrices_selected_correctly() {
        use super::{ACESCG_TO_DCIP3, ACESCG_TO_P3D65, ACESCG_TO_REC2020};
        let cases = [
            (AcesOdt::Rec2020_1000nits, ACESCG_TO_REC2020[0][0]),
            (AcesOdt::P3D65, ACESCG_TO_P3D65[0][0]),
            (AcesOdt::DciP3, ACESCG_TO_DCIP3[0][0]),
        ];
        for (odt, expected_00) in cases {
            let opts = Render3DOptions {
                color_odt: odt,
                ..Default::default()
            };
            let (_pre, post) = opts.aces_full_matrices();
            // post[0] is column 0 of the std140-packed matrix → first
            // element is (0,0) of the original row-major matrix.
            assert!(
                (post[0][0] - expected_00).abs() < 1e-6,
                "ODT={odt:?}: post[0][0]={} ≠ expected {}",
                post[0][0],
                expected_00,
            );
        }
    }

    #[test]
    fn working_space_changes_aces_pre_post_matrices() {
        // Three working spaces should produce three visibly different
        // (pre, post) pairs. LinearSRGB → identity pair; ACEScg →
        // sRGB↔AP1; ACES2065-1 → wrapped through AP0 (different from
        // the AP1 pair).
        use super::ColorWorkingSpace;

        let mut opts = Render3DOptions {
            color_tonemap: TonemapKind::AcesFull,
            color_working: ColorWorkingSpace::LinearSRGB,
            ..Default::default()
        };
        let (pre_lin, post_lin) = opts.aces_full_matrices();
        // LinearSRGB → identity pre/post.
        assert!((pre_lin[0][0] - 1.0).abs() < 1e-6, "Linear pre[0][0] ≠ 1");
        assert!((pre_lin[1][0]).abs() < 1e-6, "Linear pre off-diag ≠ 0");
        assert!((post_lin[0][0] - 1.0).abs() < 1e-6, "Linear post[0][0] ≠ 1");

        opts.color_working = ColorWorkingSpace::ACEScg;
        let (pre_cg, post_cg) = opts.aces_full_matrices();
        // ACEScg → pre[0][0] = SRGB_TO_ACESCG[0][0] = 0.61314.
        assert!(
            (pre_cg[0][0] - 0.61314).abs() < 1e-4,
            "ACEScg pre[0][0] = {} ≠ 0.61314",
            pre_cg[0][0]
        );
        // Differs from Linear.
        assert!(
            (pre_cg[0][0] - pre_lin[0][0]).abs() > 0.1,
            "ACEScg pre should differ from Linear pre"
        );

        opts.color_working = ColorWorkingSpace::ACES2065_1;
        let (pre_2065, post_2065) = opts.aces_full_matrices();
        // AP0 routing → both pre and post differ from the AP1 case.
        // (AP1→AP0 ≈ 0.6955 for [0][0], times sRGB→AP1 ≈ 0.6131 →
        // composite is around 0.46 or so, distinctly less than 0.6131.)
        assert!(
            (pre_2065[0][0] - pre_cg[0][0]).abs() > 0.05,
            "ACES2065-1 pre should differ from ACEScg pre by >0.05; got {} vs {}",
            pre_2065[0][0],
            pre_cg[0][0]
        );
        assert!(
            (post_2065[0][0] - post_cg[0][0]).abs() > 0.05,
            "ACES2065-1 post should differ from ACEScg post; got {} vs {}",
            post_2065[0][0],
            post_cg[0][0]
        );
    }

    #[test]
    fn aces_full_matrices_default_returns_srgb_pair() {
        // Default options select IDT=SrgbToAp1 + LMT=None + ODT=Srgb100nits,
        // so `aces_full_matrices()` must hand back the sRGB↔AP1 pair —
        // never identity (identity would degrade AcesFull to AcesFilmic).
        let opts = Render3DOptions::default();
        let (pre, post) = opts.aces_full_matrices();
        // pre[0] is the first WGSL column = SRGB_TO_ACESCG row 0
        // ([0.61314, 0.07012, 0.02061], 0.0). Check the (0,0).
        assert!(
            (pre[0][0] - SRGB_TO_ACESCG[0][0]).abs() < 1e-6,
            "pre matrix not SRGB_TO_ACESCG"
        );
        assert!(
            (post[0][0] - ACESCG_TO_SRGB[0][0]).abs() < 1e-6,
            "post matrix not ACESCG_TO_SRGB"
        );
    }

    #[test]
    fn aces_full_matrices_passthrough_returns_identity() {
        // IDT=Ap1Passthrough means "PT already writes ACEScg" → pre is
        // identity. Pair this with a normal ODT and we get the post-
        // only behaviour (working space = AP1, no IDT applied).
        let opts = Render3DOptions {
            color_tonemap: TonemapKind::AcesFull,
            color_idt: AcesIdt::Ap1Passthrough,
            color_odt: AcesOdt::Srgb100nits,
            ..Default::default()
        };
        let (pre, _post) = opts.aces_full_matrices();
        assert!(
            (pre[0][0] - MAT3_IDENTITY[0][0]).abs() < 1e-6
                && pre[0][1].abs() < 1e-6
                && pre[0][2].abs() < 1e-6,
            "pre matrix should be identity for Ap1Passthrough IDT"
        );
    }

    #[test]
    fn saturation_matrix_identity_at_unity() {
        let m = super::saturation_matrix(1.0);
        // Identity along the diagonal, zero off-diagonal — except for
        // float rounding at the Rec.709 coefficients.
        for i in 0..3 {
            for j in 0..3 {
                let want = if i == j { 1.0 } else { 0.0 };
                assert!(
                    (m[i][j] - want).abs() < 1e-6,
                    "saturation(1.0)[{i}][{j}] = {} not {}",
                    m[i][j],
                    want
                );
            }
        }
    }

    #[test]
    fn saturation_matrix_preserves_neutral_gray() {
        // (1,1,1) should map to (1,1,1) for any saturation value —
        // achromatic input has zero chroma to scale.
        let v: [f32; 3] = [1.0, 1.0, 1.0];
        for s in [0.5, 1.0, 1.5] {
            let m = super::saturation_matrix(s);
            let out = mul3(&m, v);
            for c in 0..3 {
                assert!(
                    (out[c] - 1.0).abs() < 1e-5,
                    "saturation({s}) on gray channel {c}: {} drifted",
                    out[c]
                );
            }
        }
    }

    #[test]
    fn lmt_punchy_changes_pre_matrix() {
        // Default opts → LMT=None → pre == SRGB_TO_ACESCG exactly.
        // Switch to Punchy → pre must differ from SRGB_TO_ACESCG on
        // chromatic channels (gray is preserved by the saturation
        // matrix, so the diff shows on the off-diagonal).
        let default_opts = Render3DOptions::default();
        let (pre_none, _) = default_opts.aces_full_matrices();

        let punchy = Render3DOptions {
            color_lmt: super::AcesLmt::Punchy,
            ..Default::default()
        };
        let (pre_punchy, _) = punchy.aces_full_matrices();

        // At least one of the 12 floats must differ by >1% — saturation
        // 1.15 lifts colour channels meaningfully.
        let mut diff_count = 0;
        for col in 0..3 {
            for row in 0..3 {
                if (pre_none[col][row] - pre_punchy[col][row]).abs() > 0.01 {
                    diff_count += 1;
                }
            }
        }
        assert!(
            diff_count >= 1,
            "Punchy LMT should perturb the pre matrix; got {diff_count} cells with >1% delta"
        );
    }

    #[test]
    fn effective_gamut_compress_auto_lookup() {
        // Auto on + narrow-gamut ODT → full strength.
        let mut opts = Render3DOptions {
            color_gamut_compress_auto: true,
            color_gamut_compress: 0.42, // manual override should be ignored
            color_odt: AcesOdt::Srgb100nits,
            ..Default::default()
        };
        assert_eq!(opts.effective_gamut_compress(), 1.0);

        // Auto on + wide-gamut ODT → no compression.
        opts.color_odt = AcesOdt::Rec2020_1000nits;
        assert_eq!(opts.effective_gamut_compress(), 0.0);

        // Auto off → respect manual slider, clamped to [0,1].
        opts.color_gamut_compress_auto = false;
        opts.color_gamut_compress = 0.42;
        assert!((opts.effective_gamut_compress() - 0.42).abs() < 1e-6);
        opts.color_gamut_compress = -0.5;
        assert_eq!(opts.effective_gamut_compress(), 0.0);
        opts.color_gamut_compress = 2.0;
        assert_eq!(opts.effective_gamut_compress(), 1.0);
    }

    #[test]
    fn render_3d_options_deserialize_defaults() {
        let json = "{}";
        let opts: Render3DOptions = serde_json::from_str(json).expect("deserialize");
        let defaults = Render3DOptions::default();
        assert_eq!(opts.pt_max_bounces, defaults.pt_max_bounces);
        assert_eq!(opts.pt_samples, defaults.pt_samples);
        assert_eq!(opts.pt_gpu_bvh, defaults.pt_gpu_bvh);
        assert_eq!(opts.pt_spectral_mode, defaults.pt_spectral_mode);
        assert_eq!(opts.pt_spectral_samples, defaults.pt_spectral_samples);
        assert_eq!(opts.pt_spectral_dispersion, defaults.pt_spectral_dispersion);
        assert_eq!(opts.pt_sampler_mode, defaults.pt_sampler_mode);
    }

    #[test]
    fn render_3d_pt_sampler_roundtrip() {
        let opts = Render3DOptions {
            pt_sampler_mode: PtSamplerMode::R2,
            pt_emissive_sampling: true,
            pt_emissive_samples: 4,
            pt_emissive_min_weight: 0.01,
            ..Default::default()
        };

        let json = serde_json::to_string(&opts).expect("serialize");
        let restored: Render3DOptions = serde_json::from_str(&json).expect("deserialize");

        assert_eq!(restored.pt_sampler_mode, PtSamplerMode::R2);
        assert!(restored.pt_emissive_sampling);
        assert_eq!(restored.pt_emissive_samples, 4);
        assert_eq!(restored.pt_emissive_min_weight, 0.01);
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
            yaw: 0.0,   // Front view (matches 2D)
            pitch: 0.0, // Looking straight ahead
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
    fn fit_distance_for_aspect(width: f32, height: f32, vertical_fov: f32, aspect: f32) -> f32 {
        let half_h = height.max(1.0) * 0.5;
        let half_w = width.max(1.0) * 0.5;
        let tan_half = (vertical_fov * 0.5).tan().max(0.0001);
        let aspect = aspect.max(0.0001);
        let fit_h = half_h / tan_half;
        let fit_w = half_w / (tan_half * aspect);
        fit_h.max(fit_w)
    }

    /// Orbit the camera (left mouse drag) - non-inertia version
    #[allow(dead_code)]
    pub fn orbit(&mut self, delta_x: f32, delta_y: f32) {
        let sensitivity = 0.005;
        self.yaw += delta_x * sensitivity;
        self.pitch = (self.pitch + delta_y * sensitivity).clamp(
            -std::f32::consts::FRAC_PI_2 + 0.1,
            std::f32::consts::FRAC_PI_2 - 0.1,
        );
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

    /// Zoom the camera (right mouse drag or scroll) — exponential, no inertia.
    ///
    /// Each unit of `delta` multiplies `distance` by `exp(delta * k)`, so
    /// zoom feels uniform across the whole 10..5000 range: one wheel tick
    /// is always the same *ratio*, not the same absolute distance. The
    /// previous linear approximation (`1 + delta * k`) degenerated for
    /// large negative deltas — factor could hit 0 or go negative,
    /// which was only saved by the post-clamp.
    pub fn zoom(&mut self, delta: f32) {
        let factor = (delta * 0.001).exp();
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

    /// Get projection matrix.
    ///
    /// Uses **reversed-Z with infinite far plane**: near maps to NDC depth
    /// 1.0, infinity maps to 0.0. This makes f32 depth-buffer precision
    /// distribute logarithmically across the entire depth range instead of
    /// piling up near 1.0. Eliminates z-fighting / strobing on the far
    /// background (the previous near=0.1 / far=100000 ratio of 1e6 left
    /// far values essentially indistinguishable in f32).
    ///
    /// Callers using this matrix:
    /// * pipelines must clear depth to **0.0** (not 1.0) and use
    ///   `CompareFunction::Greater(Equal)` (not `Less(Equal)`).
    /// * ray-picking from NDC must place the near point at `ndc.z = 1.0`
    ///   and the far point at `ndc.z = 0.0`.
    ///
    /// `self.far` is kept on the struct for backwards-compat with existing
    /// serde presets but is ignored — there is no finite far plane.
    pub fn projection_matrix(&self, aspect: f32) -> Mat4 {
        Mat4::perspective_infinite_reverse_rh(self.fov, aspect, self.near)
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

    /// Zoom with inertia — exponential in distance.
    ///
    /// `distance_velocity` is interpreted as a *log-space* rate
    /// (`d/dt of ln(distance)`), so the per-frame integration in
    /// `update_inertia` multiplies `distance` by `exp(velocity * dt)`.
    /// Same constant coefficient regardless of current zoom level: the
    /// wheel feels identical close-up and far-away, no “sluggish near
    /// the cubes, runaway far out” effect.
    pub fn zoom_inertia(&mut self, delta: f32) {
        self.distance_velocity += delta * 0.0005;
    }

    /// Update camera with inertia (call each frame)
    /// Returns true if camera is still moving
    pub fn update_inertia(&mut self, dt: f32, friction: f32, cutoff: f32) -> bool {
        let decay = (-friction * dt).exp();
        let threshold = cutoff.max(0.000001);

        // Apply velocities
        self.yaw += self.yaw_velocity * dt;
        self.pitch = (self.pitch + self.pitch_velocity * dt).clamp(
            -std::f32::consts::FRAC_PI_2 + 0.1,
            std::f32::consts::FRAC_PI_2 - 0.1,
        );
        // Distance lives in log-space for inertia integration so the wheel
        // feels uniform across the whole zoom range (see `zoom_inertia`).
        // `distance_velocity` is the log-rate; multiplying by `exp(v*dt)`
        // each frame is the proper time-stepped solution.
        self.distance = (self.distance * (self.distance_velocity * dt).exp())
            .clamp(10.0, 5000.0);
        self.target += self.target_velocity * dt;

        // Apply friction
        self.yaw_velocity *= decay;
        self.pitch_velocity *= decay;
        self.distance_velocity *= decay;
        self.target_velocity *= decay;

        // Snap to rest below threshold to avoid jitter
        if self.yaw_velocity.abs() < threshold {
            self.yaw_velocity = 0.0;
        }
        if self.pitch_velocity.abs() < threshold {
            self.pitch_velocity = 0.0;
        }
        if self.distance_velocity.abs() < threshold {
            self.distance_velocity = 0.0;
        }
        if self.target_velocity.length() < threshold {
            self.target_velocity = Vec3::ZERO;
        }

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
        if !self.animating {
            return false;
        }

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
        let aspect = width.max(1.0) / height.max(1.0);
        self.set_front_view_for_viewport(width, height, aspect);
    }

    /// Set front-view for a scene whose dimensions are independent from the viewport.
    pub fn set_front_view_for_viewport(&mut self, width: f32, height: f32, viewport_aspect: f32) {
        self.yaw = 0.0;
        self.pitch = 0.0;
        self.target = Vec3::new(width / 2.0, -(height / 2.0), 0.0);
        self.distance = Self::fit_distance_for_aspect(width, height, self.fov, viewport_aspect);
    }

    /// Set front-view with animation (full reset including rotation)
    pub fn animate_to_front_view(&mut self, width: f32, height: f32) {
        let aspect = width.max(1.0) / height.max(1.0);
        self.animate_to_front_view_for_viewport(width, height, aspect);
    }

    /// Set front-view with animation for a scene whose dimensions are independent from the viewport.
    pub fn animate_to_front_view_for_viewport(
        &mut self,
        width: f32,
        height: f32,
        viewport_aspect: f32,
    ) {
        let target = Vec3::new(width / 2.0, -(height / 2.0), 0.0);
        let distance = Self::fit_distance_for_aspect(width, height, self.fov, viewport_aspect);
        self.animate_to(0.0, 0.0, distance, target);
    }

    /// Zoom to fit scene without changing rotation
    pub fn zoom_to_fit_scene(&mut self, width: f32, height: f32) {
        let aspect = width.max(1.0) / height.max(1.0);
        self.zoom_to_fit_scene_for_viewport(width, height, aspect);
    }

    /// Zoom to fit scene without changing rotation, using the current viewport aspect.
    pub fn zoom_to_fit_scene_for_viewport(
        &mut self,
        width: f32,
        height: f32,
        viewport_aspect: f32,
    ) {
        let target = Vec3::new(width / 2.0, -(height / 2.0), 0.0);
        let distance = Self::fit_distance_for_aspect(width, height, self.fov, viewport_aspect);
        self.animate_zoom_to(distance, target);
    }
}

/// Compute a deterministic hash from a string (for per-cube transforms)
pub fn name_hash(name: &str) -> u32 {
    name.bytes()
        .fold(0u32, |acc, b| acc.wrapping_mul(31).wrapping_add(b as u32))
}

/// Derive secondary hash without string allocation
#[inline]
fn hash_derive(hash: u32, salt: u32) -> u32 {
    hash.wrapping_mul(1664525)
        .wrapping_add(salt)
        .wrapping_mul(1013904223)
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
            let disp = Vec3::new(
                ax.sin() * strength * 3.0,
                ay.sin() * strength * 3.0,
                az.cos() * strength * 2.0,
            );
            (
                disp,
                glam::Quat::from_euler(glam::EulerRot::XYZ, ax, ay, az),
            )
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
            (
                offset,
                glam::Quat::from_euler(glam::EulerRot::XYZ, ax, ay, 0.0),
            )
        }

        HashTransformEffect::Earthquake => {
            let intensity = ((time * 0.5).sin() * 0.5 + 0.5).powf(2.0);
            let shake_x = (time * 15.0 + phase).sin() * intensity;
            let shake_y = (time * 17.0 + phase * 1.3).cos() * intensity;
            let shake_z = (time * 12.0 + phase * 0.7).sin() * intensity * 0.5;
            let offset = Vec3::new(shake_x, shake_y, shake_z) * strength * 8.0;
            let ax = (time * 12.0 + phase).sin() * intensity * strength * 0.1;
            let ay = (time * 14.0 + phase * 1.2).cos() * intensity * strength * 0.1;
            (
                offset,
                glam::Quat::from_euler(glam::EulerRot::XYZ, ax, ay, 0.0),
            )
        }

        HashTransformEffect::Ocean => {
            let rel = base_pos - center;
            let dist = (rel.x * rel.x + rel.y * rel.y).sqrt();
            let wave1 = (time * 0.3 + dist * 0.02 + phase * 0.3).sin();
            let wave2 = (time * 0.2 - dist * 0.015 + phase * 0.7).cos() * 0.5;
            let wave3 = (time * 0.4 + rel.x * 0.01 + rel.y * 0.008).sin() * 0.3;
            let offset = Vec3::new(0.0, 0.0, -(wave1 + wave2 + wave3) * strength * 15.0);
            let tilt = wave1 * strength * 0.05;
            (
                offset,
                glam::Quat::from_euler(glam::EulerRot::XYZ, tilt, tilt * 0.5, 0.0),
            )
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
            let rot = glam::Quat::from_euler(
                glam::EulerRot::XYZ,
                rot_angle.sin() * 0.3,
                rot_angle.cos() * 0.3,
                rot_angle * 0.2,
            );
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
    let hash_f = (hash as f32) / (u32::MAX as f32); // 0.0 to 1.0
    let tau = std::f32::consts::TAU;
    let phase = hash_f * tau; // unique phase per cube

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
        HashTransformEffect::Echo => Vec3::ZERO,
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
    pub slice_normal: [f32; 3], // Slice plane normal (normalized)
    pub _pad: [f32; 5],         // Pad to 256 bytes total
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
                direction: [-0.5, -0.7, -0.5], // Top-left front
                _pad: 0.0,
                color: [1.0, 0.98, 0.95], // Warm white
                intensity: 1.2,
            },
            fill: LightUniform {
                direction: [0.7, -0.3, 0.5], // Right side, softer
                _pad: 0.0,
                color: [0.7, 0.8, 1.0], // Cool blue fill
                intensity: 0.5,
            },
            rim: LightUniform {
                direction: [0.0, -0.2, 1.0], // Behind, edge light
                _pad: 0.0,
                color: [1.0, 1.0, 1.0],
                intensity: 0.3,
            },
            ambient: [0.15, 0.15, 0.18],
            _pad: 0.0,
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
            outline_color: [1.0, 0.5, 0.0, 1.0], // Orange
            tint_color: [1.0, 0.7, 0.2, 0.15],   // Warm tint
            viewport_size: [0.0, 0.0],
            _pad1: [0.0; 2],
        }
    }
}
