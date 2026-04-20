//! Material presets and materializer for PBR/PT rendering.

use std::path::Path;

use serde::{Deserialize, Serialize};

use pt_core::bvh::GpuMaterial;

// ============================================================================
// Material Source - what data determines the material
// ============================================================================

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
pub enum MaterialSource {
    #[default]
    None,       // All same material
    Extension,  // File extension hash
    Path,       // Full path hash
    Size,       // File size (normalized)
    Age,        // File age (normalized)
    Depth,      // Directory depth (normalized)
    Random,     // Random per-file
}

impl MaterialSource {
    pub fn name(self) -> &'static str {
        match self {
            MaterialSource::None => "None",
            MaterialSource::Extension => "Extension",
            MaterialSource::Path => "Path",
            MaterialSource::Size => "Size",
            MaterialSource::Age => "Age",
            MaterialSource::Depth => "Depth",
            MaterialSource::Random => "Random",
        }
    }

    pub fn all() -> &'static [MaterialSource] {
        &[
            MaterialSource::None,
            MaterialSource::Extension,
            MaterialSource::Path,
            MaterialSource::Size,
            MaterialSource::Age,
            MaterialSource::Depth,
            MaterialSource::Random,
        ]
    }
}

// ============================================================================
// Material Distribution - how values map to materials
// ============================================================================

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
pub enum MaterialDistribution {
    #[default]
    Direct,      // Direct mapping: value -> material index
    Quantized,   // Quantize to N discrete levels (uses quant_levels)
    Gradient,    // Smooth gradient across palette
    Spatial,     // Spatial coherence (Perlin-like noise)
    Bands,       // Alternating bands/stripes
}

impl MaterialDistribution {
    pub fn name(self) -> &'static str {
        match self {
            MaterialDistribution::Direct => "Direct",
            MaterialDistribution::Quantized => "Quantized",
            MaterialDistribution::Gradient => "Gradient",
            MaterialDistribution::Spatial => "Spatial",
            MaterialDistribution::Bands => "Bands",
        }
    }

    pub fn all() -> &'static [MaterialDistribution] {
        &[
            MaterialDistribution::Direct,
            MaterialDistribution::Quantized,
            MaterialDistribution::Gradient,
            MaterialDistribution::Spatial,
            MaterialDistribution::Bands,
        ]
    }
}

// Legacy enum for compatibility
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum MaterializeMode {
    None,
    ByExtension,
    ByPath,
    BySize,
    ByAge,
    Random,
}

impl MaterializeMode {
    pub fn name(self) -> &'static str {
        match self {
            MaterializeMode::None => "None",
            MaterializeMode::ByExtension => "By Extension",
            MaterializeMode::ByPath => "By Path",
            MaterializeMode::BySize => "By Size",
            MaterializeMode::ByAge => "By Age",
            MaterializeMode::Random => "Random",
        }
    }

    pub fn all() -> &'static [MaterializeMode] {
        &[
            MaterializeMode::None,
            MaterializeMode::ByExtension,
            MaterializeMode::ByPath,
            MaterializeMode::BySize,
            MaterializeMode::ByAge,
            MaterializeMode::Random,
        ]
    }
    
    /// Convert legacy mode to new source
    pub fn to_source(self) -> MaterialSource {
        match self {
            MaterializeMode::None => MaterialSource::None,
            MaterializeMode::ByExtension => MaterialSource::Extension,
            MaterializeMode::ByPath => MaterialSource::Path,
            MaterializeMode::BySize => MaterialSource::Size,
            MaterializeMode::ByAge => MaterialSource::Age,
            MaterializeMode::Random => MaterialSource::Random,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MaterialClass {
    Default = 0,
    Rubber,
    GlassClear,
    GlassBlue,
    GlassGreen,
    GlassAmber,
    GlassPink,
    Metal,
    Plastic,
    Water,
    Emissive,
    LightWarm2700,
    LightWarm3500,
    LightNeutral4500,
    LightCool6500,
    LightCool10000,
    Paint,
    Chalk,
    Ceramic,
    Concrete,
    Gold,
    Copper,
    Silver,
    Ruby,
    Jade,
    Diamond,
    Velvet,
    Wood,
    Marble,
    Neon,
    NeonPink,
    NeonPurple,
    NeonOrange,
    NeonBlue,
}

impl MaterialClass {
    /// Display color for PBR (RGB 0-255)
    pub fn color(self) -> [u8; 3] {
        match self {
            MaterialClass::Default => [200, 200, 200],
            MaterialClass::Rubber => [30, 30, 30],
            MaterialClass::GlassClear => [230, 240, 255],
            MaterialClass::GlassBlue => [160, 200, 255],
            MaterialClass::GlassGreen => [150, 220, 170],
            MaterialClass::GlassAmber => [240, 190, 110],
            MaterialClass::GlassPink => [240, 150, 190],
            MaterialClass::Metal => [210, 200, 180],
            MaterialClass::Plastic => [140, 160, 180],
            MaterialClass::Water => [100, 180, 230],
            MaterialClass::Emissive => [255, 180, 80],
            MaterialClass::LightWarm2700 => [255, 160, 110],
            MaterialClass::LightWarm3500 => [255, 185, 135],
            MaterialClass::LightNeutral4500 => [255, 220, 180],
            MaterialClass::LightCool6500 => [205, 225, 255],
            MaterialClass::LightCool10000 => [180, 210, 255],
            MaterialClass::Paint => [220, 100, 60],
            MaterialClass::Chalk => [245, 245, 245],
            MaterialClass::Ceramic => [200, 220, 240],
            MaterialClass::Concrete => [130, 130, 130],
            MaterialClass::Gold => [255, 215, 0],
            MaterialClass::Copper => [184, 115, 51],
            MaterialClass::Silver => [192, 192, 192],
            MaterialClass::Ruby => [224, 17, 95],
            MaterialClass::Jade => [0, 168, 107],
            MaterialClass::Diamond => [185, 242, 255],
            MaterialClass::Velvet => [75, 0, 130],
            MaterialClass::Wood => [139, 90, 43],
            MaterialClass::Marble => [240, 235, 230],
            MaterialClass::Neon => [57, 255, 20],
            MaterialClass::NeonPink => [255, 20, 147],
            MaterialClass::NeonPurple => [191, 0, 255],
            MaterialClass::NeonOrange => [255, 95, 31],
            MaterialClass::NeonBlue => [0, 191, 255],
        }
    }

    pub fn id(self) -> u32 {
        self as u32
    }

    pub fn is_emissive(self) -> bool {
        matches!(
            self,
            MaterialClass::Emissive
                | MaterialClass::LightWarm2700
                | MaterialClass::LightWarm3500
                | MaterialClass::LightNeutral4500
                | MaterialClass::LightCool6500
                | MaterialClass::LightCool10000
                | MaterialClass::Neon
                | MaterialClass::NeonPink
                | MaterialClass::NeonPurple
                | MaterialClass::NeonOrange
                | MaterialClass::NeonBlue
        )
    }

    pub fn is_transparent(self) -> bool {
        matches!(
            self,
            MaterialClass::GlassClear
                | MaterialClass::GlassBlue
                | MaterialClass::GlassGreen
                | MaterialClass::GlassAmber
                | MaterialClass::GlassPink
                | MaterialClass::Water
                | MaterialClass::Diamond
                | MaterialClass::Ruby
                | MaterialClass::Jade
        )
    }

    pub fn is_light(self) -> bool {
        matches!(
            self,
            MaterialClass::Emissive
                | MaterialClass::LightWarm2700
                | MaterialClass::LightWarm3500
                | MaterialClass::LightNeutral4500
                | MaterialClass::LightCool6500
                | MaterialClass::LightCool10000
                | MaterialClass::Neon
                | MaterialClass::NeonPink
                | MaterialClass::NeonPurple
                | MaterialClass::NeonOrange
                | MaterialClass::NeonBlue
        )
    }

    pub fn is_temperature_light(self) -> bool {
        matches!(
            self,
            MaterialClass::LightWarm2700
                | MaterialClass::LightWarm3500
                | MaterialClass::LightNeutral4500
                | MaterialClass::LightCool6500
                | MaterialClass::LightCool10000
        )
    }

    pub fn is_glass(self) -> bool {
        matches!(
            self,
            MaterialClass::GlassClear
                | MaterialClass::GlassBlue
                | MaterialClass::GlassGreen
                | MaterialClass::GlassAmber
                | MaterialClass::GlassPink
        )
    }
}

const BASE_CLASSES: &[MaterialClass] = &[
    MaterialClass::Default,
    MaterialClass::Rubber,
    MaterialClass::Metal,
    MaterialClass::Plastic,
    MaterialClass::Paint,
    MaterialClass::Chalk,
    MaterialClass::Ceramic,
    MaterialClass::Concrete,
    MaterialClass::Gold,
    MaterialClass::Copper,
    MaterialClass::Silver,
    MaterialClass::Velvet,
    MaterialClass::Wood,
    MaterialClass::Marble,
];

const GLASS_CLASSES: &[MaterialClass] = &[
    MaterialClass::GlassClear,
    MaterialClass::GlassBlue,
    MaterialClass::GlassGreen,
    MaterialClass::GlassAmber,
    MaterialClass::GlassPink,
];

const LIGHT_WARM: &[MaterialClass] = &[
    MaterialClass::LightWarm2700,
    MaterialClass::LightWarm3500,
];

const LIGHT_NEUTRAL: &[MaterialClass] = &[
    MaterialClass::LightNeutral4500,
];

const LIGHT_COOL: &[MaterialClass] = &[
    MaterialClass::LightCool6500,
    MaterialClass::LightCool10000,
];

pub struct MaterialLibrary {
    materials: Vec<GpuMaterial>,
}

impl Default for MaterialLibrary {
    fn default() -> Self {
        Self::new()
    }
}

impl MaterialLibrary {
    pub fn new() -> Self {
        let mut materials = Vec::with_capacity(MaterialClass::ALL.len());
        for class in MaterialClass::ALL {
            materials.push(material_for_class(class));
        }
        Self { materials }
    }

    pub fn materials(&self) -> &[GpuMaterial] {
        &self.materials
    }

    pub fn material_id(&self, class: MaterialClass) -> u32 {
        class.id().min(self.materials.len().saturating_sub(1) as u32)
    }
}

impl MaterialClass {
    pub const ALL: [MaterialClass; 34] = [
        MaterialClass::Default,
        MaterialClass::Rubber,
        MaterialClass::GlassClear,
        MaterialClass::GlassBlue,
        MaterialClass::GlassGreen,
        MaterialClass::GlassAmber,
        MaterialClass::GlassPink,
        MaterialClass::Metal,
        MaterialClass::Plastic,
        MaterialClass::Water,
        MaterialClass::Emissive,
        MaterialClass::LightWarm2700,
        MaterialClass::LightWarm3500,
        MaterialClass::LightNeutral4500,
        MaterialClass::LightCool6500,
        MaterialClass::LightCool10000,
        MaterialClass::Paint,
        MaterialClass::Chalk,
        MaterialClass::Ceramic,
        MaterialClass::Concrete,
        MaterialClass::Gold,
        MaterialClass::Copper,
        MaterialClass::Silver,
        MaterialClass::Ruby,
        MaterialClass::Jade,
        MaterialClass::Diamond,
        MaterialClass::Velvet,
        MaterialClass::Wood,
        MaterialClass::Marble,
        MaterialClass::Neon,
        MaterialClass::NeonPink,
        MaterialClass::NeonPurple,
        MaterialClass::NeonOrange,
        MaterialClass::NeonBlue,
    ];
}

#[derive(Debug, Clone, Copy)]
pub struct MaterializeSettings {
    pub allow_lights: bool,
    pub light_prob: f32,
    pub light_warm: f32,
    pub light_cool: f32,
    pub allow_glass: bool,
    pub glass_prob: f32,
    pub is_pt: bool,
    pub seed: u32,
    // New unified system
    pub source: MaterialSource,
    pub distribution: MaterialDistribution,
    pub quant_levels: u32,     // For Quantized distribution
    pub band_count: u32,       // For Bands distribution
    pub spatial_scale: f32,    // For Spatial distribution
}

impl Default for MaterializeSettings {
    fn default() -> Self {
        Self {
            allow_lights: false,
            light_prob: 0.15,
            light_warm: 0.5,
            light_cool: 0.5,
            allow_glass: false,
            glass_prob: 0.5,
            is_pt: false,
            seed: 2654435761,
            source: MaterialSource::None,
            distribution: MaterialDistribution::Direct,
            quant_levels: 5,
            band_count: 8,
            spatial_scale: 0.01,
        }
    }
}

/// Input data for material classification
#[derive(Debug, Clone, Copy)]
pub struct MaterialInput {
    pub name_hash: u32,
    pub path_hash: u32,
    pub size: u64,
    pub max_size: u64,
    pub depth: u32,
    pub max_depth: u32,
    pub age_normalized: f32,  // 0.0 = newest, 1.0 = oldest
    pub position: [f32; 3],   // Cube center position
}

impl Default for MaterialInput {
    fn default() -> Self {
        Self {
            name_hash: 0,
            path_hash: 0,
            size: 0,
            max_size: 1,
            depth: 0,
            max_depth: 1,
            age_normalized: 0.5,
            position: [0.0, 0.0, 0.0],
        }
    }
}

// ============================================================================
// New unified material classification
// ============================================================================

/// Classify material using the new unified system
pub fn classify_material(input: &MaterialInput, settings: &MaterializeSettings) -> MaterialClass {
    // Step 1: Get raw value from source (0.0 - 1.0)
    let raw_value = get_source_value(input, settings);
    
    // Step 2: Apply seed for variation
    let seeded = apply_seed(raw_value, input.name_hash, settings.seed);
    
    // Step 3: Apply distribution algorithm
    let distributed = apply_distribution(seeded, input, settings);
    
    // Step 4: Map to material class
    let class = value_to_material(distributed);
    
    // Step 5: Apply light/glass filters
    let final_hash = float_to_hash(seeded);
    apply_filters(class, final_hash, *settings)
}

/// Get normalized value (0.0-1.0) from the selected source
fn get_source_value(input: &MaterialInput, settings: &MaterializeSettings) -> f32 {
    match settings.source {
        MaterialSource::None => 0.5,  // Constant middle value
        MaterialSource::Extension => hash_to_float(input.name_hash),
        MaterialSource::Path => hash_to_float(input.path_hash),
        MaterialSource::Size => {
            if input.max_size == 0 { 0.5 }
            else {
                // Log scale for size
                let log_size = (input.size as f64 + 1.0).log10();
                let log_max = (input.max_size as f64 + 1.0).log10();
                (log_size / log_max.max(1.0)) as f32
            }
        }
        MaterialSource::Age => input.age_normalized,
        MaterialSource::Depth => {
            if input.max_depth == 0 { 0.0 }
            else { input.depth as f32 / input.max_depth as f32 }
        }
        MaterialSource::Random => hash_to_float(input.name_hash.wrapping_mul(0x9E3779B9)),
    }
}

/// Apply seed to create variation
fn apply_seed(value: f32, hash: u32, seed: u32) -> f32 {
    let seeded_hash = hash.wrapping_mul(seed);
    let noise = hash_to_float(seeded_hash) * 0.1;  // Small noise from seed
    (value + noise).fract()  // Keep in 0-1 range
}

/// Apply distribution algorithm
fn apply_distribution(value: f32, input: &MaterialInput, settings: &MaterializeSettings) -> f32 {
    match settings.distribution {
        MaterialDistribution::Direct => value,
        
        MaterialDistribution::Quantized => {
            let levels = settings.quant_levels.max(1) as f32;
            (value * levels).floor() / (levels - 1.0).max(1.0)
        }
        
        MaterialDistribution::Gradient => {
            // Smooth S-curve for gradient
            let t = value.clamp(0.0, 1.0);
            t * t * (3.0 - 2.0 * t)  // Smoothstep
        }
        
        MaterialDistribution::Spatial => {
            // Perlin-like noise based on position
            let scale = settings.spatial_scale;
            let px = input.position[0] * scale;
            let py = input.position[1] * scale;
            let pz = input.position[2] * scale;
            
            // Simple coherent noise (3D hash grid interpolation)
            let noise = spatial_noise(px, py, pz, settings.seed);
            (value * 0.3 + noise * 0.7).clamp(0.0, 1.0)
        }
        
        MaterialDistribution::Bands => {
            // Create alternating bands
            let bands = settings.band_count.max(1) as f32;
            let band_idx = (value * bands).floor();
            band_idx / (bands - 1.0).max(1.0)
        }
    }
}

/// Simple 3D noise function for spatial coherence
fn spatial_noise(x: f32, y: f32, z: f32, seed: u32) -> f32 {
    // Grid cell coordinates
    let ix = x.floor() as i32;
    let iy = y.floor() as i32;
    let iz = z.floor() as i32;
    
    // Fractional part for interpolation
    let fx = x - x.floor();
    let fy = y - y.floor();
    let fz = z - z.floor();
    
    // Smooth interpolation weights
    let ux = fx * fx * (3.0 - 2.0 * fx);
    let uy = fy * fy * (3.0 - 2.0 * fy);
    let uz = fz * fz * (3.0 - 2.0 * fz);
    
    // Hash corners of the cell
    let h000 = grid_hash(ix, iy, iz, seed);
    let h001 = grid_hash(ix, iy, iz + 1, seed);
    let h010 = grid_hash(ix, iy + 1, iz, seed);
    let h011 = grid_hash(ix, iy + 1, iz + 1, seed);
    let h100 = grid_hash(ix + 1, iy, iz, seed);
    let h101 = grid_hash(ix + 1, iy, iz + 1, seed);
    let h110 = grid_hash(ix + 1, iy + 1, iz, seed);
    let h111 = grid_hash(ix + 1, iy + 1, iz + 1, seed);
    
    // Trilinear interpolation
    let lerp = |a: f32, b: f32, t: f32| a + t * (b - a);
    
    let x00 = lerp(h000, h100, ux);
    let x01 = lerp(h001, h101, ux);
    let x10 = lerp(h010, h110, ux);
    let x11 = lerp(h011, h111, ux);
    
    let y0 = lerp(x00, x10, uy);
    let y1 = lerp(x01, x11, uy);
    
    lerp(y0, y1, uz)
}

/// Hash grid coordinates to float
fn grid_hash(x: i32, y: i32, z: i32, seed: u32) -> f32 {
    let h = (x as u32).wrapping_mul(73856093)
        ^ (y as u32).wrapping_mul(19349663)
        ^ (z as u32).wrapping_mul(83492791)
        ^ seed;
    hash_to_float(h)
}

/// Convert hash to float 0.0-1.0
fn hash_to_float(h: u32) -> f32 {
    (h as f32) / (u32::MAX as f32)
}

/// Convert float back to hash for filters
fn float_to_hash(f: f32) -> u32 {
    (f.clamp(0.0, 1.0) * u32::MAX as f32) as u32
}

/// Map normalized value to material class
fn value_to_material(value: f32) -> MaterialClass {
    let idx = ((value * BASE_CLASSES.len() as f32) as usize).min(BASE_CLASSES.len() - 1);
    BASE_CLASSES[idx]
}

// ============================================================================
// Legacy API (for compatibility)
// ============================================================================

/// Classify file into material based on mode with optional filtering (legacy API)
pub fn classify_path_filtered(
    path: &Path,
    size: u64,
    name_hash: u32,
    mode: MaterializeMode,
    settings: MaterializeSettings,
) -> MaterialClass {
    // Convert legacy mode to new system
    let mut new_settings = settings;
    new_settings.source = mode.to_source();
    // Keep distribution from settings (don't override)
    
    let input = MaterialInput {
        name_hash,
        path_hash: path_hash(path),
        size,
        max_size: size.max(1),  // Legacy: no max available
        depth: 0,
        max_depth: 1,
        age_normalized: hash_to_float(name_hash),  // Legacy: use hash as age proxy
        position: [0.0, 0.0, 0.0],
    };
    
    classify_material(&input, &new_settings)
}

fn apply_filters(class: MaterialClass, seeded_hash: u32, settings: MaterializeSettings) -> MaterialClass {
    // Normalize to base classes first (lights/glass are optional categories)
    let mut base = class;
    if base.is_light() || base.is_glass() {
        base = remap_to_base(seeded_hash);
    }

    // Optional light override (PT only)
    if settings.is_pt && settings.allow_lights
        && passes_prob(seeded_hash, 0x1234_5678, settings.light_prob) {
            return select_light_variant(seeded_hash, settings);
        }

    // Optional glass override (PT + PBR)
    if settings.allow_glass
        && passes_prob(seeded_hash, 0x8765_4321, settings.glass_prob) {
            return select_glass_variant(seeded_hash);
        }

    base
}

fn passes_prob(hash: u32, salt: u32, prob: f32) -> bool {
    let h = hash_derive(hash, salt);
    let p = (h as f32) / (u32::MAX as f32);
    p <= prob
}

fn remap_to_base(hash: u32) -> MaterialClass {
    let idx = (hash as usize) % BASE_CLASSES.len().max(1);
    BASE_CLASSES[idx]
}

fn select_glass_variant(hash: u32) -> MaterialClass {
    let idx = (hash_derive(hash, 0x5bd1_e995) as usize) % GLASS_CLASSES.len().max(1);
    GLASS_CLASSES[idx]
}

fn select_light_variant(hash: u32, settings: MaterializeSettings) -> MaterialClass {
    let warm = settings.light_warm.max(0.0);
    let cool = settings.light_cool.max(0.0);
    let neutral = 1.0;
    let total = (warm + cool + neutral).max(0.001);
    let r = (hash_derive(hash, 0x9e37_79b9) as f32) / (u32::MAX as f32);

    let warm_t = warm / total;
    let neutral_t = (warm + neutral) / total;

    if r < warm_t {
        let idx = (hash_derive(hash, 0xa2c2_a01d) as usize) % LIGHT_WARM.len().max(1);
        LIGHT_WARM[idx]
    } else if r < neutral_t {
        let idx = (hash_derive(hash, 0x1656_67b1) as usize) % LIGHT_NEUTRAL.len().max(1);
        LIGHT_NEUTRAL[idx]
    } else {
        let idx = (hash_derive(hash, 0x6d2b_79f5) as usize) % LIGHT_COOL.len().max(1);
        LIGHT_COOL[idx]
    }
}

#[allow(dead_code)]
fn classify_by_hash_with_palette(hash: u32, _settings: MaterializeSettings) -> MaterialClass {
    let idx = (hash as usize) % BASE_CLASSES.len().max(1);
    BASE_CLASSES[idx]
}

/// Classify by file extension
#[allow(dead_code)]
fn classify_by_extension(path: &Path) -> MaterialClass {
    let ext = path.extension().and_then(|e| e.to_str()).unwrap_or("").to_lowercase();
    let name = path.file_stem().and_then(|s| s.to_str()).unwrap_or("").to_lowercase();

    if name.contains("light") || name.contains("lamp") || name.contains("emit") {
        return MaterialClass::Metal;
    }
    if name.contains("neon") {
        return MaterialClass::Plastic;
    }
    if name.contains("glass") {
        return MaterialClass::Plastic;
    }
    if name.contains("water") {
        return MaterialClass::Plastic;
    }
    if name.contains("gold") {
        return MaterialClass::Gold;
    }
    if name.contains("diamond") || name.contains("gem") {
        return MaterialClass::Marble;
    }

    match ext.as_str() {
        "png" | "jpg" | "jpeg" | "gif" | "bmp" | "tga" => MaterialClass::Paint,
        "hdr" | "exr" => MaterialClass::Marble,
        "wav" | "mp3" | "flac" | "ogg" | "aac" | "m4a" => MaterialClass::Velvet,
        "mp4" | "mkv" | "mov" | "avi" | "webm" => MaterialClass::Silver,
        "zip" | "7z" | "rar" | "tar" | "gz" | "xz" | "bz2" => MaterialClass::Copper,
        "iso" | "img" | "dmg" => MaterialClass::Gold,
        "exe" | "dll" | "so" | "dylib" | "bin" => MaterialClass::Metal,
        "txt" | "md" | "rtf" => MaterialClass::Chalk,
        "doc" | "docx" | "pdf" => MaterialClass::Marble,
        "rs" | "go" | "c" | "cpp" | "h" | "hpp" => MaterialClass::Ceramic,
        "py" | "rb" => MaterialClass::Paint,
        "js" | "ts" | "java" | "cs" => MaterialClass::Ceramic,
        "html" | "css" | "scss" | "json" | "xml" | "yaml" | "toml" => MaterialClass::Plastic,
        "ttf" | "otf" | "woff" | "woff2" => MaterialClass::Marble,
        "obj" | "fbx" | "gltf" | "glb" | "stl" | "blend" => MaterialClass::Silver,
        "db" | "sqlite" | "sql" => MaterialClass::Concrete,
        "log" => MaterialClass::Wood,
        _ => MaterialClass::Default,
    }
}

/// Classify by file size ranges
#[allow(dead_code)]
fn classify_by_size(size: u64) -> MaterialClass {
    match size {
        0..=1024 => MaterialClass::Chalk,
        1025..=102400 => MaterialClass::Plastic,
        102401..=1048576 => MaterialClass::Paint,
        1048577..=10485760 => MaterialClass::Ceramic,
        10485761..=104857600 => MaterialClass::Metal,
        104857601..=1073741824 => MaterialClass::Silver,
        _ => MaterialClass::Gold,
    }
}

fn path_hash(path: &Path) -> u32 {
    let path_str = path.to_string_lossy();
    name_hash(&path_str)
}

/// Compute deterministic hash from a string
fn name_hash(name: &str) -> u32 {
    name.bytes().fold(0u32, |acc, b| acc.wrapping_mul(31).wrapping_add(b as u32))
}

#[inline]
fn hash_derive(hash: u32, salt: u32) -> u32 {
    hash.wrapping_mul(1664525).wrapping_add(salt).wrapping_mul(1013904223)
}

fn material_for_class(class: MaterialClass) -> GpuMaterial {
    match class {
        MaterialClass::Default => make_plastic([0.8, 0.8, 0.8], 0.3),
        MaterialClass::Rubber => make_plastic([0.05, 0.05, 0.05], 0.9),
        MaterialClass::GlassClear => make_glass([0.95, 0.98, 1.0], 1.52, 0.02),
        MaterialClass::GlassBlue => make_glass([0.7, 0.85, 1.0], 1.52, 0.02),
        MaterialClass::GlassGreen => make_glass([0.6, 0.95, 0.7], 1.52, 0.02),
        MaterialClass::GlassAmber => make_glass([1.0, 0.7, 0.4], 1.52, 0.03),
        MaterialClass::GlassPink => make_glass([1.0, 0.7, 0.85], 1.52, 0.03),
        MaterialClass::Metal => make_metal([0.9, 0.85, 0.75], 0.2),
        MaterialClass::Plastic => make_plastic([0.6, 0.65, 0.7], 0.35),
        MaterialClass::Water => make_glass([0.6, 0.8, 1.0], 1.33, 0.01),
        MaterialClass::Emissive => make_emissive([1.0, 0.6, 0.2], 8.0),
        MaterialClass::LightWarm2700 => make_emissive([1.0, 0.62, 0.3], 14.0),
        MaterialClass::LightWarm3500 => make_emissive([1.0, 0.72, 0.45], 16.0),
        MaterialClass::LightNeutral4500 => make_emissive([1.0, 0.85, 0.65], 18.0),
        MaterialClass::LightCool6500 => make_emissive([0.78, 0.88, 1.0], 18.0),
        MaterialClass::LightCool10000 => make_emissive([0.65, 0.8, 1.0], 20.0),
        MaterialClass::Paint => make_paint([0.85, 0.4, 0.2], 0.3, 0.1),
        MaterialClass::Chalk => make_plastic([0.95, 0.95, 0.95], 0.95),
        MaterialClass::Ceramic => make_paint([0.85, 0.9, 0.95], 0.2, 0.2),
        MaterialClass::Concrete => make_plastic([0.5, 0.5, 0.5], 0.75),
        MaterialClass::Gold => make_metal([1.0, 0.84, 0.0], 0.1),
        MaterialClass::Copper => make_metal([0.72, 0.45, 0.2], 0.15),
        MaterialClass::Silver => make_metal([0.95, 0.93, 0.88], 0.05),
        MaterialClass::Ruby => make_gem([0.88, 0.07, 0.37], 1.77),
        MaterialClass::Jade => make_gem([0.0, 0.66, 0.42], 1.61),
        MaterialClass::Diamond => make_glass([0.97, 0.97, 1.0], 2.42, 0.0),
        MaterialClass::Velvet => make_velvet([0.29, 0.0, 0.51]),
        MaterialClass::Wood => make_plastic([0.55, 0.35, 0.17], 0.6),
        MaterialClass::Marble => make_marble([0.94, 0.92, 0.90]),
        MaterialClass::Neon => make_emissive([0.22, 1.0, 0.08], 15.0),
        MaterialClass::NeonPink => make_emissive([1.0, 0.08, 0.58], 15.0),
        MaterialClass::NeonPurple => make_emissive([0.75, 0.0, 1.0], 15.0),
        MaterialClass::NeonOrange => make_emissive([1.0, 0.37, 0.12], 15.0),
        MaterialClass::NeonBlue => make_emissive([0.0, 0.75, 1.0], 15.0),
    }
}

fn make_plastic(color: [f32; 3], roughness: f32) -> GpuMaterial {
    let mut m = default_material();
    m.base_color_weight = [color[0], color[1], color[2], 1.0];
    m.params1[2] = roughness.max(0.04);
    m
}

fn make_metal(color: [f32; 3], roughness: f32) -> GpuMaterial {
    let mut m = default_material();
    m.base_color_weight = [color[0], color[1], color[2], 1.0];
    m.params1[1] = 1.0;
    m.params1[2] = roughness.max(0.02);
    m
}

fn make_glass(color: [f32; 3], ior: f32, roughness: f32) -> GpuMaterial {
    let mut m = default_material();
    m.base_color_weight = [color[0], color[1], color[2], 0.0];
    m.transmission_color_weight = [color[0], color[1], color[2], 1.0];
    m.params1[2] = roughness.max(0.0);
    m.params1[3] = ior;
    m
}

fn make_emissive(color: [f32; 3], intensity: f32) -> GpuMaterial {
    let mut m = default_material();
    m.base_color_weight = [0.0, 0.0, 0.0, 0.0];
    m.specular_color_weight[3] = 0.0;
    m.emission_color_weight = [color[0], color[1], color[2], intensity];
    m
}

fn make_paint(color: [f32; 3], roughness: f32, coat_weight: f32) -> GpuMaterial {
    let mut m = make_plastic(color, roughness);
    m.coat_color_weight = [1.0, 1.0, 1.0, coat_weight];
    m.params2[1] = 0.15;
    m
}

fn make_gem(color: [f32; 3], ior: f32) -> GpuMaterial {
    let mut m = default_material();
    m.base_color_weight = [color[0] * 0.3, color[1] * 0.3, color[2] * 0.3, 0.5];
    m.transmission_color_weight = [color[0], color[1], color[2], 0.8];
    m.specular_color_weight = [1.0, 1.0, 1.0, 1.0];
    m.params1[2] = 0.02;
    m.params1[3] = ior;
    m
}

fn make_velvet(color: [f32; 3]) -> GpuMaterial {
    let mut m = default_material();
    m.base_color_weight = [color[0], color[1], color[2], 1.0];
    m.params1[2] = 0.8;
    m.coat_color_weight = [1.0, 1.0, 1.0, 0.1];
    m.params2[1] = 0.9;
    m
}

fn make_marble(color: [f32; 3]) -> GpuMaterial {
    let mut m = default_material();
    m.base_color_weight = [color[0], color[1], color[2], 1.0];
    m.params1[2] = 0.15;
    m.subsurface_color_weight = [color[0] * 0.9, color[1] * 0.85, color[2] * 0.8, 0.1];
    m
}

fn default_material() -> GpuMaterial {
    GpuMaterial {
        base_color_weight: [0.8, 0.8, 0.8, 1.0],
        specular_color_weight: [1.0, 1.0, 1.0, 1.0],
        transmission_color_weight: [1.0, 1.0, 1.0, 0.0],
        subsurface_color_weight: [1.0, 1.0, 1.0, 0.0],
        coat_color_weight: [1.0, 1.0, 1.0, 0.0],
        emission_color_weight: [1.0, 1.0, 1.0, 0.0],
        opacity: [1.0, 1.0, 1.0, 1.0],
        params1: [0.0, 0.0, 0.2, 1.5],
        params2: [0.0, 0.1, 1.5, 1.0],
    }
}
