//! Command-line option parsing.
//!
//! Extracted from main.rs to keep the binary root (`main.rs`) small.
//! The struct `CliOptions`, the parser `parse_args`, the help text
//! `print_help`, and the per-field `parse_*` helpers all live here.

use pt_mats::MaterializeMode;

use crate::renderer::{self, RenderBackend, RenderMode};

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
    pub screenshot_path: Option<String>, // Default: temp/screenshot.png
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
    pub mat_allow_lights: Option<bool>,
    pub mat_light_prob: Option<f32>,
    pub mat_allow_glass: Option<bool>,
    pub mat_glass_prob: Option<f32>,
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
    pub pt_max_samples: Option<u32>,
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
    pub pt_denoise_enabled: Option<bool>,
    pub pt_denoise_iterations: Option<u32>,
    pub pt_denoise_sigma_color: Option<f32>,
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

    /// Subcommand: `test` with remaining args (`dirstat-rs test ping`, etc.).
    pub test_args: Option<Vec<String>>,
}

pub fn print_help() {
    eprintln!(
        r#"dirstat-rs - Disk usage visualization tool

USAGE:
    dirstat-rs [OPTIONS] [PATH]

ARGS:
    [PATH]    Path to scan on startup

OPTIONS:
    -m, --mode <MODE>       Render mode: 2d, 3d (default: 2d)
    -B, --backend <BACK>    Render backend: cpu, gpu (default: cpu, only for 2d mode)
    -v                      INFO level logging
    -vv                     DEBUG level logging
    -vvv                    TRACE level logging
    -l, --log [FILE]        Log to file (default: dirstat-rs.log)
    --log-pt                Force PT module to TRACE
    --log-wf                Force wavefront module to TRACE
    --log-pg                Force pathguide module to TRACE
    --log-modules <LIST>    Force TRACE for modules: pt,wf,pg (csv)
    --log-ptwf              Alias for --log-modules pt,wf
    --log-ptall             Alias for --log-modules pt,wf,pg
    -h, --help              Print this help message

TESTING OPTIONS:
    --screenshot <SECS>     Take screenshot after N seconds
    --screenshot-path <P>   Screenshot output path (default: temp/screenshot.png)
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
    --mat-allow-lights           Allow emissive materials
    --no-mat-allow-lights        Disallow emissive materials
    --mat-light-prob <F>         Emissive material probability
    --mat-allow-glass            Allow glass materials
    --no-mat-allow-glass         Disallow glass materials
    --mat-glass-prob <F>         Glass material probability
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
    dirstat-rs test [NAME] [ARGS...]         # See: dirstat-rs test help

EXAMPLES:
    dirstat-rs /home                         # Scan /home with default settings
    dirstat-rs --mode 3d /home               # Scan /home in 3D mode
    dirstat-rs -vv --mode 3d                 # 3D mode with DEBUG logging
    dirstat-rs --mode 3d --path-trace --screenshot 3 .  # Test PT, screenshot after 3s
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
    Some([x, y, z])
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

pub fn parse_args() -> CliOptions {
    let mut opts = CliOptions::default();
    let args: Vec<String> = std::env::args().skip(1).collect();
    let mut i = 0;

    while i < args.len() {
        let arg = &args[i];
        match arg.as_str() {
            "test" => {
                opts.test_args = Some(args[i + 1..].to_vec());
                break;
            }
            "-h" | "--help" => {
                opts.help = true;
                return opts;
            }
            "-v" => opts.verbosity = opts.verbosity.max(1),
            "-vv" => opts.verbosity = opts.verbosity.max(2),
            "-vvv" => opts.verbosity = opts.verbosity.max(3),
            "-l" | "--log" => {
                i += 1;
                if i < args.len() && !args[i].starts_with('-') {
                    opts.log_file = Some(args[i].clone());
                } else {
                    opts.log_file = Some("dirstat-rs.log".to_string());
                    if i < args.len() {
                        i -= 1;
                    }
                }
            }
            "--log-modules" => {
                i += 1;
                if i < args.len() {
                    opts.log_modules = Some(args[i].clone());
                } else {
                    eprintln!("Missing value for --log-modules, ignoring");
                }
            }
            "--log-ptwf" => {
                opts.log_modules = Some("pt,wf".to_string());
            }
            "--log-ptall" => {
                opts.log_modules = Some("pt,wf,pg".to_string());
            }
            "--log-pt" => {
                opts.log_pt = true;
            }
            "--log-wf" => {
                opts.log_wf = true;
            }
            "--log-pg" => {
                opts.log_pg = true;
            }
            "-m" | "--mode" => {
                i += 1;
                if i < args.len() {
                    opts.mode = match args[i].to_lowercase().as_str() {
                        "2d" => Some(RenderMode::Mode2D),
                        "3d" => Some(RenderMode::Mode3D),
                        other => {
                            eprintln!("Unknown mode '{}', using default", other);
                            None
                        }
                    };
                }
            }
            "-B" | "--backend" => {
                i += 1;
                if i < args.len() {
                    opts.backend = match args[i].to_lowercase().as_str() {
                        "cpu" => Some(RenderBackend::Cpu),
                        "gpu" => Some(RenderBackend::Gpu),
                        other => {
                            eprintln!("Unknown backend '{}', using default", other);
                            None
                        }
                    };
                }
            }
            // Screenshot options
            "--screenshot" => {
                i += 1;
                if i < args.len() {
                    if let Ok(secs) = args[i].parse::<f32>() {
                        opts.screenshot_delay = Some(secs);
                    } else {
                        eprintln!("Invalid screenshot delay '{}', ignoring", args[i]);
                    }
                }
            }
            "--screenshot-path" => {
                i += 1;
                if i < args.len() {
                    opts.screenshot_path = Some(args[i].clone());
                }
            }
            "--exit-after-screenshot" => {
                opts.exit_after_screenshot = true;
            }
            // Render settings
            "-p" | "--path-trace" => {
                opts.path_tracing = Some(true);
            }
            "-P" | "--no-path-trace" => {
                opts.path_tracing = Some(false);
            }
            "-w" | "--wavefront" => {
                opts.wavefront = Some(true);
            }
            "-W" | "--no-wavefront" => {
                opts.wavefront = Some(false);
            }
            "-t" | "--pt-wavefront-tile" => {
                i += 1;
                if i < args.len() {
                    opts.pt_wavefront_tile_size = args[i].parse::<u32>().ok();
                    if opts.pt_wavefront_tile_size.is_none() {
                        eprintln!("Invalid value for --pt-wavefront-tile, ignoring");
                    }
                } else {
                    eprintln!("Missing value for --pt-wavefront-tile, ignoring");
                }
            }
            "-b" | "--bounces" => {
                i += 1;
                if i < args.len() {
                    if let Ok(n) = args[i].parse::<u32>() {
                        opts.pt_max_bounces = Some(n);
                    }
                }
            }
            "-s" | "--samples" => {
                i += 1;
                if i < args.len() {
                    if let Ok(n) = args[i].parse::<u32>() {
                        opts.pt_max_samples = Some(n);
                    }
                }
            }
            "-e" | "--env-map" => {
                opts.env_map_enabled = Some(true);
            }
            "-E" | "--no-env-map" => {
                opts.env_map_enabled = Some(false);
            }
            "-f" | "--wireframe" => {
                opts.wireframe = Some(true);
            }
            "--pt-path-guiding" => {
                opts.pt_path_guiding = Some(true);
            }
            "--no-pt-path-guiding" => {
                opts.pt_path_guiding = Some(false);
            }
            "--pt-denoise" => {
                opts.pt_denoise_enabled = Some(true);
            }
            "--no-pt-denoise" => {
                opts.pt_denoise_enabled = Some(false);
            }
            "--pt-denoise-iterations" => {
                i += 1;
                if i < args.len() {
                    if let Ok(n) = args[i].parse::<u32>() {
                        opts.pt_denoise_iterations = Some(n.clamp(1, 5));
                    }
                }
            }
            "--pt-denoise-sigma-color" => {
                i += 1;
                if i < args.len() {
                    if let Ok(v) = args[i].parse::<f32>() {
                        opts.pt_denoise_sigma_color = Some(v.max(1e-3));
                    }
                }
            }
            "--pt-restir-di" => {
                opts.pt_restir_di = Some(true);
            }
            "--no-pt-restir-di" => {
                opts.pt_restir_di = Some(false);
            }
            "--pt-restir-gi" => {
                opts.pt_restir_gi = Some(true);
            }
            "--no-pt-restir-gi" => {
                opts.pt_restir_gi = Some(false);
            }
            "--pt-adaptive-sampling" => {
                opts.pt_adaptive_sampling = Some(true);
            }
            "--no-pt-adaptive-sampling" => {
                opts.pt_adaptive_sampling = Some(false);
            }
            "--no-pt-gpu-bvh" => {
                opts.pt_gpu_bvh = Some(false);
            }
            "-a" | "--animate" => {
                opts.animate = Some(true);
            }
            "-A" | "--no-animate" => {
                opts.animate = Some(false);
            }
            "-F" | "--no-wireframe" => {
                opts.wireframe = Some(false);
            }
            "-g" | "--pt-gpu-bvh" => {
                opts.pt_gpu_bvh = Some(true);
            }
            "--pt-bvh-refit" => {
                opts.pt_bvh_refit = Some(true);
            }
            "--no-pt-bvh-refit" => {
                opts.pt_bvh_refit = Some(false);
            }
            "-r" | "--pt-russian-roulette" => {
                opts.pt_russian_roulette = Some(true);
            }
            "-R" | "--no-pt-russian-roulette" => {
                opts.pt_russian_roulette = Some(false);
            }
            "-d" | "--pt-dof" => {
                opts.pt_dof_enabled = Some(true);
            }
            "-D" | "--no-pt-dof" => {
                opts.pt_dof_enabled = Some(false);
            }
            "-c" | "--pt-camera-snap" => {
                opts.pt_camera_snap = Some(true);
            }
            "-C" | "--no-pt-camera-snap" => {
                opts.pt_camera_snap = Some(false);
            }
            "-u" | "--pt-spp" => {
                i += 1;
                if i < args.len() {
                    opts.pt_samples_per_update = args[i].parse::<u32>().ok();
                    if opts.pt_samples_per_update.is_none() {
                        eprintln!("Invalid value for --pt-spp, ignoring");
                    }
                } else {
                    eprintln!("Missing value for --pt-spp, ignoring");
                }
            }
            "--pt-max-transmission" => {
                i += 1;
                if i < args.len() {
                    opts.pt_max_transmission_depth = args[i].parse::<u32>().ok();
                }
            }
            "--pt-aperture" => {
                i += 1;
                if i < args.len() {
                    opts.pt_aperture = args[i].parse::<f32>().ok();
                }
            }
            "--pt-focus" => {
                i += 1;
                if i < args.len() {
                    opts.pt_focus_distance = args[i].parse::<f32>().ok();
                }
            }
            "--pt-env-importance" => {
                opts.pt_env_importance_sampling = Some(true);
            }
            "--no-pt-env-importance" => {
                opts.pt_env_importance_sampling = Some(false);
            }
            "--pt-target-fps" => {
                i += 1;
                if i < args.len() {
                    opts.pt_target_fps = args[i].parse::<f32>().ok();
                }
            }
            "--pt-auto-spp" => {
                opts.pt_auto_spp = Some(true);
            }
            "--no-pt-auto-spp" => {
                opts.pt_auto_spp = Some(false);
            }
            "--pt-spectral" => {
                i += 1;
                if i < args.len() {
                    opts.pt_spectral_mode = parse_spectral_mode(&args[i]);
                    if opts.pt_spectral_mode.is_none() {
                        eprintln!("Invalid value for --pt-spectral, ignoring");
                    }
                } else {
                    eprintln!("Missing value for --pt-spectral, ignoring");
                }
            }
            "--pt-spectral-samples" => {
                i += 1;
                if i < args.len() {
                    opts.pt_spectral_samples = args[i].parse::<u32>().ok();
                    if opts.pt_spectral_samples.is_none() {
                        eprintln!("Invalid value for --pt-spectral-samples, ignoring");
                    }
                } else {
                    eprintln!("Missing value for --pt-spectral-samples, ignoring");
                }
            }
            "--pt-spectral-dispersion" => {
                opts.pt_spectral_dispersion = Some(true);
            }
            "--no-pt-spectral-dispersion" => {
                opts.pt_spectral_dispersion = Some(false);
            }
            "--pt-restir-temporal" => {
                opts.pt_restir_temporal = Some(true);
            }
            "--no-pt-restir-temporal" => {
                opts.pt_restir_temporal = Some(false);
            }
            "--pt-restir-spatial" => {
                opts.pt_restir_spatial = Some(true);
            }
            "--no-pt-restir-spatial" => {
                opts.pt_restir_spatial = Some(false);
            }
            "--pt-restir-mmax" => {
                i += 1;
                if i < args.len() {
                    opts.pt_restir_m_max = args[i].parse::<u32>().ok();
                }
            }
            "--pt-svo-resolution" => {
                i += 1;
                if i < args.len() {
                    opts.pt_svo_resolution = args[i].parse::<u32>().ok().map(|v| v.clamp(16, 512));
                }
            }
            "--height-mode" => {
                i += 1;
                if i < args.len() {
                    let mode_lc = args[i].to_lowercase();
                    if matches!(
                        mode_lc.as_str(),
                        "depth2" | "depth_squared" | "depthsquared"
                    ) {
                        opts.height_mode = Some(renderer::CubeHeightMode::Depth);
                        opts.height_squared = Some(true);
                    } else {
                        opts.height_mode = parse_height_mode(&args[i]);
                    }
                }
            }
            "--height-squared" => {
                opts.height_squared = Some(true);
            }
            "--no-height-squared" => {
                opts.height_squared = Some(false);
            }
            "--height-scale" => {
                i += 1;
                if i < args.len() {
                    opts.height_scale = args[i].parse::<f32>().ok();
                }
            }
            "--color-mode" => {
                i += 1;
                if i < args.len() {
                    opts.color_mode = parse_color_mode(&args[i]);
                }
            }
            "--hash-effect" => {
                i += 1;
                if i < args.len() {
                    opts.hash_effect = parse_hash_effect(&args[i]);
                }
            }
            "--hash-strength" => {
                i += 1;
                if i < args.len() {
                    opts.hash_effect_strength = args[i].parse::<f32>().ok();
                }
            }
            "--animation-time" => {
                i += 1;
                if i < args.len() {
                    opts.animation_time = args[i].parse::<f32>().ok();
                }
            }
            "--animation-speed" => {
                i += 1;
                if i < args.len() {
                    opts.animation_speed = args[i].parse::<f32>().ok();
                }
            }
            "--hover-mode" => {
                i += 1;
                if i < args.len() {
                    opts.hover_mode = parse_hover_mode(&args[i]);
                }
            }
            "--hover-outline-width" => {
                i += 1;
                if i < args.len() {
                    opts.hover_outline_width = args[i].parse::<f32>().ok();
                }
            }
            "--hover-outline-alpha" => {
                i += 1;
                if i < args.len() {
                    opts.hover_outline_alpha = args[i].parse::<f32>().ok();
                }
            }
            "--roughness" => {
                i += 1;
                if i < args.len() {
                    opts.roughness = args[i].parse::<f32>().ok();
                }
            }
            "--metalness" => {
                i += 1;
                if i < args.len() {
                    opts.metalness = args[i].parse::<f32>().ok();
                }
            }
            "--specular-ior" => {
                i += 1;
                if i < args.len() {
                    opts.specular_ior = args[i].parse::<f32>().ok();
                }
            }
            "--xray-alpha" => {
                i += 1;
                if i < args.len() {
                    opts.xray_alpha = args[i].parse::<f32>().ok();
                }
            }
            "--flat-shading" => {
                opts.flat_shading = Some(true);
            }
            "--no-flat-shading" => {
                opts.flat_shading = Some(false);
            }
            "--double-sided" => {
                opts.double_sided = Some(true);
            }
            "--no-double-sided" => {
                opts.double_sided = Some(false);
            }
            "--materialize" => {
                i += 1;
                if i < args.len() {
                    opts.materialize_mode = parse_materialize_mode(&args[i]);
                }
            }
            "--mat-allow-lights" => {
                opts.mat_allow_lights = Some(true);
            }
            "--no-mat-allow-lights" => {
                opts.mat_allow_lights = Some(false);
            }
            "--mat-light-prob" => {
                i += 1;
                if i < args.len() {
                    opts.mat_light_prob = args[i].parse::<f32>().ok();
                }
            }
            "--mat-allow-glass" => {
                opts.mat_allow_glass = Some(true);
            }
            "--no-mat-allow-glass" => {
                opts.mat_allow_glass = Some(false);
            }
            "--mat-glass-prob" => {
                i += 1;
                if i < args.len() {
                    opts.mat_glass_prob = args[i].parse::<f32>().ok();
                }
            }
            "--env-intensity" => {
                i += 1;
                if i < args.len() {
                    opts.env_map_intensity = args[i].parse::<f32>().ok();
                }
            }
            "--env-rotation" => {
                i += 1;
                if i < args.len() {
                    opts.env_map_rotation = args[i].parse::<f32>().ok();
                }
            }
            "--env-visible" => {
                opts.env_map_visible = Some(true);
            }
            "--no-env-visible" => {
                opts.env_map_visible = Some(false);
            }
            "--env-path" => {
                i += 1;
                if i < args.len() {
                    opts.env_map_path = Some(args[i].clone());
                }
            }
            "--env-animate" => {
                opts.env_animate = Some(true);
            }
            "--no-env-animate" => {
                opts.env_animate = Some(false);
            }
            "--env-speed" => {
                i += 1;
                if i < args.len() {
                    opts.env_speed = args[i].parse::<f32>().ok();
                }
            }
            "--background-color" => {
                i += 1;
                if i < args.len() {
                    if let Some(color) = parse_vec3(&args[i]) {
                        opts.background_color = Some(color);
                    } else {
                        eprintln!("Invalid background color '{}', ignoring", args[i]);
                    }
                }
            }
            "--slice" => {
                opts.slice_enabled = Some(true);
            }
            "--no-slice" => {
                opts.slice_enabled = Some(false);
            }
            "--slice-axis" => {
                i += 1;
                if i < args.len() {
                    opts.slice_axis = args[i].parse::<u32>().ok();
                }
            }
            "--slice-pos" => {
                i += 1;
                if i < args.len() {
                    opts.slice_position = args[i].parse::<f32>().ok();
                }
            }
            "--slice-pos-vector" => {
                i += 1;
                if i < args.len() {
                    opts.slice_position_vector = args[i].parse::<f32>().ok();
                }
            }
            "--slice-invert" => {
                opts.slice_invert = Some(true);
            }
            "--no-slice-invert" => {
                opts.slice_invert = Some(false);
            }
            "--slice-use-vector" => {
                opts.slice_use_vector = Some(true);
            }
            "--slice-use-axis" => {
                opts.slice_use_vector = Some(false);
            }
            "--slice-normal" => {
                i += 1;
                if i < args.len() {
                    if let Some(normal) = parse_vec3(&args[i]) {
                        opts.slice_normal = Some(normal);
                    } else {
                        eprintln!("Invalid slice normal '{}', ignoring", args[i]);
                    }
                }
            }
            "--lod" => {
                opts.lod_enabled = Some(true);
            }
            "--no-lod" => {
                opts.lod_enabled = Some(false);
            }
            "--lod-min-size" => {
                i += 1;
                if i < args.len() {
                    opts.lod_min_screen_size = args[i].parse::<f32>().ok();
                }
            }
            "--inertia" => {
                opts.inertia_enabled = Some(true);
            }
            "--no-inertia" => {
                opts.inertia_enabled = Some(false);
            }
            "--inertia-friction" => {
                i += 1;
                if i < args.len() {
                    opts.inertia_friction = args[i].parse::<f32>().ok();
                }
            }
            "--inertia-cutoff" => {
                i += 1;
                if i < args.len() {
                    opts.inertia_cutoff = args[i].parse::<f32>().ok();
                }
            }
            _ => {
                // Assume it's a path if it doesn't start with -
                if !arg.starts_with('-') {
                    opts.path = Some(arg.clone());
                } else {
                    eprintln!("Unknown option '{}', ignoring", arg);
                }
            }
        }
        i += 1;
    }

    opts
}
