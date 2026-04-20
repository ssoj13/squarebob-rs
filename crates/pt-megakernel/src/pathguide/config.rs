//! Path guiding configuration.

/// Path guiding configuration options.
#[allow(dead_code)]
#[derive(Debug, Clone)]
pub struct PathGuideConfig {
    /// Enable path guiding
    pub enabled: bool,
    /// SVO resolution (power of 2)
    pub svo_resolution: u32,
    /// Use product sampling (BSDF * guiding)
    pub product_sampling: bool,
    /// Guiding weight (0=BSDF only, 1=guided only)
    pub guide_weight: f32,
    /// Training iterations before using guide
    pub warmup_frames: u32,
}

impl Default for PathGuideConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            svo_resolution: 64,
            product_sampling: true,
            guide_weight: 0.5,
            warmup_frames: 8,
        }
    }
}
