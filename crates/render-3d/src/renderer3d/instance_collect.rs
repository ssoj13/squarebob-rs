//! Instance collection: builds per-cube `CubeInstance` data + ID mapping.
//!
//! Extracted from `lib.rs` (Stage B.1 of TODO4 roadmap). Pure mechanical
//! move — no behaviour change. Methods remain on `Renderer3D` via re-opened
//! impl block.

use glam::{Mat4, Vec3};
use log::debug;

use dirstat_core::DirEntry;
use pt_mats::{
    hierarchical_path_value, sample_palette, MaterialClass, MaterialDistribution, MaterializeMode,
    Palette,
};
use render_shared::{hash_transform, name_hash, ColorMode, FolderColorMode, HoverMode, RampParams, Render3DOptions};
use treemap::{self, TreeMapOptions};

use crate::geometry::CubeInstance;
use crate::Renderer3D;

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
        self.cached_instances_rebuild_count =
            self.cached_instances_rebuild_count.wrapping_add(1);
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
        self.collect_recursive(
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
    pub(crate) fn collect_recursive(
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
        let [x, y, w, h] = node.rect.get();
        if w < 1.0 || h < 1.0 || node.size == 0 {
            return;
        }

        let too_small = w < treemap::MIN_RECT_SIZE || h < treemap::MIN_RECT_SIZE;
        let camera_lod_collapse = if let Some((cam_eye, screen_h, fov, min_size)) = lod_ctx {
            if node.is_dir && !node.children.is_empty() && !too_small && depth > 0 {
                let base_height = Self::compute_cube_height(node, depth, opts);
                // Cube anchored at the treemap plane (z=0) on its BACK face — height
// grows forward toward the camera so each bar's top is visible
// instead of all cubes sharing a flat "wall" at z=0.
let pos = Vec3::new(x + w / 2.0, -(y + h / 2.0), base_height / 2.0);
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
            let mode_default_palette = default_palette_for_color_mode(opts.color_mode);
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
                    FolderColorMode::PathHash => {
                        hierarchical_path_value(folder_path)
                    }
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

            let allow_dirs = opts.mat_include_dirs || !node.is_dir;
            // Material classification is cached, so this is O(1) on warm cache.
            // Returns the final library index (legacy class slots or palette
            // sample). Shader handles albedo blending via
            // `mat_global.materialize_mix`, so we do NOT lerp on the CPU
            // anymore — instances stay stable across slider changes, the
            // slider itself just rewrites the small UBO.
            let material_id = if opts.materialize_mode != MaterializeMode::None && allow_dirs {
                self.mat_cache
                    .classify_or_get(&node.path, node.size, depth, opts, false)
            } else {
                self.material_library.material_id(MaterialClass::Default)
            };
            // color_f is the pure color_mode result (per-instance tint).
            let color_f = base_color;

            // Treemap XY -> 3D XY (wall facing camera), depth (height) along -Z
            // Cube anchored at the treemap plane (z=0) on its BACK face — height
// grows forward toward the camera so each bar's top is visible
// instead of all cubes sharing a flat "wall" at z=0.
let pos = Vec3::new(x + w / 2.0, -(y + h / 2.0), base_height / 2.0);
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
            let my_hash = treemap::path_hash(&node.name, dir_hash);
            for child in &node.children {
                self.collect_recursive(
                    child,
                    depth + 1,
                    my_hash,
                    opts,
                    _treemap_opts,
                    world_center,
                    need_picking,
                    lod_ctx,
                    out,
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
        MaterialDistribution::Quantized => {
            let n = ramp.quant_levels.max(1) as f32;
            (tt * n).floor() / (n - 1.0).max(1.0)
        }
        MaterialDistribution::Gradient => tt * tt * (3.0 - 2.0 * tt),
        MaterialDistribution::Bands => {
            let n = ramp.band_count.max(1) as f32;
            (tt * n).floor() / (n - 1.0).max(1.0)
        }
        MaterialDistribution::Spatial => {
            // Mix in a deterministic path-based wobble — closest cheap
            // proxy for "spatial coherence" that survives the path-keyed
            // cube cache.
            let n = hierarchical_path_value(path);
            (tt * 0.3 + n * 0.7).clamp(0.0, 1.0)
        }
    };
    let palette = ramp.palette.unwrap_or(default_palette);
    let rgb = sample_palette(palette, tt);
    [rgb[0], rgb[1], rgb[2], 1.0]
}

/// Recursively scan the directory tree to find the deepest depth and the
/// largest file size. Used by `collect_cubes` to set the normalisation
/// denominators for `Depth` / `Size` material sources before
/// classification kicks in. Cheap pre-walk: visits every node exactly
/// once with no allocations.
fn scan_scene_bounds(node: &DirEntry, depth: u32) -> (u32, u64) {
    let mut max_depth = depth;
    let mut max_size = node.size;
    for child in &node.children {
        let (cd, cs) = scan_scene_bounds(child, depth + 1);
        if cd > max_depth {
            max_depth = cd;
        }
        if cs > max_size {
            max_size = cs;
        }
    }
    (max_depth, max_size)
}
