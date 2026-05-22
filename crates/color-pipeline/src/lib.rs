//! Display-side colour pipeline.
//!
//! Two coexisting modes the host app toggles between at runtime:
//!
//! - **Built-in**: a tiny shader-side catalogue — `None` (clamp),
//!   `Linear` (no curve, debug), `Reinhard`, and `AgX`. No external
//!   data files, no OCIO config. Used for quick scrubs, debugging,
//!   and when the user just wants a fast filmic default.
//! - **Ocio**: the full [`vfx-ocio`] pipeline — load a config
//!   (built-in `aces_1_3()`, a bundled `.ocio` shipped under
//!   `data/ocio/`, or a user-provided file) and route every frame
//!   through a `Processor` built from `(input_space, display,
//!   view)` plus an optional `Look` or external LUT file.
//!
//! ## Codepath
//!
//! Both modes support a CPU **and** a GPU codepath. The host
//! picks one via [`ColorCodepath`]; the dispatcher in this crate
//! exposes the same `process_pixels` / `gpu_shader_for_blit`
//! entry points for either choice. Persistence is part of the
//! user's preset because the codepath is a real performance
//! lever (CPU is hot-readback heavy, GPU adds a translated
//! WGSL stub on top of the blit).
//!
//! ## Why a separate crate
//!
//! The old `Render3DOptions.color_*` cascade lived in
//! `render-shared` and dragged matrix math + filmic curves +
//! gamut-compress helpers into the same module that holds every
//! other piece of renderer config. Splitting the colour
//! pipeline out keeps the heavyweight `vfx-ocio` dep off the
//! shared options crate (smaller compile graph for everything
//! else) and gives the colour code a place to grow without the
//! pressure to inline into `render-shared`'s 2k-line file.

#![warn(missing_docs)]

use std::path::PathBuf;

use serde::{Deserialize, Serialize};

// ── Built-in tonemaps ──────────────────────────────────────────────────

/// Built-in tonemap kinds — purely shader-side, no external data.
/// Distinct from the OCIO Display/View path: this is the "I don't
/// want a colour management system, just give me a curve" lane.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, Default)]
pub enum BuiltInTonemap {
    /// Clamp `[0, +∞)` to `[0, 1]`. Debug — highlights become flat
    /// white blobs, only useful for sanity checks.
    None,
    /// No curve at all. Same as `None` but skips the saturate;
    /// useful for sending HDR straight into a float framebuffer.
    Linear,
    /// `c / (1 + c)`, per-channel. Cheap, soft rolloff, but a bit
    /// washed out. Reliable baseline.
    Reinhard,
    /// AgX (Eary Chow). Modern open-source filmic transform that
    /// preserves hue better than ACES at the cost of some
    /// saturation. Shader-side polynomial fit; for the LUT-based
    /// variants (Punchy / Golden / Neutral) use `Ocio` mode with
    /// a `LookTransform`.
    #[default]
    AgX,
}

// ── OCIO config source ─────────────────────────────────────────────────

/// Where to source the active OCIO `Config` from.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize, Default)]
pub enum ConfigSource {
    /// `vfx_ocio::builtin::aces_1_3()` — zero-file fallback, ACES
    /// 1.3 reference shipping inside the binary.
    #[default]
    BuiltIn,
    /// A `.ocio` (or `.ocioz` / `.json`) bundled under `data/ocio/`
    /// in the app distribution. The string is the file's path
    /// relative to that directory (e.g. `"studio-config-v2.ocio"`).
    Bundled(String),
    /// A user-loaded config from anywhere on disk. Supports plain
    /// `.ocio`, OCIO archives `.ocioz`, and OCIO 2.x `.json`.
    External(PathBuf),
}

// ── Codepath ───────────────────────────────────────────────────────────

/// CPU vs GPU dispatch. Persisted in the preset because flipping
/// between them changes frame-time characteristics significantly.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, Default)]
pub enum ColorCodepath {
    /// `Processor::apply_rgb` on the readback buffer each frame.
    /// Slow (`O(W*H)` per frame plus a GPU→CPU readback), but
    /// numerically identical to the OCIO reference and trivially
    /// correct for any transform vfx-ocio can build.
    Cpu,
    /// `GpuProcessor::extract_gpu_shader_info` → GLSL → naga →
    /// WGSL, patched into the blit shader. Fast (no readback),
    /// but the GLSL→WGSL translation has a non-zero risk surface;
    /// some exotic transforms fall back to CPU automatically.
    #[default]
    Gpu,
}

// ── Top-level mode toggle ──────────────────────────────────────────────

/// Which family of transforms the colour stage runs.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, Default)]
pub enum ColorMode {
    /// Use the [`BuiltInTonemap`] selection below.
    BuiltIn,
    /// Use the OCIO pipeline (config + input space + display + view + look).
    #[default]
    Ocio,
}

// ── Settings ───────────────────────────────────────────────────────────

/// Full colour-pipeline settings owned by `Render3DOptions`.
///
/// In `BuiltIn` mode only [`Self::builtin`] is consulted. In `Ocio`
/// mode every `ocio_*` field is live and the built-in field is
/// ignored. The split keeps the two modes orthogonal so flipping
/// the mode toggle doesn't silently disturb the other branch's
/// settings.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ColorPipelineSettings {
    /// Which mode the pipeline runs in.
    pub mode: ColorMode,
    /// Active built-in tonemap when `mode == BuiltIn`.
    pub builtin: BuiltInTonemap,
    /// Active codepath. Honoured by both modes.
    pub codepath: ColorCodepath,

    // ── OCIO-mode fields ──
    /// Which OCIO config to use.
    pub ocio_config: ConfigSource,
    /// Input colour-space name from the config (e.g.
    /// `"ACEScg"` or the `"scene_linear"` role). Path-traced
    /// scene-linear output is fed into this space.
    pub ocio_input_space: String,
    /// Display name from the config (e.g. `"sRGB"` / `"Rec.709"` /
    /// `"Rec.2020"`).
    pub ocio_display: String,
    /// View name from the config, restricted to the views
    /// available for the selected `ocio_display`.
    pub ocio_view: String,
    /// Optional named look from the config (`None` = no look).
    pub ocio_look: Option<String>,
    /// Optional user-loaded LUT (`.cube` / `.3dl` / `.spi1d` /
    /// `.spi3d` / `.csp`). Applied AFTER the display/view chain
    /// — i.e. the LUT operates in display space, the same place
    /// a vendor "creative LUT" usually sits.
    pub ocio_custom_lut: Option<PathBuf>,
}

impl Default for ColorPipelineSettings {
    fn default() -> Self {
        Self {
            mode: ColorMode::default(),
            builtin: BuiltInTonemap::default(),
            codepath: ColorCodepath::default(),
            ocio_config: ConfigSource::default(),
            // Sensible defaults for the built-in ACES 1.3 config:
            // input = working space role, output = sRGB display +
            // ACES 1.0 SDR-video view. The default look slot is
            // empty so the rendered image is identical to the
            // reference ACES output until the user dials a look in.
            ocio_input_space: "scene_linear".to_string(),
            ocio_display: "sRGB".to_string(),
            ocio_view: "ACES 1.0 SDR-video".to_string(),
            ocio_look: None,
            ocio_custom_lut: None,
        }
    }
}
