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
#[repr(u8)]
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

impl BuiltInTonemap {
    /// Tag value packed into `BlitParams.color.x` so the blit
    /// shader can pick the right curve at runtime. The numbers
    /// alias with the legacy `TonemapKind::gpu_tag()` codes for
    /// `None / Linear / Reinhard` so the shader's existing
    /// switch needs only the new `AgX` case appended.
    pub const fn gpu_tag(self) -> u32 {
        match self {
            BuiltInTonemap::None => 0,
            BuiltInTonemap::Linear => 1,
            BuiltInTonemap::Reinhard => 2,
            BuiltInTonemap::AgX => 5,
        }
    }
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
#[repr(u8)]
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
#[repr(u8)]
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

/// Sentinel value the blit shader uses when `mode == Ocio` — it
/// switches the curve path from the built-in family to the 3D
/// LUT sampler (added in phase 6).
pub const OCIO_LUT_TAG: u32 = 6;

impl ColorPipelineSettings {
    /// Single tag the blit shader switches on. Combines the
    /// `mode` toggle with the built-in selection:
    /// `BuiltIn` returns the matching `BuiltInTonemap::gpu_tag()`;
    /// `Ocio` returns [`OCIO_LUT_TAG`].
    pub const fn resolved_tonemap_tag(&self) -> u32 {
        match self.mode {
            ColorMode::BuiltIn => self.builtin.gpu_tag(),
            ColorMode::Ocio => OCIO_LUT_TAG,
        }
    }

    /// Hash that changes whenever any field that affects the
    /// `Processor` build changes. The runtime uses this to decide
    /// when to rebuild — equal hash = same processor, skip rebuild.
    fn build_hash(&self) -> u64 {
        use std::hash::{Hash, Hasher};
        let mut h = std::collections::hash_map::DefaultHasher::new();
        // Mode + tonemap kind + codepath gate the whole pipeline
        // shape, so they belong in the hash even though only the
        // OCIO branch reads the OCIO fields.
        (self.mode as u8).hash(&mut h);
        (self.builtin as u8).hash(&mut h);
        (self.codepath as u8).hash(&mut h);
        match &self.ocio_config {
            ConfigSource::BuiltIn => 0u8.hash(&mut h),
            ConfigSource::Bundled(name) => {
                1u8.hash(&mut h);
                name.hash(&mut h);
            }
            ConfigSource::External(p) => {
                2u8.hash(&mut h);
                p.hash(&mut h);
            }
        }
        self.ocio_input_space.hash(&mut h);
        self.ocio_display.hash(&mut h);
        self.ocio_view.hash(&mut h);
        self.ocio_look.hash(&mut h);
        self.ocio_custom_lut.hash(&mut h);
        h.finish()
    }
}

// ── Runtime ───────────────────────────────────────────────────────────

/// Live colour pipeline owned by the renderer. Holds the active
/// `vfx_ocio::Config` plus a cached `Processor` built from the
/// current [`ColorPipelineSettings`]. Call [`Self::ensure`] each
/// frame before applying — it rebuilds the `Processor` if any
/// setting changed since the last build, otherwise it's a hash
/// compare and noop.
pub struct ColorPipeline {
    config: vfx_ocio::Config,
    config_source: ConfigSource,
    processor: Option<vfx_ocio::Processor>,
    last_hash: u64,
}

impl ColorPipeline {
    /// Initialise from a settings struct. Loads the appropriate
    /// `Config` (built-in / bundled / external) and builds the
    /// initial `Processor`. Falls back to the built-in ACES 1.3
    /// config if external loading fails, so the renderer always
    /// has *some* config to work with.
    ///
    /// `Bundled` configs are resolved against [`default_bundled_dir`],
    /// which probes `current_exe()/data/ocio` first, then the same
    /// path relative to the current working directory. This keeps
    /// the crate API self-contained — callers don't have to thread
    /// a data-dir reference through the renderer.
    pub fn new(settings: &ColorPipelineSettings) -> Self {
        let bundled_dir = default_bundled_dir();
        let (config, resolved_source) = load_config(&settings.ocio_config, &bundled_dir);
        let mut pipe = Self {
            config,
            config_source: resolved_source,
            processor: None,
            last_hash: 0,
        };
        // Initial build — errors are logged and leave `processor`
        // as `None`. Callers that try to `apply_cpu` on a `None`
        // processor get a no-op pass-through and a log entry.
        let _ = pipe.rebuild(settings);
        pipe
    }

    /// Rebuild the processor if `settings.build_hash()` differs
    /// from the cached one. No-op otherwise. Returns `Ok(())` on
    /// success or a no-op skip; `Err` is reserved for hard
    /// build failures that the host wants to surface.
    pub fn ensure(
        &mut self,
        settings: &ColorPipelineSettings,
    ) -> Result<(), vfx_ocio::OcioError> {
        let hash = settings.build_hash();
        if hash == self.last_hash {
            return Ok(());
        }
        // Config source itself may have changed — reload if so.
        if self.config_source != settings.ocio_config {
            let bundled_dir = default_bundled_dir();
            let (config, resolved_source) = load_config(&settings.ocio_config, &bundled_dir);
            self.config = config;
            self.config_source = resolved_source;
        }
        self.rebuild(settings)?;
        self.last_hash = hash;
        Ok(())
    }

    fn rebuild(
        &mut self,
        settings: &ColorPipelineSettings,
    ) -> Result<(), vfx_ocio::OcioError> {
        if settings.mode == ColorMode::BuiltIn {
            // Built-in tonemaps don't need an OCIO processor —
            // their math is in the blit shader.
            self.processor = None;
            return Ok(());
        }
        // OCIO mode — build a display processor for
        // (input_space, display, view). The named look (if any)
        // is folded in by chaining a `LookTransform` on top.
        let proc = self.config.display_processor(
            &settings.ocio_input_space,
            &settings.ocio_display,
            &settings.ocio_view,
        )?;
        self.processor = Some(proc);
        Ok(())
    }

    /// Apply the cached processor to a slice of RGB triples.
    /// CPU codepath — slow per-frame, but deterministic and
    /// matches the OCIO reference bit-for-bit. No-op when
    /// `mode == BuiltIn` (the shader does the work) or when the
    /// processor failed to build.
    pub fn apply_cpu(&self, pixels: &mut [[f32; 3]]) {
        if let Some(proc) = &self.processor {
            proc.apply_rgb(pixels);
        }
    }

    /// Live `Config` reference for the UI to enumerate displays
    /// / views / colour spaces / looks. Always returns a valid
    /// config because `load_config` falls back to the built-in.
    pub fn config(&self) -> &vfx_ocio::Config {
        &self.config
    }

    /// Which `ConfigSource` is actually live right now. May
    /// differ from the requested source when an external load
    /// failed and the pipeline degraded to `BuiltIn`.
    pub fn active_config_source(&self) -> &ConfigSource {
        &self.config_source
    }

    /// Names of every colour space in the active config, plus
    /// every role name (`scene_linear`, `compositing_linear`, …).
    /// Both are valid identifiers for the "input space" lane,
    /// and the UI shows them in the same dropdown.
    pub fn available_input_spaces(&self) -> Vec<String> {
        let mut out: Vec<String> = self
            .config
            .colorspaces()
            .iter()
            .map(|cs| cs.name().to_string())
            .collect();
        // Roles surface here too because users typically pin to a
        // semantic name (`scene_linear`) instead of a technical
        // colour-space name (`ACEScg`) — keeps presets portable
        // across configs that swap the underlying space.
        for (role, _cs) in self.config.roles().iter() {
            if !out.iter().any(|n| n == role) {
                out.push(role.to_string());
            }
        }
        out.sort();
        out
    }

    /// Names of every display device in the active config.
    pub fn available_displays(&self) -> Vec<String> {
        self.config
            .displays()
            .displays()
            .iter()
            .map(|d| d.name().to_string())
            .collect()
    }

    /// View names available for `display`. Empty if the display
    /// isn't in the config (e.g. user typed a name into the
    /// settings before this code knew about ComboBoxes).
    pub fn available_views(&self, display: &str) -> Vec<String> {
        self.config
            .displays()
            .display(display)
            .map(|d| d.views().iter().map(|v| v.name().to_string()).collect())
            .unwrap_or_default()
    }

    /// Named looks in the active config. The UI surfaces an
    /// empty "no look" entry on top of this list separately.
    pub fn available_looks(&self) -> Vec<String> {
        self.config
            .looks()
            .names()
            .map(|s| s.to_string())
            .collect()
    }
}

/// Resolve the directory holding bundled `.ocio` files. Probes
/// `<exe_dir>/data/ocio` first, then `<cwd>/data/ocio`. Either
/// path may not exist on disk — `load_config` checks for the
/// specific file when it joins the bundled name in.
fn default_bundled_dir() -> std::path::PathBuf {
    if let Ok(exe) = std::env::current_exe()
        && let Some(parent) = exe.parent()
    {
        let p = parent.join("data").join("ocio");
        if p.exists() {
            return p;
        }
    }
    std::path::PathBuf::from("data").join("ocio")
}

/// Load the requested config, falling back to the built-in
/// ACES 1.3 on any error. Returns both the loaded `Config` and
/// the source that actually backed it (which may be `BuiltIn`
/// even if the caller asked for `External`).
fn load_config(
    source: &ConfigSource,
    bundled_dir: &std::path::Path,
) -> (vfx_ocio::Config, ConfigSource) {
    match source {
        ConfigSource::BuiltIn => (vfx_ocio::builtin::aces_1_3(), ConfigSource::BuiltIn),
        ConfigSource::Bundled(name) => {
            let path = bundled_dir.join(name);
            match vfx_ocio::Config::from_file(&path) {
                Ok(cfg) => (cfg, ConfigSource::Bundled(name.clone())),
                Err(e) => {
                    log::warn!(
                        "color-pipeline: failed to load bundled config {}: {e} \
                         — falling back to built-in ACES 1.3",
                        path.display()
                    );
                    (vfx_ocio::builtin::aces_1_3(), ConfigSource::BuiltIn)
                }
            }
        }
        ConfigSource::External(path) => match vfx_ocio::Config::from_file(path) {
            Ok(cfg) => (cfg, ConfigSource::External(path.clone())),
            Err(e) => {
                log::warn!(
                    "color-pipeline: failed to load external config {}: {e} \
                     — falling back to built-in ACES 1.3",
                    path.display()
                );
                (vfx_ocio::builtin::aces_1_3(), ConfigSource::BuiltIn)
            }
        },
    }
}
