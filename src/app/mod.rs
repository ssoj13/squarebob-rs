//! dirstat-rs application module.
//!
//! Modular structure:
//! - state.rs: App struct, PersistState, defaults
//! - toolbar.rs: top toolbar + search bar
//! - status_bar.rs: bottom status bar
//! - settings.rs: settings side panel
//! - tree_panel.rs: left file tree panel
//! - ext_panel.rs: right extension stats panel
//! - treemap_view.rs: central treemap + interactions
//! - filters.rs: tree filter/mask/glob logic
//! - shell.rs: OS shell operations
//! - helpers.rs: utility functions

mod cli_apply;
mod dock;
mod ext_panel;
pub mod filters;
pub mod helpers;
pub mod presets;
mod render_loop;
mod scan_orchestration;
mod screenshot;
mod settings;
mod shell;
mod state;
mod status_bar;
mod toolbar;
mod tree_panel;
mod treemap_view;
// render_callback module removed - using egui native texture display

use std::path::PathBuf;
use std::sync::atomic::Ordering;
use std::sync::Arc;

use eframe::egui;

use crate::exclusions;
use crate::renderer::{self, RenderBackend, RenderMode};
use dirstat_core::DirEntry;
use render_3d::Renderer3D;
use render_core::gpu::GpuContext;
use treemap::GpuRenderer2D;
use treemap::{self, LayoutStyle};

pub use dock::DockTab;
pub use state::{App, ScannerMode};
// Render3DResources removed - using egui native texture display
use filters::{
    collect_matching_paths, filter_by_extension, filter_by_mask, filter_excluded, filter_tree,
    merge_tree_by_size_range,
};
use helpers::{compute_ext_stats, find_node_by_path};
use state::{PersistState, SavedOpts};

impl App {
    pub fn new(cc: &eframe::CreationContext<'_>, cli: crate::CliOptions) -> Self {
        let default_path = if cfg!(windows) {
            "C:\\".to_string()
        } else {
            "/".to_string()
        };
        let mut app = Self {
            scan_path: default_path,
            ..Default::default()
        };

        // Restore persisted state
        if let Some(storage) = cc.storage {
            if let Some(json) = storage.get_string("dirstat_state") {
                if let Ok(s) = serde_json::from_str::<PersistState>(&json) {
                    app.scan_path = s.scan_path;
                    app.show_settings = s.show_settings;
                    app.dark_mode = s.dark_mode;
                    app.scanner_mode = s.scanner_mode;
                    app.filter_auto_rebuild = s.filter_auto_rebuild;
                    app.path_history = s.path_history;
                    app.tree_panel_width = s.tree_panel_width;
                    app.settings_panel_width = s.settings_panel_width;
                    app.show_free_space = s.show_free_space;
                    app.render_backend = s.render_backend;
                    app.render_mode = s.render_mode;
                    app.render_3d_opts = s.render_3d_opts;
                    app.dock_state = s.dock_state;
                    app.font_size = s.font_size;
                    app.settings_tab = s.settings_tab;
                    app.ext_filter = s.ext_filter;
                    app.ext_filter_invert = s.ext_filter_invert;
                    app.settings_tint_mix = s.settings_tint_mix;
                    app.preset_autosave = s.preset_autosave;
                    app.autosave_interval_secs = s.autosave_interval_secs;
                    app.filter_merge_outside = s.filter_merge_outside;
                    app.opts.grid = s.opts.grid;
                    app.opts.brightness = s.opts.brightness;
                    app.opts.height = s.opts.height;
                    app.opts.scale_factor = s.opts.scale_factor;
                    app.opts.ambient_light = s.opts.ambient_light;
                    app.opts.light_x = s.opts.light_x;
                    app.opts.light_y = s.opts.light_y;
                    app.opts.style = match s.opts.style.as_str() {
                        "sequoia" => LayoutStyle::SequoiaView,
                        _ => LayoutStyle::KDirStat,
                    };
                }
            }
        }

        if !app.scan_path.is_empty() && !PathBuf::from(&app.scan_path).exists() {
            log::warn!(
                "Persisted scan path does not exist ({}), resetting to default",
                app.scan_path
            );
            app.scan_path = if cfg!(windows) {
                "C:\\".to_string()
            } else {
                "/".to_string()
            };
        }

        // CLI overrides for mode/backend
        if let Some(mode) = cli.mode {
            app.render_mode = mode;
        }
        if let Some(backend) = cli.backend {
            app.render_backend = backend;
        }

        // CLI render settings (mirroring extracted to cli_apply::apply_cli_overrides)
        crate::app::cli_apply::apply_cli_overrides(&mut app.render_3d_opts, &cli);

        // Post-CLI migrations for derived settings
        if matches!(
            app.render_3d_opts.height_mode,
            renderer::CubeHeightMode::DepthSquared
        ) {
            app.render_3d_opts.height_mode = renderer::CubeHeightMode::Depth;
            app.render_3d_opts.height_power_enabled = true;
            app.render_3d_opts.height_power = 2.0;
        }
        if app.render_3d_opts.slice_use_vector
            && app.render_3d_opts.slice_position_vector == 0.0
            && app.render_3d_opts.slice_position != 0.0
        {
            app.render_3d_opts.slice_position_vector = app.render_3d_opts.slice_position;
        }

        // Screenshot mode: disable advanced PT features to reduce GPU risk during capture.
        if cli.screenshot_delay.is_some() {
            app.render_3d_opts.pt_path_guiding = false;
            app.render_3d_opts.pt_restir_di = false;
            app.render_3d_opts.pt_restir_gi = false;
            app.render_3d_opts.pt_adaptive_sampling = false;
        }

        // Screenshot settings
        app.screenshot_delay = cli.screenshot_delay;
        app.screenshot_path = cli.screenshot_path;
        app.exit_after_screenshot = cli.exit_after_screenshot;
        // screenshot_start_time is set when scan completes, not at startup

        // Start scan if path provided
        if let Some(p) = cli.path {
            app.scan_path = p;
            app.start_scan();
        }

        // Apply theme
        cc.egui_ctx.set_visuals(if app.dark_mode {
            egui::Visuals::dark()
        } else {
            egui::Visuals::light()
        });

        // Use eframe's wgpu device for zero-copy rendering
        // Note: eframe's device may not have POLYGON_MODE_LINE feature,
        // so we check before using it for our renderer
        if let Some(render_state) = cc.wgpu_render_state.as_ref() {
            app.wgpu_render_state = Some(render_state.clone());
            let error_flag = app.wgpu_error_flag.clone();
            render_state
                .device
                .on_uncaptured_error(Arc::new(move |err| {
                    log::error!("wgpu uncaptured error: {:?}", err);
                    error_flag.store(true, Ordering::SeqCst);
                }));
            let limits = render_state.device.limits();
            log::info!(
                "wgpu limits: max_storage_buffer_binding_size={}, max_storage_buffers_per_shader_stage={}, max_uniform_buffers_per_shader_stage={}, max_bind_groups={}, max_texture_dimension_2d={}, max_buffer_size={}, min_uniform_buffer_offset_alignment={}, min_storage_buffer_offset_alignment={}",
                limits.max_storage_buffer_binding_size,
                limits.max_storage_buffers_per_shader_stage,
                limits.max_uniform_buffers_per_shader_stage,
                limits.max_bind_groups,
                limits.max_texture_dimension_2d,
                limits.max_buffer_size,
                limits.min_uniform_buffer_offset_alignment,
                limits.min_storage_buffer_offset_alignment
            );
            let has_polygon_mode = render_state
                .device
                .features()
                .contains(wgpu::Features::POLYGON_MODE_LINE);
            if has_polygon_mode {
                // Use eframe's device for zero-copy
                let gpu_ctx = GpuContext::from_eframe(
                    Arc::new(render_state.device.clone()),
                    Arc::new(render_state.queue.clone()),
                );
                app.gpu_context = Some(Arc::new(gpu_ctx));
                log::info!("Using eframe wgpu device for zero-copy rendering");
            } else {
                log::warn!("eframe device lacks POLYGON_MODE_LINE, will create separate device");
            }

            // Note: using egui native texture display, no callback resources needed
        }

        app
    }

    // ── Navigation ──

    pub(super) fn select(&mut self, path: PathBuf) {
        self.selected_path = Some(path.clone());
        self.scroll_to_selected = true;
        let mut p = path;
        while let Some(par) = p.parent() {
            self.expanded.insert(par.to_path_buf());
            p = par.to_path_buf();
        }
    }

    pub(super) fn nav_to(&mut self, path: PathBuf) {
        self.select(path.clone());
        let zoom_dir = if path.is_dir() {
            path
        } else {
            path.parent().map(|p| p.to_path_buf()).unwrap_or(path)
        };
        let is_root = self.active_tree().is_some_and(|t| t.path == zoom_dir);
        self.zoom_path = if is_root { None } else { Some(zoom_dir) };
        self.needs_layout = true;
    }

    pub(super) fn zoom_step_toward(&mut self, target: &PathBuf) {
        if let Some(tree) = self.active_tree() {
            if let Some(node) = find_node_by_path(tree, target) {
                if node.lod_expand.is_some() && !node.is_dir {
                    self.lod_expanded_paths.insert(target.clone());
                    self.rebuild_filtered_tree();
                    self.zoom_path = Some(target.clone());
                    self.needs_layout = true;
                    self.select(target.clone());
                    return;
                }
            }
        }

        let tree = self.active_tree_cloned_path();
        let Some((_tree_path, zoom_root_path)) = tree else {
            return;
        };
        if *target == zoom_root_path {
            return;
        }

        let next = self.active_tree().and_then(|t| {
            let root = find_node_by_path(t, &zoom_root_path).unwrap_or(t);
            root.children
                .iter()
                .find(|c| c.is_dir && (c.path == *target || target.starts_with(&c.path)))
                .map(|c| c.path.clone())
        });

        if let Some(next_path) = next {
            self.zoom_path = Some(next_path.clone());
            self.expanded.insert(next_path);
            self.needs_layout = true;
        }
    }

    pub(super) fn zoom_up(&mut self) {
        let root_path = self.active_tree().map(|t| t.path.clone());
        if let Some(zp) = &self.zoom_path.clone() {
            if let Some(root) = &root_path {
                if zp == root {
                    self.zoom_path = None;
                } else if let Some(parent) = zp.parent() {
                    let parent = parent.to_path_buf();
                    self.zoom_path = if parent == *root { None } else { Some(parent) };
                } else {
                    self.zoom_path = None;
                }
            } else {
                self.zoom_path = None;
            }
            self.needs_layout = true;
        }
    }

    pub(super) fn zoom_reset(&mut self) {
        self.zoom_path = None;
        self.needs_layout = true;
    }

    // ── Tree helpers ──

    pub(super) fn active_tree(&self) -> Option<&DirEntry> {
        self.filtered_tree.as_ref().or(self.tree.as_ref())
    }

    fn active_tree_cloned_path(&self) -> Option<(PathBuf, PathBuf)> {
        let tree = self.active_tree()?;
        let tree_path = tree.path.clone();
        let zoom_root = self.zoom_path.clone().unwrap_or_else(|| tree_path.clone());
        Some((tree_path, zoom_root))
    }

    pub(super) fn rebuild_filtered_tree(&mut self) {
        if let Some(tree) = &self.tree {
            if !self.filter_merge_outside || self.filter_invert {
                self.lod_expanded_paths.clear();
            }
            let filtered = if self.filter_merge_outside && !self.filter_invert {
                merge_tree_by_size_range(
                    tree,
                    self.filter_min,
                    self.filter_max,
                    &self.lod_expanded_paths,
                )
            } else {
                filter_tree(tree, self.filter_min, self.filter_max, self.filter_invert)
            };
            self.ext_stats = compute_ext_stats(&filtered);
            self.filtered_tree = Some(filtered);
            self.rebuild_display_tree();
            self.needs_layout = true;
            self.needs_filter_rebuild = false;
        }
    }

    pub(super) fn display_root(&self) -> Option<&DirEntry> {
        let has_exclusions = !self.exclusions.is_empty();
        if (self.show_free_space || has_exclusions) && self.zoom_path.is_none() {
            if let Some(ref cached) = self.display_tree_cache {
                return Some(cached);
            }
        }

        let base_tree = if has_exclusions || self.show_free_space {
            self.display_tree_cache.as_ref().or(self.active_tree())
        } else {
            self.active_tree()
        };

        let tree = base_tree?;
        if let Some(zp) = &self.zoom_path {
            find_node_by_path(tree, zp).or(Some(tree))
        } else {
            Some(tree)
        }
    }

    pub(super) fn rebuild_display_tree(&mut self) {
        let Some(tree) = self.active_tree() else {
            self.display_tree_cache = None;
            return;
        };

        let masks: Vec<String> = if self.use_file_mask && !self.file_mask_text.is_empty() {
            self.file_mask_text
                .split([' ', ',', ';'])
                .map(|s| s.trim().to_lowercase())
                .filter(|s| !s.is_empty())
                .collect()
        } else {
            Vec::new()
        };

        let ext_filter: std::collections::HashSet<String> =
            self.ext_filter.iter().map(|e| e.to_lowercase()).collect();

        let has_exclusions = !self.exclusions.is_empty();
        let has_masks = !masks.is_empty();
        let has_ext_filter = !ext_filter.is_empty();

        if !self.show_free_space
            && !has_exclusions
            && !has_masks
            && !has_ext_filter
            && self.zoom_path.is_none()
        {
            self.display_tree_cache = None;
            return;
        }

        // Build filtered tree without unnecessary clones: each filter already
        // produces a new tree, so we only clone when no filter runs at all
        // (i.e. only show_free_space is active).
        let mut filtered = if has_exclusions {
            filter_excluded(tree, &self.exclusions, self.show_excluded)
        } else if has_masks {
            filter_by_mask(tree, &masks)
        } else if has_ext_filter {
            filter_by_extension(tree, &ext_filter, self.ext_filter_invert)
        } else {
            tree.clone()
        };

        // Apply remaining filters on top (skip the one already applied above)
        if has_exclusions && has_masks {
            filtered = filter_by_mask(&filtered, &masks);
        }
        if (has_exclusions || has_masks) && has_ext_filter {
            filtered = filter_by_extension(&filtered, &ext_filter, self.ext_filter_invert);
        }
        if !self.show_free_space {
            self.display_tree_cache = Some(filtered);
            return;
        }

        let free = helpers::disk_free_total(&self.scan_path)
            .map(|(f, _)| f)
            .unwrap_or(0);

        if free == 0 {
            self.display_tree_cache = Some(filtered);
            return;
        }

        let mut wrapper = DirEntry::new_dir(filtered.name.clone(), filtered.path.clone());
        wrapper.size = filtered.size;
        wrapper.file_count = filtered.file_count;
        wrapper.dir_count = filtered.dir_count;
        wrapper.children = filtered.children;

        let free_space_path = wrapper.path.join("__FREE_SPACE__");
        let mut free_entry = DirEntry::new_file(
            "Free Space".to_string(),
            free_space_path,
            free,
            "__free__".to_string(),
            None, // No modified time for synthetic entry
        );
        free_entry.file_count = 0;
        wrapper.children.push(free_entry);
        wrapper.size += free;
        wrapper.sort_children_by_size_desc();

        self.display_tree_cache = Some(wrapper);
    }

    pub(super) fn exclude_path(&mut self, path: &std::path::Path) {
        self.exclusions.add(path);
        exclusions::save(&self.exclusions);
        self.rebuild_display_tree();
        self.treemap_tex = None;
        self.needs_layout = true;
    }

    pub(super) fn include_path(&mut self, path: &std::path::Path) {
        self.exclusions.remove(path);
        exclusions::save(&self.exclusions);
        self.treemap_tex = None;
        self.rebuild_display_tree();
        self.needs_layout = true;
    }

    pub(super) fn rebuild_filtered_paths_cache(&mut self) {
        let Some(tree) = &self.tree else {
            self.filtered_paths_cache = None;
            return;
        };

        let search = self.search_text.to_lowercase();
        let masks: Vec<String> = if self.use_file_mask && !self.file_mask_text.is_empty() {
            self.file_mask_text
                .split([' ', ',', ';'])
                .map(|s| s.trim().to_lowercase())
                .filter(|s| !s.is_empty())
                .collect()
        } else {
            Vec::new()
        };

        if search.is_empty() && masks.is_empty() {
            self.filtered_paths_cache = None;
            self.last_search_text = self.search_text.clone();
            self.last_file_mask_text = self.file_mask_text.clone();
            self.last_use_file_mask = self.use_file_mask;
            return;
        }

        let mut matching = std::collections::HashSet::new();
        collect_matching_paths(tree, &search, &masks, &mut matching);

        self.filtered_paths_cache = Some(matching);
        self.last_search_text = self.search_text.clone();
        self.last_file_mask_text = self.file_mask_text.clone();
        self.last_use_file_mask = self.use_file_mask;
    }

    pub(super) fn needs_filter_cache_rebuild(&self) -> bool {
        self.search_text != self.last_search_text
            || self.file_mask_text != self.last_file_mask_text
            || self.use_file_mask != self.last_use_file_mask
    }

    pub(super) fn hit_test_at(&self, lx: f32, ly: f32) -> Option<&DirEntry> {
        let root = self.display_root()?;
        treemap::hit_test(root, lx, ly)
    }

    fn render_treemap(&mut self, ctx: &egui::Context, size: (u32, u32)) {
        let (w, h) = size;
        if w == 0 || h == 0 {
            return;
        }

        let t0 = std::time::Instant::now();

        self.viewport.width = w;
        self.viewport.height = h;

        // GPU context should already be initialized from eframe in App::new()
        // Fallback to creating our own if not available
        let needs_gpu = self.render_mode == RenderMode::Mode3D
            || (self.render_mode == RenderMode::Mode2D
                && self.render_backend == RenderBackend::Gpu);

        if needs_gpu && self.gpu_context.is_none() {
            self.gpu_context = GpuContext::new().map(Arc::new);
        }

        if self.render_mode == RenderMode::Mode3D {
            if self.renderer_3d.is_none() {
                if let Some(gpu_ctx) = &self.gpu_context {
                    let mut r3d = Renderer3D::new(gpu_ctx.clone());
                    if self.render_3d_opts.env_map_enabled {
                        if let Some(ref path) = self.render_3d_opts.env_map_path {
                            if path.exists() {
                                if let Err(e) = r3d.load_env_map(path) {
                                    log::error!("Auto-load env map failed: {e}");
                                }
                            }
                        }
                    }
                    self.renderer_3d = Some(r3d);
                }
            }
            if self.orbit_camera.target == glam::Vec3::ZERO {
                let (scene_w, scene_h) = self
                    .renderer_3d
                    .as_ref()
                    .map(|r| r.current_scene_layout_size())
                    .unwrap_or((w, h));
                self.orbit_camera.set_front_view_for_viewport(
                    scene_w as f32,
                    scene_h as f32,
                    w as f32 / h.max(1) as f32,
                );
            }
        }

        if self.render_mode == RenderMode::Mode2D
            && self.render_backend == RenderBackend::Gpu
            && self.renderer_2d_gpu.is_none()
        {
            if let Some(gpu_ctx) = &self.gpu_context {
                self.renderer_2d_gpu = Some(GpuRenderer2D::new(gpu_ctx.clone()));
            }
        }

        // TODO: Zero-copy rendering requires using eframe's device for all rendering
        // For now, this is disabled because textures can't be shared between devices
        // if self.render_mode == RenderMode::Mode3D && self.wgpu_render_state.is_some() { ... }

        // Legacy path with CPU readback
        let pixels = match self.render_mode {
            RenderMode::Mode2D => match self.render_backend {
                RenderBackend::Cpu => {
                    let Some(root) = self.display_root() else {
                        return;
                    };
                    renderer::cpu::render(root, &self.viewport, &self.opts)
                }
                RenderBackend::Gpu => {
                    let mut renderer_2d = self.renderer_2d_gpu.take();
                    let pixels = if let Some(r) = &mut renderer_2d {
                        let Some(root) = self.display_root() else {
                            self.renderer_2d_gpu = renderer_2d;
                            return;
                        };
                        r.render(root, &self.viewport, &self.opts)
                    } else {
                        let Some(root) = self.display_root() else {
                            self.renderer_2d_gpu = renderer_2d;
                            return;
                        };
                        renderer::cpu::render(root, &self.viewport, &self.opts)
                    };
                    self.renderer_2d_gpu = renderer_2d;
                    pixels
                }
            },
            RenderMode::Mode3D => {
                // TODO: Zero-copy path disabled - needs double-buffering to avoid blocking
                // Currently causes "Not Responding" due to GPU sync issues

                // Legacy CPU readback path (slower but stable)
                let mut renderer_3d = self.renderer_3d.take();
                let pixels = if let Some(r) = &mut renderer_3d {
                    let Some(root) = self.display_root() else {
                        self.renderer_3d = renderer_3d;
                        return;
                    };
                    r.render(
                        root,
                        w,
                        h,
                        &self.orbit_camera,
                        &self.render_3d_opts,
                        &self.opts,
                    )
                } else {
                    let Some(root) = self.display_root() else {
                        self.renderer_3d = renderer_3d;
                        return;
                    };
                    renderer::cpu::render(root, &self.viewport, &self.opts)
                };
                self.renderer_3d = renderer_3d;
                pixels
            }
        };

        let image = egui::ColorImage::from_rgba_unmultiplied([w as usize, h as usize], &pixels);
        self.treemap_tex = Some(ctx.load_texture("treemap", image, egui::TextureOptions::NEAREST));
        self.last_render_size = size;
        self.needs_layout = false;

        let total_ms = t0.elapsed().as_secs_f64() * 1000.0;
        let samples_per_frame = if self.render_3d_opts.path_tracing {
            self.renderer_3d
                .as_ref()
                .map(|r| r.pt_samples_per_update())
                .unwrap_or(0)
        } else {
            0
        };
        self.last_frame_ms = total_ms as f32;
        self.last_fps = if total_ms > 0.0 {
            (1000.0 / total_ms) as f32
        } else {
            0.0
        };
        self.last_samples_per_frame = samples_per_frame;
        self.last_samples_per_sec = if samples_per_frame > 0 && total_ms > 0.0 {
            samples_per_frame as f32 / (total_ms as f32 / 1000.0)
        } else {
            0.0
        };
    }

}

// ── eframe::App impl ──

impl eframe::App for App {
    fn ui(&mut self, ui: &mut egui::Ui, frame: &mut eframe::Frame) {
        self.run_frame(ui, frame);
    }

    fn save(&mut self, storage: &mut dyn eframe::Storage) {
        let state = PersistState {
            scan_path: self.scan_path.clone(),
            show_settings: self.show_settings,
            dark_mode: self.dark_mode,
            scanner_mode: self.scanner_mode,
            filter_auto_rebuild: self.filter_auto_rebuild,
            path_history: self.path_history.clone(),
            opts: SavedOpts {
                style: match self.opts.style {
                    LayoutStyle::KDirStat => "kdirstat".into(),
                    LayoutStyle::SequoiaView => "sequoia".into(),
                },
                grid: self.opts.grid,
                brightness: self.opts.brightness,
                height: self.opts.height,
                scale_factor: self.opts.scale_factor,
                ambient_light: self.opts.ambient_light,
                light_x: self.opts.light_x,
                light_y: self.opts.light_y,
            },
            tree_panel_width: self.tree_panel_width,
            settings_panel_width: self.settings_panel_width,
            show_free_space: self.show_free_space,
            render_backend: self.render_backend,
            render_mode: self.render_mode,
            render_3d_opts: self.render_3d_opts.clone(),
            dock_state: self.dock_state.clone(),
            font_size: self.font_size,
            settings_tab: self.settings_tab,
            ext_filter: self.ext_filter.clone(),
            ext_filter_invert: self.ext_filter_invert,
            settings_tint_mix: self.settings_tint_mix,
            preset_autosave: self.preset_autosave,
            autosave_interval_secs: self.autosave_interval_secs,
            filter_merge_outside: self.filter_merge_outside,
        };
        if let Ok(json) = serde_json::to_string(&state) {
            storage.set_string("dirstat_state", json);
        }
    }
}

