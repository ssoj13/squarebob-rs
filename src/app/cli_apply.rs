//! CLI → Render3DOptions applicator. Centralises the mirroring that
//! used to live inline in App::new — single source of truth for which
//! CLI flags map to which Render3DOptions field.

use std::path::PathBuf;

use crate::CliOptions;
use render_shared::Render3DOptions;

/// Apply every CLI override to `opts`. Each `Some(_)` flag overwrites the
/// corresponding field; `None` leaves the existing value (from `PersistState`
/// or default) intact.
///
/// Note: mode/backend overrides target `App` fields (not `Render3DOptions`)
/// and intentionally remain inline in `App::new` — they don't fit this
/// applicator's contract.
pub(super) fn apply_cli_overrides(opts: &mut Render3DOptions, cli: &CliOptions) {
    // CLI render settings
    if let Some(pt) = cli.path_tracing {
        opts.path_tracing = pt;
    }
    if let Some(wf) = cli.wavefront {
        opts.pt_wavefront = wf;
    }
    if let Some(t) = cli.pt_wavefront_tile_size {
        opts.pt_wavefront_tile_size = t;
    }
    if let Some(b) = cli.pt_max_bounces {
        opts.pt_max_bounces = b;
    }
    if let Some(s) = cli.pt_samples {
        opts.pt_samples = s;
    }
    if let Some(g) = cli.pt_gpu_bvh {
        opts.pt_gpu_bvh = g;
    }
    if let Some(pg) = cli.pt_path_guiding {
        opts.pt_path_guiding = pg;
    }
    if let Some(ref m) = cli.pt_oidn_mode {
        opts.pt_oidn_mode = match m.to_ascii_lowercase().as_str() {
            "off" => render_shared::OidnModeOption::Off,
            "color" => render_shared::OidnModeOption::Color,
            "color_albedo" | "color+albedo" => render_shared::OidnModeOption::ColorAlbedo,
            // default for any other input → full quality
            _ => render_shared::OidnModeOption::ColorAlbedoNormal,
        };
    }
    if let Some(ref q) = cli.pt_oidn_quality {
        opts.pt_oidn_quality = match q.to_ascii_lowercase().as_str() {
            "large" | "high" => render_shared::OidnQualityOption::Large,
            "small" | "fast" => render_shared::OidnQualityOption::Small,
            _ => render_shared::OidnQualityOption::Base,
        };
    }
    if let Some(a) = cli.pt_oidn_auto {
        opts.pt_oidn_auto = a;
    }
    if let Some(di) = cli.pt_restir_di {
        opts.pt_restir_di = di;
    }
    if let Some(gi) = cli.pt_restir_gi {
        opts.pt_restir_gi = gi;
    }
    if let Some(ad) = cli.pt_adaptive_sampling {
        opts.pt_adaptive_sampling = ad;
    }
    if let Some(e) = cli.env_map_enabled {
        opts.env_map_enabled = e;
    }
    if let Some(w) = cli.wireframe {
        opts.show_wireframe = w;
    }
    if let Some(a) = cli.animate {
        opts.animate = a;
    }
    if let Some(mode) = cli.height_mode {
        opts.height_mode = mode;
    }
    if let Some(sq) = cli.height_squared {
        // CLI legacy flag → sets exponent = 2 on the active mode's curve.
        let idx = opts.height_mode as usize;
        let curve = opts.height_curves.get_mut(idx);
        curve.exponent = if sq { 2.0 } else { 1.0 };
    }
    if let Some(scale) = cli.height_scale {
        let idx = opts.height_mode as usize;
        opts.height_curves.get_mut(idx).scale = scale;
    }
    if let Some(mode) = cli.color_mode {
        opts.color_mode = mode;
    }
    if let Some(effect) = cli.hash_effect {
        opts.hash_effect = effect;
    }
    if let Some(strength) = cli.hash_effect_strength {
        let idx = opts.hash_effect as usize;
        opts.effects.hash_per_variant.get_mut(idx).strength = strength;
    }
    if let Some(time) = cli.animation_time {
        opts.animation_time = time;
    }
    if let Some(speed) = cli.animation_speed {
        opts.animation_speed = speed;
    }
    if let Some(mode) = cli.hover_mode {
        opts.hover_mode = mode;
    }
    if let Some(width) = cli.hover_outline_width {
        opts.hover_outline_width = width;
    }
    if let Some(alpha) = cli.hover_outline_alpha {
        opts.hover_outline_alpha = alpha;
    }
    if let Some(roughness) = cli.roughness {
        opts.roughness = roughness;
    }
    if let Some(metalness) = cli.metalness {
        opts.metalness = metalness;
    }
    if let Some(ior) = cli.specular_ior {
        opts.specular_ior = ior;
    }
    if let Some(alpha) = cli.xray_alpha {
        opts.xray_alpha = alpha;
    }
    if let Some(flat) = cli.flat_shading {
        opts.flat_shading = flat;
    }
    if let Some(double_sided) = cli.double_sided {
        opts.double_sided = double_sided;
    }
    if let Some(mode) = cli.materialize_mode {
        opts.materialize_mode = mode;
    }
    if let Some(allow) = cli.mat_allow_lights {
        opts.mat_allow_lights = allow;
    }
    if let Some(prob) = cli.mat_light_prob {
        opts.mat_light_prob = prob;
    }
    if let Some(allow) = cli.mat_allow_glass {
        opts.mat_allow_glass = allow;
    }
    if let Some(prob) = cli.mat_glass_prob {
        opts.mat_glass_prob = prob;
    }
    if let Some(intensity) = cli.env_map_intensity {
        opts.env_map_intensity = intensity;
    }
    if let Some(rotation) = cli.env_map_rotation {
        opts.env_map_rotation = rotation.to_radians();
    }
    if let Some(visible) = cli.env_map_visible {
        opts.env_map_visible = visible;
    }
    if let Some(path) = cli.env_map_path.as_ref() {
        opts.env_map_path = Some(PathBuf::from(path));
    }
    if let Some(anim) = cli.env_animate {
        opts.env_animate = anim;
    }
    if let Some(speed) = cli.env_speed {
        opts.env_speed = speed;
    }
    if let Some(color) = cli.background_color {
        opts.background_color = color;
    }
    if let Some(samples) = cli.pt_samples_per_update {
        opts.pt_samples_per_update = samples;
    }
    if let Some(depth) = cli.pt_max_transmission_depth {
        opts.pt_max_transmission_depth = depth;
    }
    if let Some(enabled) = cli.pt_dof_enabled {
        opts.pt_dof_enabled = enabled;
    }
    if let Some(aperture) = cli.pt_aperture {
        opts.pt_aperture = aperture;
    }
    if let Some(distance) = cli.pt_focus_distance {
        opts.pt_focus_distance = distance;
    }
    if let Some(enabled) = cli.pt_env_importance_sampling {
        opts.pt_env_importance_sampling = enabled;
    }
    if let Some(fps) = cli.pt_target_fps {
        opts.pt_target_fps = fps;
    }
    if let Some(enabled) = cli.pt_auto_spp {
        opts.pt_auto_spp = enabled;
    }
    if let Some(enabled) = cli.pt_camera_snap {
        opts.pt_camera_snap = enabled;
    }
    if let Some(mode) = cli.pt_spectral_mode {
        opts.pt_spectral_mode = mode;
    }
    if let Some(samples) = cli.pt_spectral_samples {
        opts.pt_spectral_samples = samples;
    }
    if let Some(enabled) = cli.pt_spectral_dispersion {
        opts.pt_spectral_dispersion = enabled;
    }
    if let Some(enabled) = cli.pt_bvh_refit {
        opts.pt_bvh_refit = enabled;
    }
    if let Some(enabled) = cli.pt_russian_roulette {
        opts.pt_russian_roulette = enabled;
    }
    if let Some(enabled) = cli.pt_restir_temporal {
        opts.pt_restir_temporal = enabled;
    }
    if let Some(enabled) = cli.pt_restir_spatial {
        opts.pt_restir_spatial = enabled;
    }
    if let Some(mmax) = cli.pt_restir_m_max {
        opts.pt_restir_m_max = mmax;
    }
    if let Some(res) = cli.pt_svo_resolution {
        opts.pt_svo_resolution = res;
    }
    if let Some(enabled) = cli.slice_enabled {
        opts.slice_enabled = enabled;
    }
    if let Some(axis) = cli.slice_axis {
        opts.slice_axis = axis;
    }
    if let Some(pos) = cli.slice_position {
        opts.slice_position = pos;
    }
    if let Some(pos) = cli.slice_position_vector {
        opts.slice_position_vector = pos;
    }
    if let Some(invert) = cli.slice_invert {
        opts.slice_invert = invert;
    }
    if let Some(use_vector) = cli.slice_use_vector {
        opts.slice_use_vector = use_vector;
    }
    if let Some(normal) = cli.slice_normal {
        opts.slice_normal = normal;
    }
    if let Some(enabled) = cli.lod_enabled {
        opts.lod_enabled = enabled;
    }
    if let Some(size) = cli.lod_min_screen_size {
        opts.lod_min_screen_size = size;
    }
    if let Some(enabled) = cli.inertia_enabled {
        opts.inertia_enabled = enabled;
    }
    if let Some(friction) = cli.inertia_friction {
        opts.inertia_friction = friction;
    }
    if let Some(cutoff) = cli.inertia_cutoff {
        opts.inertia_cutoff = cutoff;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::CliOptions;
    use pt_mats::MaterializeMode;
    use render_shared::{
        ColorMode, CubeHeightMode, HashTransformEffect, HoverMode, Render3DOptions, SpectralMode,
    };

    /// Build a CliOptions where every Option<_> is Some(non-default-ish)
    /// so we can assert each lands in the expected Render3DOptions field.
    fn populated_cli() -> CliOptions {
        CliOptions {
            // Non-applicator fields — irrelevant for this test, leave default.
            path: None,
            mode: None,
            backend: None,
            help: false,
            verbosity: 0,
            log_file: None,
            log_pt: false,
            log_wf: false,
            log_pg: false,
            log_modules: None,
            screenshot_delay: None,
            screenshot_path: None,
            exit_after_screenshot: false,
            test_args: None,

            // Applicator inputs.
            path_tracing: Some(true),
            wavefront: Some(true),
            height_mode: Some(CubeHeightMode::Depth),
            height_squared: Some(true),
            height_scale: Some(7.5),
            color_mode: Some(ColorMode::FileType),
            hash_effect: Some(HashTransformEffect::Wave),
            hash_effect_strength: Some(0.42),
            animation_time: Some(11.0),
            animation_speed: Some(2.5),
            hover_mode: Some(HoverMode::Outline),
            hover_outline_width: Some(3.5),
            hover_outline_alpha: Some(0.9),
            roughness: Some(0.33),
            metalness: Some(0.66),
            specular_ior: Some(1.7),
            xray_alpha: Some(0.25),
            flat_shading: Some(true),
            double_sided: Some(true),
            materialize_mode: Some(MaterializeMode::ByExtension),
            mat_allow_lights: Some(true),
            mat_light_prob: Some(0.77),
            mat_allow_glass: Some(true),
            mat_glass_prob: Some(0.55),
            env_map_intensity: Some(2.0),
            env_map_rotation: Some(180.0),
            env_map_enabled: Some(true),
            env_map_visible: Some(true),
            env_map_path: Some("/tmp/env.hdr".to_string()),
            env_animate: Some(true),
            env_speed: Some(0.5),
            background_color: Some([0.1, 0.2, 0.3]),
            wireframe: Some(true),
            animate: Some(true),
            pt_max_bounces: Some(13),
            pt_samples: Some(123),
            pt_samples_per_update: Some(4),
            pt_max_transmission_depth: Some(8),
            pt_dof_enabled: Some(true),
            pt_aperture: Some(0.05),
            pt_focus_distance: Some(2.5),
            pt_env_importance_sampling: Some(true),
            pt_target_fps: Some(45.0),
            pt_auto_spp: Some(true),
            pt_camera_snap: Some(true),
            pt_spectral_mode: Some(SpectralMode::Hero),
            pt_spectral_samples: Some(7),
            pt_spectral_dispersion: Some(true),
            pt_gpu_bvh: Some(true),
            pt_bvh_refit: Some(true),
            pt_russian_roulette: Some(true),
            pt_adaptive_sampling: Some(true),
            pt_wavefront_tile_size: Some(64),
            pt_restir_di: Some(true),
            pt_restir_gi: Some(true),
            pt_restir_temporal: Some(true),
            pt_restir_spatial: Some(true),
            pt_restir_m_max: Some(20),
            pt_path_guiding: Some(true),
            pt_svo_resolution: Some(128),
            pt_oidn_mode: Some("color_albedo_normal".to_string()),
            pt_oidn_quality: Some("high".to_string()),
            pt_oidn_auto: Some(true),
            slice_enabled: Some(true),
            slice_axis: Some(2),
            slice_position: Some(0.5),
            slice_position_vector: Some(0.75),
            slice_invert: Some(true),
            slice_use_vector: Some(true),
            slice_normal: Some([0.0, 1.0, 0.0]),
            lod_enabled: Some(true),
            lod_min_screen_size: Some(4.0),
            inertia_enabled: Some(true),
            inertia_friction: Some(3.0),
            inertia_cutoff: Some(0.01),
        }
    }

    #[test]
    fn every_some_flag_lands_in_expected_field() {
        let cli = populated_cli();
        let mut opts = Render3DOptions::default();
        apply_cli_overrides(&mut opts, &cli);

        assert!(opts.path_tracing);
        assert!(opts.pt_wavefront);
        assert_eq!(opts.pt_wavefront_tile_size, 64);
        assert_eq!(opts.pt_max_bounces, 13);
        assert_eq!(opts.pt_samples, 123);
        assert!(opts.pt_gpu_bvh);
        assert!(opts.pt_path_guiding);
        assert_eq!(opts.pt_oidn_mode, render_shared::OidnModeOption::ColorAlbedoNormal);
        assert_eq!(opts.pt_oidn_quality, render_shared::OidnQualityOption::Large);
        assert!(opts.pt_oidn_auto);
        assert!(opts.pt_restir_di);
        assert!(opts.pt_restir_gi);
        assert!(opts.pt_adaptive_sampling);
        assert!(opts.env_map_enabled);
        assert!(opts.show_wireframe);
        assert!(opts.animate);
        assert!(matches!(opts.height_mode, CubeHeightMode::Depth));
        let depth_curve = opts.height_curves.get(CubeHeightMode::Depth as usize);
        assert_eq!(depth_curve.exponent, 2.0);
        assert_eq!(depth_curve.scale, 7.5);
        assert!(matches!(opts.color_mode, ColorMode::FileType));
        assert!(matches!(opts.hash_effect, HashTransformEffect::Wave));
        assert_eq!(opts.active_hash_strength(), 0.42);
        assert_eq!(opts.animation_time, 11.0);
        assert_eq!(opts.animation_speed, 2.5);
        assert!(matches!(opts.hover_mode, HoverMode::Outline));
        assert_eq!(opts.hover_outline_width, 3.5);
        assert_eq!(opts.hover_outline_alpha, 0.9);
        assert_eq!(opts.roughness, 0.33);
        assert_eq!(opts.metalness, 0.66);
        assert_eq!(opts.specular_ior, 1.7);
        assert_eq!(opts.xray_alpha, 0.25);
        assert!(opts.flat_shading);
        assert!(opts.double_sided);
        assert!(matches!(
            opts.materialize_mode,
            MaterializeMode::ByExtension
        ));
        assert!(opts.mat_allow_lights);
        assert_eq!(opts.mat_light_prob, 0.77);
        assert!(opts.mat_allow_glass);
        assert_eq!(opts.mat_glass_prob, 0.55);
        assert_eq!(opts.env_map_intensity, 2.0);
        // env_map_rotation is converted from degrees → radians.
        assert!((opts.env_map_rotation - 180.0_f32.to_radians()).abs() < 1e-6);
        assert!(opts.env_map_visible);
        assert_eq!(opts.env_map_path, Some(PathBuf::from("/tmp/env.hdr")));
        assert!(opts.env_animate);
        assert_eq!(opts.env_speed, 0.5);
        assert_eq!(opts.background_color, [0.1, 0.2, 0.3]);
        assert_eq!(opts.pt_samples_per_update, 4);
        assert_eq!(opts.pt_max_transmission_depth, 8);
        assert!(opts.pt_dof_enabled);
        assert_eq!(opts.pt_aperture, 0.05);
        assert_eq!(opts.pt_focus_distance, 2.5);
        assert!(opts.pt_env_importance_sampling);
        assert_eq!(opts.pt_target_fps, 45.0);
        assert!(opts.pt_auto_spp);
        assert!(opts.pt_camera_snap);
        assert!(matches!(opts.pt_spectral_mode, SpectralMode::Hero));
        assert_eq!(opts.pt_spectral_samples, 7);
        assert!(opts.pt_spectral_dispersion);
        assert!(opts.pt_bvh_refit);
        assert!(opts.pt_russian_roulette);
        assert!(opts.pt_restir_temporal);
        assert!(opts.pt_restir_spatial);
        assert_eq!(opts.pt_restir_m_max, 20);
        assert_eq!(opts.pt_svo_resolution, 128);
        assert!(opts.slice_enabled);
        assert_eq!(opts.slice_axis, 2);
        assert_eq!(opts.slice_position, 0.5);
        assert_eq!(opts.slice_position_vector, 0.75);
        assert!(opts.slice_invert);
        assert!(opts.slice_use_vector);
        assert_eq!(opts.slice_normal, [0.0, 1.0, 0.0]);
        assert!(opts.lod_enabled);
        assert_eq!(opts.lod_min_screen_size, 4.0);
        assert!(opts.inertia_enabled);
        assert_eq!(opts.inertia_friction, 3.0);
        assert_eq!(opts.inertia_cutoff, 0.01);
    }

    #[test]
    fn none_flags_leave_existing_values_intact() {
        let cli = CliOptions::default();
        let mut opts = Render3DOptions::default();
        let baseline = opts.clone();
        apply_cli_overrides(&mut opts, &cli);

        // Spot-check a few fields from each major group.
        assert_eq!(opts.path_tracing, baseline.path_tracing);
        assert_eq!(opts.pt_max_bounces, baseline.pt_max_bounces);
        assert_eq!(
            opts.height_curves.get(opts.height_mode as usize).scale,
            baseline
                .height_curves
                .get(baseline.height_mode as usize)
                .scale
        );
        assert_eq!(opts.background_color, baseline.background_color);
        assert_eq!(opts.env_map_path, baseline.env_map_path);
        assert_eq!(opts.slice_axis, baseline.slice_axis);
        assert_eq!(opts.inertia_friction, baseline.inertia_friction);
    }
}
