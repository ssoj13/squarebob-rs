//! Command-line option parsing.
//!
//! Extracted from main.rs to keep the binary root (`main.rs`) small.
//! The struct `CliOptions`, the parser `parse_args`, the help text
//! `print_help`, and the per-field `parse_*` helpers all live here.

use pt_mats::MaterializeMode;
use render_shared::{OidnModeOption, OidnQualityOption};

use crate::renderer::{self, RenderBackend, RenderMode};

pub(crate) const DEFAULT_SCREENSHOT_FILE: &str = "squarebob_screenshot.png";

/// CLI options parsed from arguments
#[derive(Default, Clone)]
pub struct CliOptions {
    pub path: Option<String>,
    pub mode: Option<RenderMode>,
    pub backend: Option<RenderBackend>,
    pub help: bool,
    pub verbosity: u8, // 0=warn, 1=info, 2=debug, 3=trace
    pub log_file: Option<String>,
    pub log_pt: bool,
    pub log_wf: bool,
    pub log_pg: bool,
    pub log_modules: Option<String>,

    // Screenshot/testing options
    pub screenshot_delay: Option<f32>, // Take screenshot after N seconds
    pub screenshot_path: Option<String>, // Resolved from DEFAULT_SCREENSHOT_FILE in parse_args
    pub exit_after_screenshot: bool,

    // Render settings (for automated testing)
    pub path_tracing: Option<bool>,
    pub wavefront: Option<bool>,

    // Full Render3DOptions overrides
    pub height_mode: Option<renderer::CubeHeightMode>,
    pub height_squared: Option<bool>,
    pub height_scale: Option<f32>,
    pub color_mode: Option<renderer::ColorMode>,
    pub hash_effect: Option<renderer::HashTransformEffect>,
    pub hash_effect_strength: Option<f32>,
    pub animation_time: Option<f32>,
    pub animation_speed: Option<f32>,
    pub hover_mode: Option<renderer::HoverMode>,
    pub hover_outline_width: Option<f32>,
    pub hover_outline_alpha: Option<f32>,
    pub roughness: Option<f32>,
    pub metalness: Option<f32>,
    pub specular_ior: Option<f32>,
    pub xray_alpha: Option<f32>,
    pub flat_shading: Option<bool>,
    pub double_sided: Option<bool>,
    pub materialize_mode: Option<MaterializeMode>,
    pub env_map_intensity: Option<f32>,
    pub env_map_rotation: Option<f32>,
    pub env_map_enabled: Option<bool>,
    pub env_map_visible: Option<bool>,
    pub env_map_path: Option<String>,
    pub env_animate: Option<bool>,
    pub env_speed: Option<f32>,
    pub background_color: Option<[f32; 3]>,
    pub wireframe: Option<bool>,
    pub animate: Option<bool>,
    pub pt_max_bounces: Option<u32>,
    pub pt_samples: Option<u32>,
    pub pt_samples_per_update: Option<u32>,
    pub pt_max_transmission_depth: Option<u32>,
    pub pt_dof_enabled: Option<bool>,
    pub pt_aperture: Option<f32>,
    pub pt_focus_distance: Option<f32>,
    pub pt_env_importance_sampling: Option<bool>,
    pub pt_target_fps: Option<f32>,
    pub pt_auto_spp: Option<bool>,
    pub pt_camera_snap: Option<bool>,
    pub pt_spectral_mode: Option<renderer::SpectralMode>,
    pub pt_spectral_samples: Option<u32>,
    pub pt_spectral_dispersion: Option<bool>,
    pub pt_gpu_bvh: Option<bool>,
    pub pt_bvh_refit: Option<bool>,
    pub pt_russian_roulette: Option<bool>,
    pub pt_adaptive_sampling: Option<bool>,
    pub pt_wavefront_tile_size: Option<u32>,
    pub pt_restir_di: Option<bool>,
    pub pt_restir_gi: Option<bool>,
    pub pt_restir_temporal: Option<bool>,
    pub pt_restir_spatial: Option<bool>,
    pub pt_restir_m_max: Option<u32>,
    pub pt_path_guiding: Option<bool>,
    pub pt_svo_resolution: Option<u32>,
    // OIDN denoiser (replaces the previous à-trous filter).
    pub pt_oidn_mode: Option<OidnModeOption>,
    pub pt_oidn_quality: Option<OidnQualityOption>,
    pub pt_oidn_auto: Option<bool>,
    pub slice_enabled: Option<bool>,
    pub slice_axis: Option<u32>,
    pub slice_position: Option<f32>,
    pub slice_position_vector: Option<f32>,
    pub slice_invert: Option<bool>,
    pub slice_use_vector: Option<bool>,
    pub slice_normal: Option<[f32; 3]>,
    pub lod_enabled: Option<bool>,
    pub lod_min_screen_size: Option<f32>,
    pub inertia_enabled: Option<bool>,
    pub inertia_friction: Option<f32>,
    pub inertia_cutoff: Option<f32>,

    /// Subcommand: `test` with remaining arguments.
    pub test_args: Option<Vec<String>>,
}

pub fn print_help() {
    let bin = env!("CARGO_BIN_NAME");
    let screenshot_file = DEFAULT_SCREENSHOT_FILE;
    eprintln!(
        r#"{bin} - Disk usage visualization tool

USAGE:
    {bin} [OPTIONS] [PATH]

ARGS:
    [PATH]    Path to scan on startup (use -- before a path beginning with -)

OPTIONS:
    -m, --mode <MODE>       Render mode: 2d, 3d (default: 2d)
    -B, --backend <BACK>    Render backend: cpu, gpu (default: cpu, only for 2d mode)
    -v                      INFO level logging
    -vv                     DEBUG level logging
    -vvv                    TRACE level logging
    -l, --log [FILE]        Log to file (default: squarebob-rs.log)
    --log-pt                Force PT module to TRACE
    --log-wf                Force wavefront module to TRACE
    --log-pg                Force pathguide module to TRACE
    --log-modules <LIST>    Force TRACE for modules: pt,wf,pg,oidn (csv)
    --log-ptwf              Alias for --log-modules pt,wf
    --log-ptall             Alias for --log-modules pt,wf,pg
    -h, --help              Print this help message

TESTING OPTIONS:
    --screenshot <SECS>     Take screenshot after N seconds
    --screenshot-path <P>   Screenshot output path (default: OS temp/{screenshot_file})
    --exit-after-screenshot Exit after taking screenshot

RENDER SETTINGS (3D, overrides saved config):
    -p, --path-trace             Enable path tracing
    -P, --no-path-trace          Disable path tracing
    -w, --wavefront              Use wavefront PT (experimental)
    -W, --no-wavefront           Use megakernel PT
    -t, --pt-wavefront-tile <N>  Wavefront tile size in pixels (0 = disabled)
    -b, --bounces <N>            PT max bounces
    -s, --samples <N>            PT max samples
    -u, --pt-spp <N>             PT samples per update
    -g, --pt-gpu-bvh             Enable GPU BVH build
    -G, --no-pt-gpu-bvh          Disable GPU BVH build (force CPU BVH)
    --pt-bvh-refit               Enable BVH refit
    --no-pt-bvh-refit            Disable BVH refit
    -r, --pt-russian-roulette    Enable Russian roulette
    -R, --no-pt-russian-roulette Disable Russian roulette
    --pt-path-guiding            Enable path guiding
    --no-pt-path-guiding         Disable path guiding
    --oidn-mode <MODE>            OIDN mode (off|color|color_albedo|color_albedo_normal)
    --oidn-quality <QUALITY>      OIDN model size (small|base|large)
    --oidn-auto                   Enable automatic OIDN
    --no-oidn-auto                Disable automatic OIDN
    --pt-restir-di               Enable ReSTIR DI
    --no-pt-restir-di            Disable ReSTIR DI
    --pt-restir-gi               Enable ReSTIR GI
    --no-pt-restir-gi            Disable ReSTIR GI
    --pt-restir-temporal         Enable ReSTIR temporal
    --no-pt-restir-temporal      Disable ReSTIR temporal
    --pt-restir-spatial          Enable ReSTIR spatial
    --no-pt-restir-spatial       Disable ReSTIR spatial
    --pt-restir-mmax <N>         ReSTIR M max
    --pt-adaptive-sampling       Enable adaptive sampling
    --no-pt-adaptive-sampling    Disable adaptive sampling
    -d, --pt-dof                 Enable depth of field
    -D, --no-pt-dof              Disable depth of field
    --pt-aperture <F>            DOF aperture
    --pt-focus <F>               DOF focus distance
    --pt-max-transmission <N>    PT max transmission depth
    --pt-env-importance          Enable env importance sampling
    --no-pt-env-importance       Disable env importance sampling
    --pt-target-fps <F>          Target FPS for auto SPP
    --pt-auto-spp                Enable auto SPP
    --no-pt-auto-spp             Disable auto SPP
    -c, --pt-camera-snap         Enable camera snap
    -C, --no-pt-camera-snap      Disable camera snap
    --pt-svo-resolution <N>      SVO resolution (clamped to 16..512)
    --pt-spectral <MODE>         Spectral PT mode (off|hero|multi)
    --pt-spectral-samples <N>    Spectral samples per path (hint)
    --pt-spectral-dispersion     Enable spectral dispersion (hint)
    --no-pt-spectral-dispersion  Disable spectral dispersion
    -e, --env-map                Enable environment map
    -E, --no-env-map             Disable environment map
    --env-intensity <F>          Environment intensity
    --env-rotation <F>           Environment rotation (degrees)
    --env-visible                Show environment
    --no-env-visible             Hide environment
    --env-path <P>               Environment HDR path
    --env-animate                Animate environment rotation
    --no-env-animate             Disable environment animation
    --env-speed <F>              Environment animation speed
    --background-color <R,G,B>   Background color (0-1)
    -f, --wireframe              Enable wireframe
    -F, --no-wireframe           Disable wireframe
    --height-mode <MODE>         Cube height mode (filesize|depth|constant)
    --height-squared             Square the height metric
    --no-height-squared          Disable height squaring
    --height-scale <F>           Cube height scale
    --color-mode <MODE>          Color mode (treemap|filetype|fileage|filesize|depth)
    --hash-effect <EFFECT>       Hash effect (none|wave|random_height|random_offset|explode|noise|pulse|spiral|ocean|rotate_3d|twist|breathe|swarm|earthquake|ripple|vortex|glitch|echo)
    --hash-strength <F>          Hash effect strength
    --animation-time <F>         Animation time (override)
    --animation-speed <F>        Animation speed
    -a, --animate                Enable animation
    -A, --no-animate             Disable animation
    --hover-mode <MODE>          Hover mode (none|outline|tint|both)
    --hover-outline-width <F>    Hover outline width
    --hover-outline-alpha <F>    Hover outline alpha
    --roughness <F>              PBR roughness
    --metalness <F>              PBR metalness
    --specular-ior <F>           PBR specular IOR
    --xray-alpha <F>             X-ray alpha
    --flat-shading               Enable flat shading
    --no-flat-shading            Disable flat shading
    --double-sided               Enable double-sided
    --no-double-sided            Disable double-sided
    --materialize <MODE>         Materialize mode (none|byextension|bypath|bysize|byage|random)
    --slice                       Enable slice plane
    --no-slice                    Disable slice plane
    --slice-axis <N>              Slice axis (0=X,1=Y,2=Z)
    --slice-pos <F>               Slice position (axis mode)
    --slice-pos-vector <F>        Slice position (vector mode)
    --slice-invert                Invert slice
    --no-slice-invert             Disable slice invert
    --slice-use-vector            Use slice normal vector
    --slice-use-axis              Use slice axis
    --slice-normal <X,Y,Z>        Slice normal vector
    --lod                          Enable LOD
    --no-lod                       Disable LOD
    --lod-min-size <F>             LOD min screen size
    --inertia                      Enable camera inertia
    --no-inertia                   Disable camera inertia
    --inertia-friction <F>         Camera inertia friction
    --inertia-cutoff <F>           Camera inertia cutoff

TEST HARNESS (no GUI):
    {bin} test [NAME] [ARGS...]         # See: {bin} test help

EXAMPLES:
    {bin} /home                         # Scan /home with default settings
    {bin} --mode 3d /home               # Scan /home in 3D mode
    {bin} -vv --mode 3d                 # 3D mode with DEBUG logging
    {bin} --mode 3d --path-trace --screenshot 3 .  # Test PT, screenshot after 3s
"#
    );
}

fn parse_vec3(input: &str) -> Option<[f32; 3]> {
    let parts: Vec<&str> = input.split(',').collect();
    if parts.len() != 3 {
        return None;
    }
    let x = parts[0].trim().parse::<f32>().ok()?;
    let y = parts[1].trim().parse::<f32>().ok()?;
    let z = parts[2].trim().parse::<f32>().ok()?;
    if [x, y, z].iter().all(|component| component.is_finite()) {
        Some([x, y, z])
    } else {
        None
    }
}

fn parse_height_mode(input: &str) -> Option<renderer::CubeHeightMode> {
    match input.to_lowercase().as_str() {
        "filesize" | "file_size" | "file" => Some(renderer::CubeHeightMode::FileSize),
        "depth" => Some(renderer::CubeHeightMode::Depth),
        "constant" => Some(renderer::CubeHeightMode::Constant),
        _ => None,
    }
}

fn parse_color_mode(input: &str) -> Option<renderer::ColorMode> {
    match input.to_lowercase().as_str() {
        "treemap" => Some(renderer::ColorMode::Treemap),
        "filetype" | "file_type" => Some(renderer::ColorMode::FileType),
        "fileage" | "file_age" => Some(renderer::ColorMode::FileAge),
        "filesize" | "file_size" => Some(renderer::ColorMode::FileSize),
        "depth" => Some(renderer::ColorMode::Depth),
        _ => None,
    }
}

fn parse_hash_effect(input: &str) -> Option<renderer::HashTransformEffect> {
    match input.to_lowercase().as_str() {
        "none" => Some(renderer::HashTransformEffect::None),
        "wave" => Some(renderer::HashTransformEffect::Wave),
        "randomheight" | "random_height" => Some(renderer::HashTransformEffect::RandomHeight),
        "randomoffset" | "random_offset" => Some(renderer::HashTransformEffect::RandomOffset),
        "explode" => Some(renderer::HashTransformEffect::Explode),
        "noise" => Some(renderer::HashTransformEffect::Noise),
        "pulse" => Some(renderer::HashTransformEffect::Pulse),
        "spiral" => Some(renderer::HashTransformEffect::Spiral),
        "ocean" => Some(renderer::HashTransformEffect::Ocean),
        "rotate3d" | "rotate_3d" => Some(renderer::HashTransformEffect::Rotate3D),
        "twist" => Some(renderer::HashTransformEffect::Twist),
        "breathe" => Some(renderer::HashTransformEffect::Breathe),
        "swarm" => Some(renderer::HashTransformEffect::Swarm),
        "earthquake" => Some(renderer::HashTransformEffect::Earthquake),
        "ripple" => Some(renderer::HashTransformEffect::Ripple),
        "vortex" => Some(renderer::HashTransformEffect::Vortex),
        "glitch" => Some(renderer::HashTransformEffect::Glitch),
        "echo" => Some(renderer::HashTransformEffect::Echo),
        _ => None,
    }
}

fn parse_hover_mode(input: &str) -> Option<renderer::HoverMode> {
    match input.to_lowercase().as_str() {
        "none" => Some(renderer::HoverMode::None),
        "outline" => Some(renderer::HoverMode::Outline),
        "tint" => Some(renderer::HoverMode::Tint),
        "both" => Some(renderer::HoverMode::Both),
        _ => None,
    }
}

fn parse_materialize_mode(input: &str) -> Option<MaterializeMode> {
    match input.to_lowercase().as_str() {
        "none" => Some(MaterializeMode::None),
        "byextension" | "by_extension" => Some(MaterializeMode::ByExtension),
        "bypath" | "by_path" => Some(MaterializeMode::ByPath),
        "bysize" | "by_size" => Some(MaterializeMode::BySize),
        "byage" | "by_age" => Some(MaterializeMode::ByAge),
        "random" => Some(MaterializeMode::Random),
        _ => None,
    }
}

fn parse_spectral_mode(input: &str) -> Option<renderer::SpectralMode> {
    match input.to_lowercase().as_str() {
        "off" | "none" => Some(renderer::SpectralMode::Off),
        "hero" => Some(renderer::SpectralMode::Hero),
        "multi" | "multi_sample" | "multisample" => Some(renderer::SpectralMode::Multi),
        _ => None,
    }
}

fn value<'a>(args: &'a [String], index: &mut usize, option: &str) -> Result<&'a str, String> {
    let next = args
        .get(*index + 1)
        .ok_or_else(|| format!("Missing value for {option}"))?;
    if next.starts_with('-') && next.parse::<f32>().is_err() {
        return Err(format!("Missing value for {option} before option '{next}'"));
    }
    *index += 1;
    Ok(next)
}

fn parse_value<T: std::str::FromStr>(
    args: &[String],
    index: &mut usize,
    option: &str,
) -> Result<T, String> {
    let raw = value(args, index, option)?;
    raw.parse::<T>()
        .map_err(|_| format!("Invalid value '{raw}' for {option}"))
}

fn finite(value: f32, option: &str) -> Result<f32, String> {
    if value.is_finite() {
        Ok(value)
    } else {
        Err(format!("Non-finite value for {option}"))
    }
}

pub fn parse_args() -> Result<CliOptions, String> {
    let mut opts = CliOptions::default();
    let args: Vec<String> = std::env::args().skip(1).collect();
    let mut i = 0;

    while i < args.len() {
        let arg = args[i].as_str();
        match arg {
            "test" => {
                opts.test_args = Some(args[i + 1..].to_vec());
                break;
            }
            "-h" | "--help" => {
                opts.help = true;
                return Ok(opts);
            }
            "-v" => opts.verbosity = opts.verbosity.max(1),
            "-vv" => opts.verbosity = opts.verbosity.max(2),
            "-vvv" => opts.verbosity = opts.verbosity.max(3),
            "-l" | "--log" => {
                if args.get(i + 1).is_some_and(|next| !next.starts_with('-')) {
                    opts.log_file = Some(value(&args, &mut i, arg)?.to_owned());
                } else {
                    opts.log_file = Some("squarebob-rs.log".to_owned());
                }
            }
            "--log-modules" => opts.log_modules = Some(value(&args, &mut i, arg)?.to_owned()),
            "--log-ptwf" => opts.log_modules = Some("pt,wf".to_owned()),
            "--log-ptall" => opts.log_modules = Some("pt,wf,pg".to_owned()),
            "--log-pt" => opts.log_pt = true,
            "--log-wf" => opts.log_wf = true,
            "--log-pg" => opts.log_pg = true,
            "-m" | "--mode" => {
                let raw = value(&args, &mut i, arg)?;
                opts.mode = Some(match raw.to_ascii_lowercase().as_str() {
                    "2d" => RenderMode::Mode2D,
                    "3d" => RenderMode::Mode3D,
                    _ => return Err(format!("Invalid value '{raw}' for {arg}")),
                });
            }
            "-B" | "--backend" => {
                let raw = value(&args, &mut i, arg)?;
                opts.backend = Some(match raw.to_ascii_lowercase().as_str() {
                    "cpu" => RenderBackend::Cpu,
                    "gpu" => RenderBackend::Gpu,
                    _ => return Err(format!("Invalid value '{raw}' for {arg}")),
                });
            }
            "--screenshot" => {
                let delay = finite(parse_value::<f32>(&args, &mut i, arg)?, arg)?;
                if delay < 0.0 {
                    return Err(format!("Screenshot delay must be non-negative: {delay}"));
                }
                opts.screenshot_delay = Some(delay);
            }
            "--screenshot-path" => {
                opts.screenshot_path = Some(value(&args, &mut i, arg)?.to_owned())
            }
            "--exit-after-screenshot" => opts.exit_after_screenshot = true,
            "-p" | "--path-trace" => opts.path_tracing = Some(true),
            "-P" | "--no-path-trace" => opts.path_tracing = Some(false),
            "-w" | "--wavefront" => opts.wavefront = Some(true),
            "-W" | "--no-wavefront" => opts.wavefront = Some(false),
            "-e" | "--env-map" => opts.env_map_enabled = Some(true),
            "-E" | "--no-env-map" => opts.env_map_enabled = Some(false),
            "-f" | "--wireframe" => opts.wireframe = Some(true),
            "--pt-path-guiding" => opts.pt_path_guiding = Some(true),
            "--no-pt-path-guiding" => opts.pt_path_guiding = Some(false),
            "--oidn-auto" => opts.pt_oidn_auto = Some(true),
            "--no-oidn-auto" => opts.pt_oidn_auto = Some(false),
            "--pt-restir-di" => opts.pt_restir_di = Some(true),
            "--no-pt-restir-di" => opts.pt_restir_di = Some(false),
            "--pt-restir-gi" => opts.pt_restir_gi = Some(true),
            "--no-pt-restir-gi" => opts.pt_restir_gi = Some(false),
            "--pt-adaptive-sampling" => opts.pt_adaptive_sampling = Some(true),
            "--no-pt-adaptive-sampling" => opts.pt_adaptive_sampling = Some(false),
            "-G" | "--no-pt-gpu-bvh" => opts.pt_gpu_bvh = Some(false),
            "-a" | "--animate" => opts.animate = Some(true),
            "-A" | "--no-animate" => opts.animate = Some(false),
            "-F" | "--no-wireframe" => opts.wireframe = Some(false),
            "-g" | "--pt-gpu-bvh" => opts.pt_gpu_bvh = Some(true),
            "--pt-bvh-refit" => opts.pt_bvh_refit = Some(true),
            "--no-pt-bvh-refit" => opts.pt_bvh_refit = Some(false),
            "-r" | "--pt-russian-roulette" => opts.pt_russian_roulette = Some(true),
            "-R" | "--no-pt-russian-roulette" => opts.pt_russian_roulette = Some(false),
            "-d" | "--pt-dof" => opts.pt_dof_enabled = Some(true),
            "-D" | "--no-pt-dof" => opts.pt_dof_enabled = Some(false),
            "-c" | "--pt-camera-snap" => opts.pt_camera_snap = Some(true),
            "-C" | "--no-pt-camera-snap" => opts.pt_camera_snap = Some(false),
            "--pt-env-importance" => opts.pt_env_importance_sampling = Some(true),
            "--no-pt-env-importance" => opts.pt_env_importance_sampling = Some(false),
            "--pt-auto-spp" => opts.pt_auto_spp = Some(true),
            "--no-pt-auto-spp" => opts.pt_auto_spp = Some(false),
            "--pt-spectral-dispersion" => opts.pt_spectral_dispersion = Some(true),
            "--no-pt-spectral-dispersion" => opts.pt_spectral_dispersion = Some(false),
            "--pt-restir-temporal" => opts.pt_restir_temporal = Some(true),
            "--no-pt-restir-temporal" => opts.pt_restir_temporal = Some(false),
            "--pt-restir-spatial" => opts.pt_restir_spatial = Some(true),
            "--no-pt-restir-spatial" => opts.pt_restir_spatial = Some(false),
            "--height-squared" => opts.height_squared = Some(true),
            "--no-height-squared" => opts.height_squared = Some(false),
            "--flat-shading" => opts.flat_shading = Some(true),
            "--no-flat-shading" => opts.flat_shading = Some(false),
            "--double-sided" => opts.double_sided = Some(true),
            "--no-double-sided" => opts.double_sided = Some(false),
            "--env-visible" => opts.env_map_visible = Some(true),
            "--no-env-visible" => opts.env_map_visible = Some(false),
            "--env-animate" => opts.env_animate = Some(true),
            "--no-env-animate" => opts.env_animate = Some(false),
            "--slice" => opts.slice_enabled = Some(true),
            "--no-slice" => opts.slice_enabled = Some(false),
            "--slice-invert" => opts.slice_invert = Some(true),
            "--no-slice-invert" => opts.slice_invert = Some(false),
            "--slice-use-vector" => opts.slice_use_vector = Some(true),
            "--slice-use-axis" => opts.slice_use_vector = Some(false),
            "--lod" => opts.lod_enabled = Some(true),
            "--no-lod" => opts.lod_enabled = Some(false),
            "--inertia" => opts.inertia_enabled = Some(true),
            "--no-inertia" => opts.inertia_enabled = Some(false),
            "-t" | "--pt-wavefront-tile" => {
                opts.pt_wavefront_tile_size = Some(parse_value::<u32>(&args, &mut i, arg)?);
            }
            "-b" | "--bounces" => {
                opts.pt_max_bounces = Some(parse_value::<u32>(&args, &mut i, arg)?)
            }
            "-s" | "--samples" => opts.pt_samples = Some(parse_value::<u32>(&args, &mut i, arg)?),
            "--env-path" => opts.env_map_path = Some(value(&args, &mut i, arg)?.to_owned()),
            "--oidn-mode" => {
                let raw = value(&args, &mut i, arg)?;
                opts.pt_oidn_mode = Some(match raw.to_ascii_lowercase().as_str() {
                    "off" => OidnModeOption::Off,
                    "color" => OidnModeOption::Color,
                    "color_albedo" | "color+albedo" => OidnModeOption::ColorAlbedo,
                    "color_albedo_normal" | "color+albedo+normal" => {
                        OidnModeOption::ColorAlbedoNormal
                    }
                    _ => return Err(format!("Invalid value '{raw}' for {arg}")),
                });
            }
            "--oidn-quality" => {
                let raw = value(&args, &mut i, arg)?;
                opts.pt_oidn_quality = Some(match raw.to_ascii_lowercase().as_str() {
                    "large" | "high" => OidnQualityOption::Large,
                    "base" | "balanced" => OidnQualityOption::Base,
                    "small" | "fast" => OidnQualityOption::Small,
                    _ => return Err(format!("Invalid value '{raw}' for {arg}")),
                });
            }
            "-u" | "--pt-spp" => {
                opts.pt_samples_per_update = Some(parse_value::<u32>(&args, &mut i, arg)?)
            }
            "--pt-svo-resolution" => {
                opts.pt_svo_resolution =
                    Some(parse_value::<u32>(&args, &mut i, arg)?.clamp(16, 512));
            }
            "--pt-spectral" => {
                let raw = value(&args, &mut i, arg)?;
                opts.pt_spectral_mode = Some(
                    parse_spectral_mode(raw)
                        .ok_or_else(|| format!("Invalid value '{raw}' for {arg}"))?,
                );
            }
            "--height-mode" => {
                let raw = value(&args, &mut i, arg)?;
                let mode = raw.to_ascii_lowercase();
                if matches!(mode.as_str(), "depth2" | "depth_squared" | "depthsquared") {
                    opts.height_mode = Some(renderer::CubeHeightMode::Depth);
                    opts.height_squared = Some(true);
                } else {
                    opts.height_mode = Some(
                        parse_height_mode(raw)
                            .ok_or_else(|| format!("Invalid value '{raw}' for {arg}"))?,
                    );
                }
            }
            "--color-mode" => {
                let raw = value(&args, &mut i, arg)?;
                opts.color_mode = Some(
                    parse_color_mode(raw)
                        .ok_or_else(|| format!("Invalid value '{raw}' for {arg}"))?,
                );
            }
            "--hash-effect" => {
                let raw = value(&args, &mut i, arg)?;
                opts.hash_effect = Some(
                    parse_hash_effect(raw)
                        .ok_or_else(|| format!("Invalid value '{raw}' for {arg}"))?,
                );
            }
            "--hover-mode" => {
                let raw = value(&args, &mut i, arg)?;
                opts.hover_mode = Some(
                    parse_hover_mode(raw)
                        .ok_or_else(|| format!("Invalid value '{raw}' for {arg}"))?,
                );
            }
            "--materialize" => {
                let raw = value(&args, &mut i, arg)?;
                opts.materialize_mode = Some(
                    parse_materialize_mode(raw)
                        .ok_or_else(|| format!("Invalid value '{raw}' for {arg}"))?,
                );
            }
            "--background-color" => {
                let raw = value(&args, &mut i, arg)?;
                opts.background_color = Some(parse_vec3(raw).ok_or_else(|| {
                    format!("Invalid value '{raw}' for {arg}: expected finite R,G,B")
                })?);
            }
            "--slice-normal" => {
                let raw = value(&args, &mut i, arg)?;
                opts.slice_normal = Some(parse_vec3(raw).ok_or_else(|| {
                    format!("Invalid value '{raw}' for {arg}: expected finite X,Y,Z")
                })?);
            }
            "--pt-max-transmission" => {
                opts.pt_max_transmission_depth = Some(parse_value::<u32>(&args, &mut i, arg)?)
            }
            "--pt-aperture" => {
                opts.pt_aperture = Some(finite(parse_value::<f32>(&args, &mut i, arg)?, arg)?)
            }
            "--pt-focus" => {
                opts.pt_focus_distance = Some(finite(parse_value::<f32>(&args, &mut i, arg)?, arg)?)
            }
            "--pt-target-fps" => {
                opts.pt_target_fps = Some(finite(parse_value::<f32>(&args, &mut i, arg)?, arg)?)
            }
            "--pt-spectral-samples" => {
                opts.pt_spectral_samples = Some(parse_value::<u32>(&args, &mut i, arg)?)
            }
            "--pt-restir-mmax" => {
                opts.pt_restir_m_max = Some(parse_value::<u32>(&args, &mut i, arg)?)
            }
            "--height-scale" => {
                opts.height_scale = Some(finite(parse_value::<f32>(&args, &mut i, arg)?, arg)?)
            }
            "--hash-strength" => {
                opts.hash_effect_strength =
                    Some(finite(parse_value::<f32>(&args, &mut i, arg)?, arg)?)
            }
            "--animation-time" => {
                opts.animation_time = Some(finite(parse_value::<f32>(&args, &mut i, arg)?, arg)?)
            }
            "--animation-speed" => {
                opts.animation_speed = Some(finite(parse_value::<f32>(&args, &mut i, arg)?, arg)?)
            }
            "--hover-outline-width" => {
                opts.hover_outline_width =
                    Some(finite(parse_value::<f32>(&args, &mut i, arg)?, arg)?)
            }
            "--hover-outline-alpha" => {
                opts.hover_outline_alpha =
                    Some(finite(parse_value::<f32>(&args, &mut i, arg)?, arg)?)
            }
            "--roughness" => {
                opts.roughness = Some(finite(parse_value::<f32>(&args, &mut i, arg)?, arg)?)
            }
            "--metalness" => {
                opts.metalness = Some(finite(parse_value::<f32>(&args, &mut i, arg)?, arg)?)
            }
            "--specular-ior" => {
                opts.specular_ior = Some(finite(parse_value::<f32>(&args, &mut i, arg)?, arg)?)
            }
            "--xray-alpha" => {
                opts.xray_alpha = Some(finite(parse_value::<f32>(&args, &mut i, arg)?, arg)?)
            }
            "--env-intensity" => {
                opts.env_map_intensity = Some(finite(parse_value::<f32>(&args, &mut i, arg)?, arg)?)
            }
            "--env-rotation" => {
                opts.env_map_rotation = Some(finite(parse_value::<f32>(&args, &mut i, arg)?, arg)?)
            }
            "--env-speed" => {
                opts.env_speed = Some(finite(parse_value::<f32>(&args, &mut i, arg)?, arg)?)
            }
            "--slice-axis" => opts.slice_axis = Some(parse_value::<u32>(&args, &mut i, arg)?),
            "--slice-pos" => {
                opts.slice_position = Some(finite(parse_value::<f32>(&args, &mut i, arg)?, arg)?)
            }
            "--slice-pos-vector" => {
                opts.slice_position_vector =
                    Some(finite(parse_value::<f32>(&args, &mut i, arg)?, arg)?)
            }
            "--lod-min-size" => {
                opts.lod_min_screen_size =
                    Some(finite(parse_value::<f32>(&args, &mut i, arg)?, arg)?)
            }
            "--inertia-friction" => {
                opts.inertia_friction = Some(finite(parse_value::<f32>(&args, &mut i, arg)?, arg)?)
            }
            "--inertia-cutoff" => {
                opts.inertia_cutoff = Some(finite(parse_value::<f32>(&args, &mut i, arg)?, arg)?)
            }
            "--" => {
                let path = args.get(i + 1).ok_or("Missing path after --")?;
                if i + 2 != args.len() {
                    return Err("Only one path can follow --".to_owned());
                }
                opts.path = Some(path.clone());
                break;
            }
            _ if arg.starts_with('-') => return Err(format!("Unknown option '{arg}'")),
            _ => opts.path = Some(arg.to_owned()),
        }
        i += 1;
    }

    if opts.screenshot_delay.is_some() && opts.screenshot_path.is_none() {
        opts.screenshot_path = Some(
            std::env::temp_dir()
                .join(DEFAULT_SCREENSHOT_FILE)
                .to_string_lossy()
                .into_owned(),
        );
    }

    Ok(opts)
}
