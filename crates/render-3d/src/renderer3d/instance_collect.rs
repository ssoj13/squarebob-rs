//! Instance collection: builds per-cube `CubeInstance` data + ID mapping.
//!
//! Extracted from `lib.rs` (Stage B.1 of TODO4 roadmap). Pure mechanical
//! move — no behaviour change. Methods remain on `Renderer3D` via re-opened
//! impl block.

use glam::{Mat4, Vec3};
use log::debug;

use dirstat_core::DirEntry;
use pt_mats::{MaterialClass, MaterializeMode};
use render_shared::{hash_transform, name_hash, HoverMode, Render3DOptions};
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
                let pos = Vec3::new(x + w / 2.0, -(y + h / 2.0), -base_height / 2.0);
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

            // Determine base color based on color_mode
            use render_shared::{
                color_for_age, color_for_depth, color_for_extension, color_for_hash,
                color_for_size, ColorMode, FolderColorMode,
            };
            let mut base_color = match opts.color_mode {
                ColorMode::FileType => {
                    // Color by file extension
                    color_for_extension(&node.ext)
                }
                ColorMode::FileAge => {
                    // Color by modification time (normalized to 0-1 over 1 year)
                    let age_norm = if let Some(mtime) = node.modified_time {
                        let now = std::time::SystemTime::now()
                            .duration_since(std::time::UNIX_EPOCH)
                            .map(|d| d.as_secs())
                            .unwrap_or(0);
                        let age_secs = now.saturating_sub(mtime);
                        let year_secs = 365 * 24 * 60 * 60;
                        (age_secs as f32 / year_secs as f32).clamp(0.0, 1.0)
                    } else {
                        // Fallback to hash if no timestamp
                        (name_hash(&node.path.to_string_lossy()) % 1000) as f32 / 1000.0
                    };
                    color_for_age(age_norm)
                }
                ColorMode::FileSize => {
                    // Color by file size (log scale normalized)
                    let size_norm = if node.size > 0 {
                        ((node.size as f64).log10() / 12.0).clamp(0.0, 1.0) as f32
                    // 0-1TB range
                    } else {
                        0.0
                    };
                    color_for_size(size_norm)
                }
                ColorMode::Treemap => {
                    // Original treemap-based coloring
                    let color = if node.is_dir && !node.children.is_empty() {
                        treemap::compute_avg_color(node, dir_hash)
                    } else {
                        treemap::dir_tinted_color(&node.ext, dir_hash)
                    };
                    [
                        color[0] as f32 / 255.0,
                        color[1] as f32 / 255.0,
                        color[2] as f32 / 255.0,
                        1.0,
                    ]
                }
                ColorMode::Depth => {
                    // Color by depth (rainbow gradient)
                    color_for_depth(depth, 10) // max_depth = 10 levels for full rainbow
                }
            };

            // Folder tint: directories get a folder color, files are tinted by parent folder color
            let folder_tint = opts.folder_tint.clamp(0.0, 1.0);
            if folder_tint > 0.0 || node.is_dir {
                let folder_depth = if node.is_dir {
                    depth
                } else {
                    depth.saturating_sub(1)
                };
                let parent_name_hash = node
                    .path
                    .parent()
                    .and_then(|p| p.file_name())
                    .map(|n| name_hash(&n.to_string_lossy()))
                    .unwrap_or(dir_hash);
                let parent_path_hash = node
                    .path
                    .parent()
                    .map(|p| name_hash(&p.to_string_lossy()))
                    .unwrap_or(dir_hash);
                let dir_name_hash = name_hash(&node.name);
                let dir_path_hash = name_hash(&node.path.to_string_lossy());
                let mut folder_color = match opts.folder_color_mode {
                    FolderColorMode::Depth => color_for_depth(folder_depth, 10),
                    FolderColorMode::NameHash => color_for_hash(if node.is_dir {
                        dir_name_hash
                    } else {
                        parent_name_hash
                    }),
                    FolderColorMode::PathHash => color_for_hash(if node.is_dir {
                        dir_path_hash
                    } else {
                        parent_path_hash
                    }),
                };
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
                    .classify_or_get(&node.path, node.size, opts, false)
            } else {
                self.material_library.material_id(MaterialClass::Default)
            };
            // color_f is the pure color_mode result (per-instance tint).
            let color_f = base_color;

            // Treemap XY -> 3D XY (wall facing camera), depth (height) along -Z
            let pos = Vec3::new(x + w / 2.0, -(y + h / 2.0), -base_height / 2.0);
            let transform = hash_transform(
                &node.name,
                pos,
                world_center,
                opts.hash_effect,
                opts.hash_effect_strength,
                opts.animation_time,
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
