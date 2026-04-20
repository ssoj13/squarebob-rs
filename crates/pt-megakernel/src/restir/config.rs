//! ReSTIR configuration.

/// ReSTIR configuration options.
#[allow(dead_code)]
#[derive(Debug, Clone)]
pub struct ReSTIRConfig {
    /// Enable ReSTIR for direct illumination
    pub di_enabled: bool,
    /// Enable ReSTIR for global illumination
    pub gi_enabled: bool,
    /// Enable temporal resampling
    pub temporal: bool,
    /// Enable spatial resampling
    pub spatial: bool,
    /// Number of initial candidates per pixel
    pub initial_candidates: u32,
    /// Number of spatial neighbors to sample
    pub spatial_neighbors: u32,
    /// Spatial sampling radius (pixels)
    pub spatial_radius: f32,
    /// Maximum history length (M_max) for temporal clamping
    pub m_max: u32,
    /// Use pairwise MIS for unbiased combination
    pub pairwise_mis: bool,
}

impl Default for ReSTIRConfig {
    fn default() -> Self {
        Self {
            di_enabled: false,
            gi_enabled: false,
            temporal: true,
            spatial: true,
            initial_candidates: 32,
            spatial_neighbors: 5,
            spatial_radius: 30.0,
            m_max: 30,
            pairwise_mis: true,
        }
    }
}
