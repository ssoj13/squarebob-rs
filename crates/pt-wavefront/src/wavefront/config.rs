//! Wavefront configuration.

/// Wavefront pass configuration.
#[allow(dead_code)]
#[derive(Debug, Clone, Default)]
pub struct WavefrontConfig {
    /// Enable wavefront PT (vs megakernel)
    pub enabled: bool,
    /// Tile size for wavefront rendering (0 = disabled)
    pub tile_size: u32,
}
