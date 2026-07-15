//! Instance collection: builds per-cube `CubeInstance` data + ID mapping.
//!
//! Extracted from `lib.rs` (Stage B.1 of TODO4 roadmap). Pure mechanical
//! move — no behaviour change. Methods remain on `Renderer3D` via re-opened
//! impl block.

use glam::{Mat4, Vec3};
use log::debug;

use pt_mats::{
    MaterialDistribution, MaterializeMode, Palette, hierarchical_path_value, sample_palette,
};
use render_shared::{
    ColorMode, FolderColorMode, HoverMode, RampParams, Render3DOptions, hash_transform, name_hash,
};
use squarebob_core::DirEntry;
use treemap::{self, TreeMapOptions};

use crate::Renderer3D;
use crate::geometry::CubeInstance;

impl Renderer3D {
    // ========================================================================
    // Cube collection (builds instances + ID mapping)
    // ========================================================================

    #[allow(clippy::too_many_arguments)]
    pub(crate) fn collect_cubes(
        &mut self,
        root: &DirEntry,
        opts: &Render3DOptions,
        treemap_opts: &TreeMapOptions,
        world_center: Vec3,
        camera_eye: Vec3,
        screen_height: f32,
        fov: f32,
    ) -> Vec<CubeInstance> {
        let start = std::time::Instant::now();
        // Stage A.1 verification instrumentation: every entry into this
        // function is a full instance rebuild. Used to confirm that
        // shader-side uniforms (e.g. materialize_mix) do not invalidate
        // the cache. Read via `instance_rebuild_count`.
        self.cached_instances_rebuild_count = self.cached_instances_rebuild_count.wrapping_add(1);
        debug!(
            "collect_cubes rebuild #{}",
            self.cached_instances_rebuild_count
        );
        let need_picking = opts.hover_mode != HoverMode::None || opts.path_tracing;
        if need_picking {
            self.picking.reset_frame();
        }
        // Drop mat-class cache once per frame if mat-settings changed.
        self.mat_cache.ensure(opts);
        // Zero per-frame override hit counters before any classify
        // calls so the UI displays only cubes from THIS pass, not
        // accumulated history.
        self.mat_cache.reset_overlay_counts();
        // Pre-walk: compute scene normalisation bounds so `Depth`/`Size`
        // sources produce meaningful values (otherwise both collapse to a
        // single point and any distribute on top is a no-op).
        let (scene_max_depth, scene_max_size) = scan_scene_bounds(root, 0);
        self.mat_cache
            .set_scene_meta(scene_max_depth, scene_max_size);
        let mut instances = Vec::new();
        let lod_ctx = if opts.lod_enabled {
            Some((camera_eye, screen_height, fov, opts.lod_min_screen_size))
        } else {
            None
        };
        self.collect_instances(
            root,
            0,
            0,
            opts,
            treemap_opts,
            world_center,
            need_picking,
            lod_ctx,
            &mut instances,
        );
        let elapsed_ms = start.elapsed().as_secs_f64() * 1000.0;
        log::debug!(
            "collect_cubes: {:.2}ms ({} cubes)",
            elapsed_ms,
            instances.len()
        );
        // Log first instance for debugging
        if let Some(first) = instances.first() {
            let m = &first.model;
            log::trace!(
                "  first cube: model[0]=({:.1},{:.1},{:.1},{:.1}), color=({:.2},{:.2},{:.2},{:.2})",
                m[0][0],
                m[0][1],
                m[0][2],
                m[0][3],
                first.color[0],
                first.color[1],
                first.color[2],
                first.color[3]
            );
            log::trace!(
                "  first cube: model[3]=({:.1},{:.1},{:.1},{:.1}) (translation column)",
                m[3][0],
                m[3][1],
                m[3][2],
                m[3][3]
            );
        }
        instances
    }

    #[allow(clippy::too_many_arguments)]
    pub(crate) fn collect_instances(
        &mut self,
        node: &DirEntry,
        depth: u32,
        dir_hash: u32,
        opts: &Render3DOptions,
        _treemap_opts: &TreeMapOptions,
        world_center: Vec3,
        need_picking: bool,
        lod_ctx: Option<(Vec3, f32, f32, f32)>, // (cam_eye, screen_h, fov, min_size)
        out: &mut Vec<CubeInstance>,
    ) {
        let mut pending = vec![(node, depth, dir_hash)];
        while let Some((node, depth, dir_hash)) = pending.pop() {
            let [x, y, w, h] = node.rect.get();
            if w < 1.0 || h < 1.0 || node.size == 0 {
                continue;
            }

            let too_small = w < treemap::MIN_RECT_SIZE || h < treemap::MIN_RECT_SIZE;
            let camera_lod_collapse = if let Some((cam_eye, screen_h, fov, min_size)) = lod_ctx {
                if node.is_dir && !node.children.is_empty() && !too_small && depth > 0 {
                    let base_height = Self::compute_cube_height(node, depth, opts);
                    // Cube centred on the treemap plane (z=0): extends half forward
                    // toward the camera, half behind. This keeps the camera *outside*
                    // every cube as long as `base_height / 2` stays under the camera
                    // distance — works for typical scenes. Outliers (huge files) can
                    // still poke into the camera; pending a global height clamp.
                    let pos = Vec3::new(x + w / 2.0, -(y + h / 2.0), 0.0);
                    let cube_size = w.max(h).max(base_height);
                    let dist = (pos - cam_eye).length().max(0.01);
                    let proj_size = (cube_size / dist) * screen_h / (2.0 * (fov / 2.0).tan());
                    proj_size < min_size
                } else {
                    false
                }
            } else {
                false
            };

            if !node.is_dir || node.children.is_empty() || too_small || camera_lod_collapse {
                // Leaf or consolidated node -> emit cube
                let base_height = Self::compute_cube_height(node, depth, opts);

                // Palette-driven per-cube tint. Each ColorMode emits a scalar
                // t∈[0,1] from the relevant property (path / ext / size / age
                // / depth); the active ramp's palette + distribution + curve
                // turns that into an RGB tint. Auto-routes palette by source
                // if the user hasn't pinned one.
                let (scene_max_depth, scene_max_size) = self.mat_cache.scene_meta();
                let t = match opts.color_mode {
                    ColorMode::FileType => name_hash(&node.ext) as f32 / u32::MAX as f32,
                    ColorMode::FileAge => {
                        if let Some(mtime) = node.modified_time {
                            let now = std::time::SystemTime::now()
                                .duration_since(std::time::UNIX_EPOCH)
                                .map(|d| d.as_secs())
                                .unwrap_or(0);
                            let age_secs = now.saturating_sub(mtime);
                            let year_secs = 365 * 24 * 60 * 60;
                            (age_secs as f32 / year_secs as f32).clamp(0.0, 1.0)
                        } else {
                            (name_hash(&node.path.to_string_lossy()) % 1000) as f32 / 1000.0
                        }
                    }
                    ColorMode::FileSize => {
                        let max_log = ((scene_max_size as f64).max(1.0).log10()).max(1.0);
                        if node.size > 0 {
                            (((node.size as f64).log10()) / max_log).clamp(0.0, 1.0) as f32
                        } else {
                            0.0
                        }
                    }
                    ColorMode::Treemap => hierarchical_path_value(&node.path),
                    ColorMode::Depth => {
                        (depth as f32 / scene_max_depth.max(1) as f32).clamp(0.0, 1.0)
                    }
                };
                // `mat_palette` (Materials section dropdown) acts as a
                // global override for the per-color-mode default palette.
                // Per-ramp `ramp.palette` (set in each ColorMode's own
                // ramp editor) still wins — this only affects ramps that
                // are on Auto. That makes the "Palette" knob in the
                // Materials section a single visible-everywhere control,
                // not a dead widget.
                let mode_default_palette = opts
                    .mat_palette
                    .unwrap_or_else(|| default_palette_for_color_mode(opts.color_mode));
                let mut base_color = sample_color_ramp(
                    t,
                    opts.color_ramps.get(opts.color_mode as usize),
                    mode_default_palette,
                    &node.path,
                );

                // Folder tint: directories get a folder color, files are tinted by parent folder color
                let folder_tint = opts.folder_tint.clamp(0.0, 1.0);
                if folder_tint > 0.0 || node.is_dir {
                    let folder_depth = if node.is_dir {
                        depth
                    } else {
                        depth.saturating_sub(1)
                    };
                    let parent_path = node.path.parent();
                    let folder_path = if node.is_dir {
                        node.path.as_path()
                    } else {
                        parent_path.unwrap_or(node.path.as_path())
                    };
                    let folder_t = match opts.folder_color_mode {
                        FolderColorMode::Depth => {
                            (folder_depth as f32 / scene_max_depth.max(1) as f32).clamp(0.0, 1.0)
                        }
                        FolderColorMode::NameHash => {
                            let h = folder_path
                                .file_name()
                                .map(|n| name_hash(&n.to_string_lossy()))
                                .unwrap_or(dir_hash);
                            h as f32 / u32::MAX as f32
                        }
                        FolderColorMode::PathHash => hierarchical_path_value(folder_path),
                    };
                    let folder_default = default_palette_for_folder_mode(opts.folder_color_mode);
                    let mut folder_color = sample_color_ramp(
                        folder_t,
                        opts.folder_ramps.get(opts.folder_color_mode as usize),
                        folder_default,
                        folder_path,
                    );
                    // Depth attenuation: deeper folders → darker tint.
                    let depth_factor = (1.0 - folder_depth as f32 * 0.04).clamp(0.35, 1.0);
                    folder_color[0] *= depth_factor;
                    folder_color[1] *= depth_factor;
                    folder_color[2] *= depth_factor;

                    if node.is_dir {
                        base_color = folder_color;
                    } else if folder_tint > 0.0 {
                        let tinted = [
                            base_color[0] * folder_color[0],
                            base_color[1] * folder_color[1],
                            base_color[2] * folder_color[2],
                            1.0,
                        ];
                        base_color[0] = base_color[0] + (tinted[0] - base_color[0]) * folder_tint;
                        base_color[1] = base_color[1] + (tinted[1] - base_color[1]) * folder_tint;
                        base_color[2] = base_color[2] + (tinted[2] - base_color[2]) * folder_tint;
                    }
                }

                // Treemap XY -> 3D XY (wall facing camera), depth (height) along -Z
                // Cube centred on the treemap plane (z=0): extends half forward
                // toward the camera, half behind. This keeps the camera *outside*
                // every cube as long as `base_height / 2` stays under the camera
                // distance — works for typical scenes. Outliers (huge files) can
                // still poke into the camera; pending a global height clamp.
                //
                // Position is computed *before* `classify_or_get` because the
                // `Spatial` / `Perlin` distributions sample world-space cube
                // centres to drive clustering. `hash_transform` jitter applied
                // afterwards stays under the classification cell size, so
                // shifting the call site doesn't perturb the picker.
                let mut pos = Vec3::new(x + w / 2.0, -(y + h / 2.0), 0.0);
                if opts.polar_layout && opts.polar_strength > 0.0 {
                    // Polar layout: treat the X axis relative to world_center
                    // as an angle (full wrap_scale = 360°), Y axis as radius.
                    // `polar_strength` lerps between the original rect layout
                    // and the fully polar interpretation. Effects (Ocean,
                    // Vortex, …) apply on top of the warped position via the
                    // hash_transform call below — they're additive offsets,
                    // not coordinate-system dependent.
                    let local = pos - world_center;
                    let theta = local.x * std::f32::consts::TAU / opts.polar_wrap_scale.max(1.0);
                    let r = local.y;
                    let polar_xy = Vec3::new(r * theta.cos(), r * theta.sin(), 0.0);
                    let blended = local.lerp(polar_xy, opts.polar_strength.clamp(0.0, 1.0));
                    pos = world_center + blended;
                }

                let allow_dirs = opts.mat_include_dirs || !node.is_dir;
                // Material classification is cached, so this is O(1) on warm cache.
                // Position-dependent distributions (`Spatial` / `Perlin`) skip
                // the cache internally. Shader handles albedo blending via
                // `mat_global.materialize_mix` so instances stay stable across
                // slider changes — the slider itself just rewrites the UBO.
                let material_id = if opts.materialize_mode != MaterializeMode::None && allow_dirs {
                    self.mat_cache.classify_or_get(
                        &node.path,
                        node.size,
                        depth,
                        pos.into(),
                        opts,
                        false,
                    )
                } else {
                    // Library slot 0 is the convention default — first
                    // material in `opts.material_library`.
                    0
                };
                // color_f is the pure color_mode result (per-instance tint).
                let color_f = base_color;
                let transform = hash_transform(
                    &node.name,
                    pos,
                    world_center,
                    opts.hash_effect,
                    opts.active_hash_strength(),
                    opts.active_hash_time(),
                );
                let model = Mat4::from_translation(pos + transform.offset)
                    * Mat4::from_quat(transform.rotation)
                    * Mat4::from_scale(Vec3::new(w.max(0.5), h.max(0.5), base_height.max(0.5)));

                let hash = name_hash(&node.name);
                let oid = if need_picking {
                    self.picking.alloc_id(&node.path, node.size, node.is_dir)
                } else {
                    0
                };
                out.push(CubeInstance::new(model, color_f, hash, oid, material_id));
            } else {
                let child_hash = treemap::path_hash(&node.name, dir_hash);
                let child_depth = depth.saturating_add(1);
                pending.extend(
                    node.children
                        .iter()
                        .rev()
                        .map(|child| (child, child_depth, child_hash)),
                );
            }
        }
    }
}

/// Auto-routed palette per `ColorMode`. Mirrors
/// `auto_palette_for_source` from pt-mats but uses the ColorMode enum.
fn default_palette_for_color_mode(m: ColorMode) -> Palette {
    match m {
        ColorMode::FileSize => Palette::Viridis,
        ColorMode::FileAge => Palette::Sunset,
        ColorMode::Depth => Palette::Cubehelix,
        ColorMode::FileType => Palette::Plasma,
        ColorMode::Treemap => Palette::Turbo,
    }
}

/// Auto-routed palette per `FolderColorMode`.
fn default_palette_for_folder_mode(m: FolderColorMode) -> Palette {
    match m {
        FolderColorMode::Depth => Palette::Cubehelix,
        FolderColorMode::NameHash => Palette::Plasma,
        FolderColorMode::PathHash => Palette::Turbo,
    }
}

/// Sample a color ramp: apply curve to `t`, apply distribution, then
/// look up the chosen palette. Position-dependent distributions
/// (Spatial) fall back to Direct for the cached cube path; they'd need
/// per-cube position which the cache key doesn't carry.
fn sample_color_ramp(
    t: f32,
    ramp: RampParams,
    default_palette: Palette,
    path: &std::path::Path,
) -> [f32; 4] {
    let mut tt = ramp.curve.apply(t).clamp(0.0, 1.0);
    tt = match ramp.distribution {
        MaterialDistribution::Direct => tt,
        // Bands the t-value into `band_count` discrete steps so the
        // ramp reads as a stepped gradient instead of continuous.
        // Was `Bands` in the old enum.
        MaterialDistribution::Stratified => {
            let n = ramp.band_count.max(1) as f32;
            (tt * n).floor() / (n - 1.0).max(1.0)
        }
        // Hierarchical path coherence — closest cheap proxy for
        // "spatial clustering" available without 3D position in the
        // color-ramp cache key.
        MaterialDistribution::Spatial => {
            let n = hierarchical_path_value(path);
            (tt * 0.3 + n * 0.7).clamp(0.0, 1.0)
        }
        // 3D value-noise driven shift. The color-ramp path doesn't
        // carry world-space cube position so we substitute a
        // path-name digest as the noise coordinate — gives smoothly
        // varied stripes that ignore the source signal entirely.
        // For full 3D Perlin, use the material-classify side.
        MaterialDistribution::Perlin => {
            let n = hierarchical_path_value(path);
            (tt * 0.5 + n * 0.5).clamp(0.0, 1.0)
        }
        // Smoothstep curve — concentrates mass at the ends.
        MaterialDistribution::Gradient => tt * tt * (3.0 - 2.0 * tt),
    };
    let palette = ramp.palette.unwrap_or(default_palette);
    let rgb = sample_palette(palette, tt);
    [rgb[0], rgb[1], rgb[2], 1.0]
}

/// Scan the directory tree to find the deepest depth and largest file size.
fn scan_scene_bounds(node: &DirEntry, depth: u32) -> (u32, u64) {
    let mut max_depth = depth;
    let mut max_size = node.size;
    let mut pending = vec![(node, depth)];

    while let Some((node, depth)) = pending.pop() {
        max_depth = max_depth.max(depth);
        max_size = max_size.max(node.size);
        let child_depth = depth.saturating_add(1);
        pending.extend(node.children.iter().rev().map(|child| (child, child_depth)));
    }
    (max_depth, max_size)
}
