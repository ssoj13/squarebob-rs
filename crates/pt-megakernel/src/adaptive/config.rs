//! Adaptive sampling configuration.

/// Adaptive sampling configuration.
#[allow(dead_code)]
#[derive(Debug, Clone)]
pub struct AdaptiveConfig {
    /// Enable adaptive sampling
    pub enabled: bool,
    /// Minimum samples per pixel
    pub min_spp: u32,
    /// Maximum samples per pixel
    pub max_spp: u32,
    /// Variance threshold for assigning extra per-pixel sample budget.
    pub variance_threshold: f32,
    /// Update interval (frames between variance updates)
    pub update_interval: u32,
}

impl Default for AdaptiveConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            min_spp: 64,
            max_spp: 1024,
            variance_threshold: 0.001,
            update_interval: 4,
        }
    }
}
