//! Display-side colour pipeline.
//!
//! Two coexisting modes the host app toggles between at runtime:
//!
//! - **Built-in**: a tiny shader-side catalogue — `None` (clamp),
//!   `Linear` (no curve, debug), `Reinhard`, and `AgX`. No external
//!   data files, no OCIO config. Used for quick scrubs, debugging,
//!   and when the user just wants a fast filmic default.
//! - **Ocio**: the full [`vfx-ocio`] pipeline — load a config
//!   (built-in `default_config()`, a bundled `.ocio` shipped under
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

/// Re-export `vfx_ocio` so downstream callers can drive the embedded
/// builtin registry (`vfx_ocio::builtin::embedded`) and other OCIO
/// types through this crate without taking a direct `vfx-ocio`
/// dependency.
pub use vfx_ocio;

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
    /// `vfx_ocio::builtin::default_config()` — the latest embedded
    /// release shipped with the vfx-ocio crate (currently ACES 2.0
    /// Studio All-Views v4.0.0).
    #[default]
    BuiltIn,
    /// One of the embedded release configs (`vfx_ocio::builtin::embedded`).
    /// The string is the canonical name from
    /// `vfx_ocio::builtin::embedded::REGISTRY` — e.g.
    /// `"studio-config-v4.0.0_aces-v2.0_ocio-v2.5"`.
    Embedded(String),
    /// Legacy variant kept for back-compat with old presets that
    /// referenced a file under `data/ocio/`. New UIs route through
    /// `External` instead.
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

/// Snapshot of the per-OCIO-source settings (input space / display
/// / view / look / custom LUT). Stored separately for each
/// [`ConfigSource`] so flipping `BuiltIn ↔ External` keeps each
/// side's last-used selections instead of forcing the user to
/// re-pick every dropdown.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct PerSourceOcio {
    /// Input colour-space name from the config (e.g.
    /// `"ACEScg"` or the `"scene_linear"` role). Path-traced
    /// scene-linear output is fed into this space.
    #[serde(default)]
    pub input_space: String,
    /// Display name from the config (e.g. `"sRGB"` / `"Rec.709"`).
    #[serde(default)]
    pub display: String,
    /// View name from the config, restricted to the views
    /// available for the selected `display`.
    #[serde(default)]
    pub view: String,
    /// Optional named look from the config (`None` = no look).
    #[serde(default)]
    pub look: Option<String>,
    /// Optional user-loaded LUT (`.cube / .3dl / .spi1d / .spi3d /
    /// .csp`). Applied AFTER the display/view chain.
    #[serde(default)]
    pub custom_lut: Option<PathBuf>,
}

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
    /// Which OCIO config the pipeline is currently consulting.
    pub ocio_config: ConfigSource,
    /// Live input colour-space name from the *active* config.
    /// Source-specific backups live in [`Self::builtin_ocio`] /
    /// [`Self::external_ocio`]; the renderer's `ColorPipeline`
    /// reads from these flat fields.
    pub ocio_input_space: String,
    /// Live display name. See `ocio_input_space`.
    pub ocio_display: String,
    /// Live view name. See `ocio_input_space`.
    pub ocio_view: String,
    /// Live optional look. See `ocio_input_space`.
    pub ocio_look: Option<String>,
    /// Live optional custom LUT. See `ocio_input_space`.
    pub ocio_custom_lut: Option<PathBuf>,

    // ── Per-source persistence ──
    /// Saved settings for the `BuiltIn` source. The UI swaps these
    /// into the flat `ocio_*` fields when the user re-selects
    /// `BuiltIn` so the previous selections come back unchanged.
    #[serde(default)]
    pub builtin_ocio: PerSourceOcio,
    /// Saved settings for the `External` source. See `builtin_ocio`.
    #[serde(default)]
    pub external_ocio: PerSourceOcio,
}

impl ColorPipelineSettings {
    /// Capture the live `ocio_*` flat fields into a snapshot.
    /// Used by the UI to back up the current source's selections
    /// before swapping to another source.
    pub fn snapshot_active_ocio(&self) -> PerSourceOcio {
        PerSourceOcio {
            input_space: self.ocio_input_space.clone(),
            display: self.ocio_display.clone(),
            view: self.ocio_view.clone(),
            look: self.ocio_look.clone(),
            custom_lut: self.ocio_custom_lut.clone(),
        }
    }

    /// Apply a snapshot back into the live `ocio_*` flat fields.
    pub fn restore_ocio(&mut self, snap: &PerSourceOcio) {
        self.ocio_input_space = snap.input_space.clone();
        self.ocio_display = snap.display.clone();
        self.ocio_view = snap.view.clone();
        self.ocio_look = snap.look.clone();
        self.ocio_custom_lut = snap.custom_lut.clone();
    }
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
            builtin_ocio: PerSourceOcio::default(),
            external_ocio: PerSourceOcio::default(),
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
    /// `Ocio + Gpu` returns [`OCIO_LUT_TAG`] (shader trilinear-samples
    /// the baked 3D LUT);
    /// `Ocio + Cpu` returns `0` — the host has already replaced the PT
    /// output texture with display-encoded pixels via
    /// `apply_cpu_color_in_place`, so the blit just needs to clamp-
    /// passthrough.
    pub const fn resolved_tonemap_tag(&self) -> u32 {
        match self.mode {
            ColorMode::BuiltIn => self.builtin.gpu_tag(),
            ColorMode::Ocio => match self.codepath {
                ColorCodepath::Gpu => OCIO_LUT_TAG,
                ColorCodepath::Cpu => 0,
            },
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
            ConfigSource::Embedded(name) => {
                3u8.hash(&mut h);
                name.hash(&mut h);
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
    /// Cached 33×33×33 RGB baked LUT. Rebuilt alongside the
    /// processor on a `last_hash` change. Lives here (not in the
    /// renderer) so multiple consumers (CPU codepath + GPU
    /// codepath upload + future LUT export) can read the same
    /// baked data without re-baking.
    lut_3d: Option<BakedLut3D>,
    last_hash: u64,
    /// Set to `true` whenever [`Self::rebuild`] produces a fresh
    /// baked LUT (including the initial bake in [`Self::new`]).
    /// The renderer host polls it via [`Self::take_pending_lut`]
    /// once per frame and uploads when it observes a pending flag.
    /// Keeps the upload signal decoupled from the `ensure()` return
    /// value, so callers that just want to refresh dropdown lists
    /// don't need to thread the renderer through.
    lut_upload_pending: bool,
    /// Outcome of the most recent `ocio_custom_lut` load attempt.
    /// Surfaced to the UI so the user can see whether the file
    /// actually loaded or fell through silently.
    custom_lut_status: CustomLutStatus,
    /// Whether OCIO output contains code values that must be decoded into
    /// eframe's linear composition transport before final output encoding.
    decode_code_values_for_transport: bool,
    /// Hard failure from the most recent settings rebuild. Kept until a
    /// different settings hash rebuilds successfully.
    last_error: Option<String>,
}

/// Status of the optional `Custom LUT` slot, observable by the host
/// UI. Updated on every [`ColorPipeline::rebuild`] in `Ocio` mode.
#[derive(Debug, Clone, Default)]
pub enum CustomLutStatus {
    /// No custom LUT requested (`ocio_custom_lut` is `None`).
    #[default]
    NotSet,
    /// LUT loaded and chained onto the display processor.
    Loaded {
        /// Path of the file actually consumed.
        path: PathBuf,
    },
    /// LUT was requested but the load or chain step failed. The
    /// display processor was rebuilt without it; the error message
    /// is suitable for inline UI display.
    Failed {
        /// Path the loader was asked for.
        path: PathBuf,
        /// One-line error description.
        error: String,
    },
}

/// Failure to build a processor or its shaped GPU representation.
#[derive(Debug, thiserror::Error)]
pub enum ColorPipelineError {
    /// OCIO config or processor construction failed.
    #[error(transparent)]
    Ocio(#[from] vfx_ocio::OcioError),
    /// LUT dimensions, allocation, or bake failed.
    #[error("{0}")]
    Lut(String),
    /// Selected OCIO view requires an HDR surface that eframe does not expose.
    #[error(
        "OCIO view '{view}' on display '{display}' requires HDR output; current compositor is SDR"
    )]
    UnsupportedHdrOutput {
        /// OCIO display selected by the active view.
        display: String,
        /// OCIO view that requires an HDR output surface.
        view: String,
    },
}

/// Logarithmic scene-linear domain used by a baked 3D LUT.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct LutShaper {
    /// Lowest non-zero exposure represented by the logarithmic section.
    pub min_ev: f32,
    /// Highest exposure represented without clipping.
    pub max_ev: f32,
}

impl Default for LutShaper {
    fn default() -> Self {
        Self {
            min_ev: -12.0,
            max_ev: 15.0,
        }
    }
}

/// Baked 3D LUT snapshot plus its scene-linear coordinate transform.
#[derive(Clone, Debug)]
pub struct BakedLut3D {
    /// Grid dimensions per axis. The flat buffer length is
    /// `size * size * size * 3` floats.
    pub size: usize,
    /// Scene-linear to LUT-coordinate transform used during this bake.
    pub shaper: LutShaper,
    /// OCIO canonical order: B fastest, then G, then R. The renderer
    /// transposes this into wgpu's X-fastest texture layout on upload.
    pub data: Vec<f32>,
}

/// Default LUT side length. 33 matches the .cube standard and
/// the OCIO Studio config baseline. 65 would be more accurate
/// at 8× the memory; 17 is too coarse for ACES rolloff. 33 is
/// the right compromise for an interactive viewport.
pub const DEFAULT_LUT_SIZE: usize = 33;

fn lut_element_count(size: usize) -> Result<usize, String> {
    size.checked_pow(3)
        .and_then(|count| count.checked_mul(3))
        .ok_or_else(|| format!("3D LUT size {size} overflows usize"))
}

fn shaper_decode(index: usize, size: usize, shaper: LutShaper) -> f32 {
    if index == 0 {
        return 0.0;
    }
    let log_steps = size.saturating_sub(2).max(1);
    let t = (index - 1) as f32 / log_steps as f32;
    (shaper.min_ev + t * (shaper.max_ev - shaper.min_ev)).exp2()
}

fn shaped_lut_inputs(size: usize, shaper: LutShaper) -> Result<Vec<[f32; 3]>, String> {
    if size < 3
        || !shaper.min_ev.is_finite()
        || !shaper.max_ev.is_finite()
        || shaper.min_ev >= shaper.max_ev
    {
        return Err(format!(
            "invalid LUT shaper: size={size}, min_ev={}, max_ev={}",
            shaper.min_ev, shaper.max_ev
        ));
    }
    let texel_count = lut_element_count(size)? / 3;
    let mut inputs = Vec::new();
    inputs
        .try_reserve_exact(texel_count)
        .map_err(|error| format!("cannot allocate {size}³ LUT: {error}"))?;
    for r in 0..size {
        for g in 0..size {
            for b in 0..size {
                inputs.push([
                    shaper_decode(r, size, shaper),
                    shaper_decode(g, size, shaper),
                    shaper_decode(b, size, shaper),
                ]);
            }
        }
    }
    Ok(inputs)
}

fn display_encoded_to_surface_linear(value: f32) -> f32 {
    let value = value.clamp(0.0, 1.0);
    if value <= 0.04045 {
        value / 12.92
    } else {
        ((value + 0.055) / 1.055).powf(2.4)
    }
}

fn bake_shaped_lut(
    processor: &vfx_ocio::Processor,
    size: usize,
    shaper: LutShaper,
    decode_code_values_for_transport: bool,
) -> Result<BakedLut3D, String> {
    let mut samples = shaped_lut_inputs(size, shaper)?;
    processor.apply_rgb(&mut samples);
    let element_count = lut_element_count(size)?;
    let mut data = Vec::new();
    data.try_reserve_exact(element_count)
        .map_err(|error| format!("cannot allocate baked {size}³ LUT: {error}"))?;
    for sample in samples {
        // OCIO display processors return display-encoded values. The
        // eframe consumes a linear offscreen texture and performs the sRGB
        // transfer in its final output stage. Decode here so the transport
        // encode/decode pair preserves OCIO code values without double-OETF.
        data.extend(sample.map(|value| {
            if !value.is_finite() {
                0.0
            } else if decode_code_values_for_transport {
                display_encoded_to_surface_linear(value)
            } else {
                value
            }
        }));
    }
    Ok(BakedLut3D { size, shaper, data })
}

fn identity_lut(size: usize, shaper: LutShaper) -> Result<BakedLut3D, String> {
    let samples = shaped_lut_inputs(size, shaper)?;
    let element_count = lut_element_count(size)?;
    let mut data = Vec::new();
    data.try_reserve_exact(element_count)
        .map_err(|error| format!("cannot allocate identity {size}³ LUT: {error}"))?;
    for sample in samples {
        data.extend(sample);
    }
    Ok(BakedLut3D { size, shaper, data })
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
            lut_3d: None,
            last_hash: 0,
            lut_upload_pending: false,
            custom_lut_status: CustomLutStatus::NotSet,
            decode_code_values_for_transport: false,
            last_error: None,
        };
        let hash = settings.build_hash();
        if let Err(error) = pipe.rebuild(settings) {
            pipe.install_error_fallback(error.to_string());
        }
        pipe.last_hash = hash;
        pipe
    }

    /// Rebuild the processor if `settings.build_hash()` differs
    /// from the cached one. No-op otherwise. Returns `Ok(())` on
    /// success or a no-op skip; `Err` is reserved for hard
    /// build failures that the host wants to surface.
    pub fn ensure(&mut self, settings: &ColorPipelineSettings) -> Result<(), ColorPipelineError> {
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
        let result = self.rebuild(settings);
        self.last_hash = hash;
        match result {
            Ok(()) => {
                self.last_error = None;
                Ok(())
            }
            Err(error) => {
                self.install_error_fallback(error.to_string());
                Err(error)
            }
        }
    }

    fn install_error_fallback(&mut self, mut message: String) {
        log::error!("color-pipeline: rebuild failed: {message}");
        self.processor = None;
        self.decode_code_values_for_transport = false;
        self.lut_3d = match identity_lut(DEFAULT_LUT_SIZE, LutShaper::default()) {
            Ok(identity) => Some(identity),
            Err(fallback_error) => {
                log::error!("color-pipeline: error fallback LUT failed: {fallback_error}");
                message.push_str("; error fallback LUT failed: ");
                message.push_str(&fallback_error);
                None
            }
        };
        self.lut_upload_pending = self.lut_3d.is_some();
        self.last_error = Some(message);
    }

    fn rebuild(&mut self, settings: &ColorPipelineSettings) -> Result<(), ColorPipelineError> {
        if settings.mode == ColorMode::BuiltIn {
            // Built-in tonemaps don't need an OCIO processor —
            // their math is in the blit shader.
            self.processor = None;
            self.decode_code_values_for_transport = false;
            // Mark a pending upload so the host re-pushes the
            // identity LUT (or whatever the renderer's default is)
            // when the user toggles back from OCIO mode. Without
            // this the previous OCIO bake would stay bound until
            // the next OCIO rebuild — case 6u doesn't run in
            // BuiltIn mode so it's not visible, but keeping the
            // bound texture meaningful is the less surprising
            // contract.
            let lut_changed = self.lut_3d.is_some();
            self.lut_3d = None;
            if lut_changed {
                self.lut_upload_pending = true;
            }
            // BuiltIn mode doesn't have a custom LUT slot; surface
            // that clearly to the UI.
            self.custom_lut_status = CustomLutStatus::NotSet;
            return Ok(());
        }
        let output_encoding = self.output_encoding(&settings.ocio_display, &settings.ocio_view);
        if output_encoding == vfx_ocio::Encoding::Hdr {
            return Err(ColorPipelineError::UnsupportedHdrOutput {
                display: settings.ocio_display.clone(),
                view: settings.ocio_view.clone(),
            });
        }
        let decode_code_values_for_transport = !matches!(
            output_encoding,
            vfx_ocio::Encoding::SceneLinear | vfx_ocio::Encoding::DisplayLinear
        );

        // OCIO mode — build a display processor via
        // `DisplayViewTransform`. The user-picked look from the UI
        // (when present) goes into the transform's
        // `looks_override` field, mirroring OCIO C++
        // `DisplayViewTransform::setLooksOverride` /
        // `setLooksOverrideEnabled(true)`.
        let mut dvt = vfx_ocio::DisplayViewTransform {
            src: settings.ocio_input_space.clone(),
            display: settings.ocio_display.clone(),
            view: settings.ocio_view.clone(),
            ..vfx_ocio::DisplayViewTransform::default()
        };
        if let Some(look) = settings.ocio_look.as_deref() {
            if !look.is_empty() {
                dvt = dvt.with_looks_override(look);
            }
        }
        // Build the display processor. If the user picked a custom
        // LUT, treat it as an ACES LMT — the canonical VFX path
        // documented in the ACES spec:
        //
        //   input → ACES2065-1 (interchange) → LUT → ACES2065-1
        //   → ACES RRT/ODT → display
        //
        // i.e. the LUT operates in the AP0 working space, BEFORE the
        // ACES output transform. This matches what Nuke's `OCIOLook`
        // node and Resolve's "ACES Look" slot do. A LUT applied as a
        // post-display "creative" transform (after sRGB encoding)
        // bakes in the OETF and breaks linearity across views — the
        // LMT path keeps the rest of the OCIO pipeline coherent.
        //
        // Routing: build (input → AP0) + Processor::from_transform(LUT) +
        // (AP0 → display) and combine. The third leg re-runs the
        // display chain from AP0 instead of the user-selected input
        // space, so the LMT is sandwiched correctly.
        let proc = match settings.ocio_custom_lut.as_ref() {
            None => {
                self.custom_lut_status = CustomLutStatus::NotSet;
                self.config.processor_for_display_view_transform(&dvt)?
            }
            Some(lut_path) => {
                // ACES configs wire `aces_interchange` to AP0 by
                // spec (`OpenColorIO-Config-ACES` v1.0+); when the
                // role is missing we use the canonical colorspace
                // name directly — `find_view` and `processor()`
                // accept either form.
                let interchange = self
                    .config
                    .role_colorspace("aces_interchange")
                    .unwrap_or("ACES2065-1")
                    .to_string();
                let mut dvt_from_aces = dvt.clone();
                dvt_from_aces.src = interchange.clone();

                let attempt = || -> Result<vfx_ocio::Processor, vfx_ocio::OcioError> {
                    let to_aces = self
                        .config
                        .processor(&settings.ocio_input_space, &interchange)?;
                    let lut_proc = vfx_ocio::Processor::from_transform(
                        &vfx_ocio::Transform::file(lut_path.clone()),
                        vfx_ocio::TransformDirection::Forward,
                    )?;
                    let display_from_aces = self
                        .config
                        .processor_for_display_view_transform(&dvt_from_aces)?;
                    let lmt_chain = vfx_ocio::Processor::combine(&to_aces, &lut_proc)?;
                    vfx_ocio::Processor::combine(&lmt_chain, &display_from_aces)
                };
                match attempt() {
                    Ok(chained) => {
                        self.custom_lut_status = CustomLutStatus::Loaded {
                            path: lut_path.clone(),
                        };
                        chained
                    }
                    Err(e) => {
                        let msg = format!("{e}");
                        log::warn!(
                            "color-pipeline: custom LUT {} failed to load as LMT: {msg} \
                             — rebuilding the display processor without it",
                            lut_path.display()
                        );
                        self.custom_lut_status = CustomLutStatus::Failed {
                            path: lut_path.clone(),
                            error: msg,
                        };
                        self.config.processor_for_display_view_transform(&dvt)?
                    }
                }
            }
        };

        // Bake over a logarithmic scene-linear domain. A plain [0,1]³
        // LUT clips every HDR highlight before the display transform.
        let shaper = LutShaper::default();
        let baked = match bake_shaped_lut(
            &proc,
            DEFAULT_LUT_SIZE,
            shaper,
            decode_code_values_for_transport,
        ) {
            Ok(baked) => baked,
            Err(error) => {
                log::warn!(
                    "color-pipeline: shaped {DEFAULT_LUT_SIZE}³ bake failed: {error}; \
                     uploading a shaped identity LUT instead of retaining stale data"
                );
                identity_lut(DEFAULT_LUT_SIZE, shaper).map_err(|fallback_error| {
                    log::error!(
                        "color-pipeline: identity LUT construction failed after bake error: \
                         {fallback_error}"
                    );
                    ColorPipelineError::Lut(fallback_error)
                })?
            }
        };
        self.processor = Some(proc);
        self.decode_code_values_for_transport = decode_code_values_for_transport;
        self.lut_3d = Some(baked);
        self.lut_upload_pending = true;
        Ok(())
    }

    /// One-shot poll for the host's per-frame GPU upload step.
    /// Returns `Some(&BakedLut3D)` exactly once per fresh bake;
    /// subsequent calls return `None` until the next rebuild
    /// sets the flag again. Caller is expected to push the
    /// returned slice into the renderer's blit-side 3D LUT
    /// binding (see `pt_megakernel::PathTraceCompute::set_blit_lut_3d`).
    /// Outcome of the most recent custom-LUT load attempt (the
    /// `ocio_custom_lut` slot on the live settings). Useful for the
    /// host UI — render a green "Loaded: …" line or a red "Failed:
    /// …" message under the Custom LUT field instead of leaving
    /// the user guessing whether the file took effect.
    pub fn custom_lut_status(&self) -> &CustomLutStatus {
        &self.custom_lut_status
    }

    /// Poll the LUT awaiting a confirmed GPU upload. The value remains
    /// pending until [`Self::mark_lut_uploaded`] is called, so transient
    /// upload failures cannot silently discard a rebuild.
    pub fn pending_lut(&self) -> Option<&BakedLut3D> {
        self.lut_upload_pending
            .then(|| self.lut_3d.as_ref())
            .flatten()
    }

    /// Confirm that the renderer uploaded the current pending LUT.
    pub fn mark_lut_uploaded(&mut self) {
        self.lut_upload_pending = false;
    }

    /// Apply the cached display processor, then convert its encoded
    /// output to the linear values expected by eframe composition.
    /// Eframe's final SDR transfer reconstructs the processor's code values.
    pub fn apply_cpu_to_surface_linear(&self, pixels: &mut [[f32; 3]]) {
        if let Some(processor) = &self.processor {
            processor.apply_rgb(pixels);
            for pixel in pixels {
                *pixel = pixel.map(|value| {
                    if !value.is_finite() {
                        0.0
                    } else if self.decode_code_values_for_transport {
                        display_encoded_to_surface_linear(value)
                    } else {
                        value
                    }
                });
            }
        }
    }

    /// Hard failure from the most recent settings rebuild.
    pub fn last_error(&self) -> Option<&str> {
        self.last_error.as_deref()
    }

    /// Cached 3D LUT for the GPU codepath. `None` when the
    /// pipeline is in `BuiltIn` mode or the bake failed; the
    /// renderer treats either case as "no LUT available, fall
    /// back to the shader-side AgX in `case 6u`".
    pub fn lut_3d(&self) -> Option<&BakedLut3D> {
        self.lut_3d.as_ref()
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

    /// View names available for `display`. Includes both local
    /// `!<View>` entries and `!<Views>` shared-view references the
    /// display pulls from `Config::shared_views`. Empty when the
    /// display isn't registered.
    pub fn available_views(&self, display: &str) -> Vec<String> {
        self.config
            .get_views(display)
            .into_iter()
            .map(|v| v.name().to_string())
            .collect()
    }

    /// Declared encoding of a display/view's final colorspace.
    /// `Unknown` is returned when either selector is stale or the config omits
    /// encoding metadata; callers must not infer HDR from display/view names.
    pub fn output_encoding(&self, display: &str, view: &str) -> vfx_ocio::Encoding {
        self.config
            .find_view(display, view)
            .and_then(|view| self.config.colorspace(view.effective_colorspace(display)))
            .map(vfx_ocio::ColorSpace::encoding)
            .unwrap_or(vfx_ocio::Encoding::Unknown)
    }

    /// Named looks in the active config. The UI surfaces an
    /// empty "no look" entry on top of this list separately.
    pub fn available_looks(&self) -> Vec<String> {
        self.config.looks().names().map(|s| s.to_string()).collect()
    }
}

/// Names of every `.ocio` / `.ocioz` / `.json` file in the
/// bundled OCIO directory (see [`default_bundled_dir`]). Sorted
/// alphabetically. Empty when the directory is missing or empty
/// — callers should fall back to `ConfigSource::BuiltIn` in that
/// case. Associated (not `&self`) because the directory location
/// doesn't depend on the live `ColorPipeline` state.
pub fn available_bundled_configs() -> Vec<String> {
    let dir = default_bundled_dir();
    let Ok(rd) = std::fs::read_dir(&dir) else {
        return Vec::new();
    };
    let mut out: Vec<String> = rd
        .flatten()
        .filter_map(|ent| {
            let p = ent.path();
            let ext = p.extension().and_then(|e| e.to_str())?;
            if !matches!(ext, "ocio" | "ocioz" | "json") {
                return None;
            }
            p.file_name().and_then(|n| n.to_str()).map(str::to_string)
        })
        .collect();
    out.sort();
    out
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
        ConfigSource::BuiltIn => {
            // "BuiltIn" semantically = the default config that ships
            // with vfx-ocio. With the programmatic ACES 1.3 port now
            // retired (kept in-tree as a dormant experiment), the
            // default is the latest embedded release.
            (vfx_ocio::builtin::default_config(), ConfigSource::BuiltIn)
        }
        ConfigSource::Embedded(name) => match vfx_ocio::builtin::embedded::get(name) {
            Some(cfg) => (cfg, ConfigSource::Embedded(name.clone())),
            None => {
                log::warn!(
                    "color-pipeline: embedded config {name:?} not found in registry \
                     — falling back to built-in ACES 1.3",
                );
                (vfx_ocio::builtin::default_config(), ConfigSource::BuiltIn)
            }
        },
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
                    (vfx_ocio::builtin::default_config(), ConfigSource::BuiltIn)
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
                (vfx_ocio::builtin::default_config(), ConfigSource::BuiltIn)
            }
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn logarithmic_shaper_has_exact_black_and_exposure_endpoints() {
        let shaper = LutShaper::default();
        assert_eq!(shaper_decode(0, DEFAULT_LUT_SIZE, shaper), 0.0);
        assert_eq!(
            shaper_decode(1, DEFAULT_LUT_SIZE, shaper),
            shaper.min_ev.exp2()
        );
        assert_eq!(
            shaper_decode(DEFAULT_LUT_SIZE - 1, DEFAULT_LUT_SIZE, shaper),
            shaper.max_ev.exp2()
        );
    }

    #[test]
    fn shaped_identity_preserves_hdr_domain_and_layout() {
        let shaper = LutShaper::default();
        let lut = identity_lut(DEFAULT_LUT_SIZE, shaper).unwrap();
        assert_eq!(lut.data.len(), DEFAULT_LUT_SIZE.pow(3) * 3);
        assert!(lut.data.iter().copied().fold(0.0_f32, f32::max) > 1.0);
    }

    #[test]
    fn srgb_transport_decode_matches_reference_points() {
        assert_eq!(display_encoded_to_surface_linear(0.0), 0.0);
        assert!((display_encoded_to_surface_linear(0.04045) - 0.0031308).abs() < 1.0e-6);
        assert_eq!(display_encoded_to_surface_linear(1.0), 1.0);
    }
}
