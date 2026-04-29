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

mod state;
mod toolbar;
mod status_bar;
mod settings;
mod tree_panel;
mod ext_panel;
mod treemap_view;
mod dock;
pub mod filters;
mod shell;
pub mod helpers;
pub mod presets;
// render_callback module removed - using egui native texture display

use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::Ordering;

use eframe::egui;
use egui_dock::DockArea;
use log::info;

use crate::cache;
use crate::events::{
    downcast, NavigateIntoEvent, NavigateUpEvent, ZoomResetEvent,
    SelectPathEvent, SettingsChangedEvent, LayoutDirtyEvent, RenderTick3DEvent,
};
use crate::exclusions;
use crate::renderer::{
    self, HashTransformEffect,
    RenderBackend, RenderMode,
};
use render_core::gpu::GpuContext;
use treemap::GpuRenderer2D;
use render_3d::Renderer3D;
use crate::scanner::{self, ScanMsg};
use dirstat_core::DirEntry;
#[cfg(windows)]
use crate::scanner_ntfs;
use treemap::{self, LayoutStyle};

pub use state::{App, ScannerMode};
pub use dock::{DockTab, DockTabs};
// Render3DResources removed - using egui native texture display
use state::{PersistState, SavedOpts, ScanProgress};
use helpers::{fmt_size, compute_ext_stats, compute_size_range, find_node_by_path};
use filters::{
    collect_matching_paths, filter_by_extension, filter_by_mask, filter_excluded, filter_tree,
    merge_tree_by_size_range,
};

impl App {
    pub fn new(
        cc: &eframe::CreationContext<'_>,
        cli: crate::CliOptions,
    ) -> Self {
        let default_path = if cfg!(windows) { "C:\\".to_string() } else { "/".to_string() };
        let mut app = Self { scan_path: default_path, ..Default::default() };

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

        // CLI overrides for mode/backend
        if let Some(mode) = cli.mode { app.render_mode = mode; }
        if let Some(backend) = cli.backend { app.render_backend = backend; }

        // CLI render settings
        if let Some(pt) = cli.path_tracing { app.render_3d_opts.path_tracing = pt; }
        if let Some(wf) = cli.wavefront { app.render_3d_opts.pt_wavefront = wf; }
        if let Some(t) = cli.pt_wavefront_tile_size { app.render_3d_opts.pt_wavefront_tile_size = t; }
        if let Some(b) = cli.pt_max_bounces { app.render_3d_opts.pt_max_bounces = b; }
        if let Some(s) = cli.pt_max_samples { app.render_3d_opts.pt_max_samples = s; }
        if let Some(g) = cli.pt_gpu_bvh { app.render_3d_opts.pt_gpu_bvh = g; }
        if let Some(pg) = cli.pt_path_guiding { app.render_3d_opts.pt_path_guiding = pg; }
        if let Some(di) = cli.pt_restir_di { app.render_3d_opts.pt_restir_di = di; }
        if let Some(gi) = cli.pt_restir_gi { app.render_3d_opts.pt_restir_gi = gi; }
        if let Some(ad) = cli.pt_adaptive_sampling { app.render_3d_opts.pt_adaptive_sampling = ad; }
        if let Some(e) = cli.env_map_enabled { app.render_3d_opts.env_map_enabled = e; }
        if let Some(w) = cli.wireframe { app.render_3d_opts.show_wireframe = w; }
        if let Some(a) = cli.animate { app.render_3d_opts.animate = a; }
        if let Some(mode) = cli.height_mode { app.render_3d_opts.height_mode = mode; }
        if let Some(sq) = cli.height_squared {
            app.render_3d_opts.height_power_enabled = sq;
            if sq { app.render_3d_opts.height_power = 2.0; }
        }
        if let Some(scale) = cli.height_scale { app.render_3d_opts.height_scale = scale; }
        if let Some(mode) = cli.color_mode { app.render_3d_opts.color_mode = mode; }
        if let Some(effect) = cli.hash_effect { app.render_3d_opts.hash_effect = effect; }
        if let Some(strength) = cli.hash_effect_strength { app.render_3d_opts.hash_effect_strength = strength; }
        if let Some(time) = cli.animation_time { app.render_3d_opts.animation_time = time; }
        if let Some(speed) = cli.animation_speed { app.render_3d_opts.animation_speed = speed; }
        if let Some(mode) = cli.hover_mode { app.render_3d_opts.hover_mode = mode; }
        if let Some(width) = cli.hover_outline_width { app.render_3d_opts.hover_outline_width = width; }
        if let Some(alpha) = cli.hover_outline_alpha { app.render_3d_opts.hover_outline_alpha = alpha; }
        if let Some(roughness) = cli.roughness { app.render_3d_opts.roughness = roughness; }
        if let Some(metalness) = cli.metalness { app.render_3d_opts.metalness = metalness; }
        if let Some(ior) = cli.specular_ior { app.render_3d_opts.specular_ior = ior; }
        if let Some(alpha) = cli.xray_alpha { app.render_3d_opts.xray_alpha = alpha; }
        if let Some(flat) = cli.flat_shading { app.render_3d_opts.flat_shading = flat; }
        if let Some(double_sided) = cli.double_sided { app.render_3d_opts.double_sided = double_sided; }
        if let Some(mode) = cli.materialize_mode { app.render_3d_opts.materialize_mode = mode; }
        if let Some(allow) = cli.mat_allow_lights { app.render_3d_opts.mat_allow_lights = allow; }
        if let Some(prob) = cli.mat_light_prob { app.render_3d_opts.mat_light_prob = prob; }
        if let Some(allow) = cli.mat_allow_glass { app.render_3d_opts.mat_allow_glass = allow; }
        if let Some(prob) = cli.mat_glass_prob { app.render_3d_opts.mat_glass_prob = prob; }
        if let Some(intensity) = cli.env_map_intensity { app.render_3d_opts.env_map_intensity = intensity; }
        if let Some(rotation) = cli.env_map_rotation { app.render_3d_opts.env_map_rotation = rotation.to_radians(); }
        if let Some(visible) = cli.env_map_visible { app.render_3d_opts.env_map_visible = visible; }
        if let Some(path) = cli.env_map_path { app.render_3d_opts.env_map_path = Some(PathBuf::from(path)); }
        if let Some(anim) = cli.env_animate { app.render_3d_opts.env_animate = anim; }
        if let Some(speed) = cli.env_speed { app.render_3d_opts.env_speed = speed; }
        if let Some(color) = cli.background_color { app.render_3d_opts.background_color = color; }
        if let Some(samples) = cli.pt_samples_per_update { app.render_3d_opts.pt_samples_per_update = samples; }
        if let Some(depth) = cli.pt_max_transmission_depth { app.render_3d_opts.pt_max_transmission_depth = depth; }
        if let Some(enabled) = cli.pt_dof_enabled { app.render_3d_opts.pt_dof_enabled = enabled; }
        if let Some(aperture) = cli.pt_aperture { app.render_3d_opts.pt_aperture = aperture; }
        if let Some(distance) = cli.pt_focus_distance { app.render_3d_opts.pt_focus_distance = distance; }
        if let Some(enabled) = cli.pt_env_importance_sampling { app.render_3d_opts.pt_env_importance_sampling = enabled; }
        if let Some(fps) = cli.pt_target_fps { app.render_3d_opts.pt_target_fps = fps; }
        if let Some(enabled) = cli.pt_auto_spp { app.render_3d_opts.pt_auto_spp = enabled; }
        if let Some(enabled) = cli.pt_camera_snap { app.render_3d_opts.pt_camera_snap = enabled; }
        if let Some(mode) = cli.pt_spectral_mode { app.render_3d_opts.pt_spectral_mode = mode; }
        if let Some(samples) = cli.pt_spectral_samples { app.render_3d_opts.pt_spectral_samples = samples; }
        if let Some(enabled) = cli.pt_spectral_dispersion { app.render_3d_opts.pt_spectral_dispersion = enabled; }
        if let Some(enabled) = cli.pt_bvh_refit { app.render_3d_opts.pt_bvh_refit = enabled; }
        if let Some(enabled) = cli.pt_russian_roulette { app.render_3d_opts.pt_russian_roulette = enabled; }
        if let Some(enabled) = cli.pt_restir_temporal { app.render_3d_opts.pt_restir_temporal = enabled; }
        if let Some(enabled) = cli.pt_restir_spatial { app.render_3d_opts.pt_restir_spatial = enabled; }
        if let Some(mmax) = cli.pt_restir_m_max { app.render_3d_opts.pt_restir_m_max = mmax; }
        if let Some(res) = cli.pt_svo_resolution { app.render_3d_opts.pt_svo_resolution = res; }
        if let Some(enabled) = cli.slice_enabled { app.render_3d_opts.slice_enabled = enabled; }
        if let Some(axis) = cli.slice_axis { app.render_3d_opts.slice_axis = axis; }
        if let Some(pos) = cli.slice_position { app.render_3d_opts.slice_position = pos; }
        if let Some(pos) = cli.slice_position_vector { app.render_3d_opts.slice_position_vector = pos; }
        if let Some(invert) = cli.slice_invert { app.render_3d_opts.slice_invert = invert; }
        if let Some(use_vector) = cli.slice_use_vector { app.render_3d_opts.slice_use_vector = use_vector; }
        if let Some(normal) = cli.slice_normal { app.render_3d_opts.slice_normal = normal; }
        if let Some(enabled) = cli.lod_enabled { app.render_3d_opts.lod_enabled = enabled; }
        if let Some(size) = cli.lod_min_screen_size { app.render_3d_opts.lod_min_screen_size = size; }
        if let Some(enabled) = cli.inertia_enabled { app.render_3d_opts.inertia_enabled = enabled; }
        if let Some(friction) = cli.inertia_friction { app.render_3d_opts.inertia_friction = friction; }
        if let Some(cutoff) = cli.inertia_cutoff { app.render_3d_opts.inertia_cutoff = cutoff; }

        // Post-CLI migrations for derived settings
        if matches!(app.render_3d_opts.height_mode, renderer::CubeHeightMode::DepthSquared) {
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
            render_state.device.on_uncaptured_error(Arc::new(move |err| {
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
            let has_polygon_mode = render_state.device.features().contains(wgpu::Features::POLYGON_MODE_LINE);
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

    // ── Scanning ──

    /// Effective backend name for UI (matches spawn choice in [`start_scan`])
    fn scan_engine_label_for_mode(mode: ScannerMode, path: &std::path::Path) -> String {
        match mode {
            ScannerMode::Standard => "jwalk".to_string(),
            ScannerMode::Ntfs => {
                #[cfg(windows)]
                {
                    if scanner_ntfs::is_ntfs_available(path) {
                        "NTFS MFT".to_string()
                    } else {
                        "jwalk".to_string()
                    }
                }
                #[cfg(not(windows))]
                {
                    "jwalk".to_string()
                }
            }
        }
    }

    pub(super) fn start_scan(&mut self) {
        let path = PathBuf::from(&self.scan_path);
        if !path.exists() {
            self.progress.error = Some(format!("Path not found: {}", self.scan_path));
            return;
        }
        self.exclusions = exclusions::load(&self.scan_path);

        // History
        let p = self.scan_path.clone();
        self.path_history.retain(|x| x != &p);
        self.path_history.insert(0, p);
        self.path_history.truncate(20);

        // Try cache
        if let Some(cached) = cache::load_cache(&self.scan_path) {
            info!("Loaded cache for: {}", self.scan_path);
            self.cache_age = Some(cache::age_secs_from_cached(&cached));
            self.ext_stats = compute_ext_stats(&cached.tree);
            let (smin, smax) = compute_size_range(&cached.tree);
            self.scan_min_size = smin;
            self.scan_max_size = smax;
            self.filter_min = smin;
            self.filter_max = smax;
            self.expanded.insert(cached.tree.path.clone());
            self.tree = Some(cached.tree);
            self.filtered_tree = None;
            self.zoom_path = None;
            self.rebuild_display_tree();
            self.needs_layout = true;

            // Start screenshot timer after cache load
            if self.screenshot_delay.is_some() && self.screenshot_start_time.is_none() {
                self.screenshot_start_time = Some(std::time::Instant::now());
            }
        } else {
            self.tree = None;
            self.filtered_tree = None;
            self.cache_age = None;
        }

        let (tx, rx) = crossbeam_channel::unbounded();
        self.scan_rx = Some(rx);
        self.treemap_tex = None;
        self.hovered = None;
        self.selected_path = None;
        if self.tree.is_none() { self.expanded.clear(); }
        self.ctx_menu_path = None;
        let scan_engine_label = Self::scan_engine_label_for_mode(self.scanner_mode, &path);
        self.progress = ScanProgress {
            scanning: true,
            start_time: Some(std::time::Instant::now()),
            scan_engine_label: Some(scan_engine_label),
            ..Default::default()
        };

        let cancel = match self.scanner_mode {
            ScannerMode::Ntfs => {
                #[cfg(windows)]
                {
                    if scanner_ntfs::is_ntfs_available(&path) {
                        scanner_ntfs::scan_ntfs_bg(path, tx)
                    } else {
                        scanner::scan_bg(path, tx)
                    }
                }
                #[cfg(not(windows))]
                { scanner::scan_bg(path, tx) }
            }
            ScannerMode::Standard => scanner::scan_bg(path, tx),
        };
        self.scan_cancel = Some(cancel);
    }

    pub(super) fn stop_scan(&mut self) {
        if let Some(cancel) = &self.scan_cancel {
            cancel.store(true, Ordering::Relaxed);
        }
    }

    fn poll_scan(&mut self) {
        let messages: Vec<ScanMsg> = if let Some(rx) = &self.scan_rx {
            rx.try_iter().collect()
        } else {
            return;
        };

        let mut needs_display_rebuild = false;

        for msg in messages {
            match msg {
                ScanMsg::Progress { files, dirs, bytes, errors } => {
                    self.progress.files = files;
                    self.progress.dirs = dirs;
                    self.progress.bytes = bytes;
                    self.progress.errors = errors;
                }
                ScanMsg::Done(tree) => {
                    info!("Scan complete: {} files", tree.file_count);
                    self.progress.scanning = false;
                    self.progress.scan_engine_label = None;
                    self.progress.error = None;
                    if let Some(t) = self.progress.start_time {
                        self.progress.elapsed_secs = t.elapsed().as_secs_f32();
                    }
                    self.ext_stats = compute_ext_stats(&tree);
                    let (smin, smax) = compute_size_range(&tree);
                    self.scan_min_size = smin;
                    self.scan_max_size = smax;
                    self.filter_min = smin;
                    self.filter_max = smax;
                    self.expanded.insert(tree.path.clone());

                    // Serialize cache on main thread (avoids cloning the tree)
                    let scan_path = self.scan_path.clone();
                    match cache::serialize_cache(&scan_path, &tree) {
                        Ok(cache_bytes) => {
                            std::thread::spawn(move || {
                                if let Err(e) = cache::write_cache_bytes(&scan_path, &cache_bytes) {
                                    log::warn!("Failed to write cache: {}", e);
                                }
                            });
                        }
                        Err(e) => {
                            log::warn!("Failed to serialize cache: {}", e);
                        }
                    }

                    self.tree = Some(tree);
                    self.filtered_tree = None;
                    self.zoom_path = None;
                    self.cache_age = None;
                    self.filtered_paths_cache = None;
                    needs_display_rebuild = true;

                    // Start screenshot timer after scan completes
                    if self.screenshot_delay.is_some() && self.screenshot_start_time.is_none() {
                        self.screenshot_start_time = Some(std::time::Instant::now());
                    }
                    self.needs_layout = true;
                }
                #[cfg(windows)]
                ScanMsg::NtfsFallback(msg) => {
                    self.scanner_mode = ScannerMode::Standard;
                    self.progress.scan_engine_label = Some("jwalk (NTFS fallback)".to_string());
                    self.progress.error = Some(format!("NTFS failed ({}), using standard scanner", msg));
                }
                ScanMsg::Error(e) => {
                    self.progress.scanning = false;
                    self.progress.scan_engine_label = None;
                    if let Some(t) = self.progress.start_time {
                        self.progress.elapsed_secs = t.elapsed().as_secs_f32();
                    }
                    self.progress.error = Some(e);
                }
            }
        }

        if needs_display_rebuild {
            self.rebuild_display_tree();
        }
    }

    /// Process queued events
    fn handle_events(&mut self, ctx: &egui::Context) {
        if !self.events.has_pending() {
            return;
        }
        for event in self.events.poll() {
            if let Some(e) = downcast::<NavigateIntoEvent>(&event) {
                self.nav_to(e.0.clone());
            }
            else if downcast::<NavigateUpEvent>(&event).is_some() {
                self.zoom_up();
            }
            else if downcast::<ZoomResetEvent>(&event).is_some() {
                self.zoom_reset();
            }
            else if let Some(e) = downcast::<SelectPathEvent>(&event) {
                self.select(e.0.clone());
            }
            else if downcast::<SettingsChangedEvent>(&event).is_some() {
                self.needs_layout = true;
                ctx.request_repaint();
            }
            else if downcast::<LayoutDirtyEvent>(&event).is_some() {
                self.needs_layout = true;
            }
            else if downcast::<RenderTick3DEvent>(&event).is_some() {
                self.render_tick_3d = true;
            }
        }
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
        let Some((_tree_path, zoom_root_path)) = tree else { return };
        if *target == zoom_root_path { return; }

        let next = self.active_tree().and_then(|t| {
            let root = find_node_by_path(t, &zoom_root_path).unwrap_or(t);
            root.children.iter().find(|c| {
                c.is_dir && (c.path == *target || target.starts_with(&c.path))
            }).map(|c| c.path.clone())
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

        let ext_filter: std::collections::HashSet<String> = self.ext_filter
            .iter()
            .map(|e| e.to_lowercase())
            .collect();

        let has_exclusions = !self.exclusions.is_empty();
        let has_masks = !masks.is_empty();
        let has_ext_filter = !ext_filter.is_empty();

        if !self.show_free_space && !has_exclusions && !has_masks && !has_ext_filter && self.zoom_path.is_none() {
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

    /// Sync dock tabs visibility with settings visibility flag.
    /// Rebuilds dock_state if visibility changed.
    fn sync_dock_tabs_visibility(&mut self) {
        let current_tabs: Vec<dock::DockTab> = self.dock_state
            .iter_all_tabs()
            .map(|(_, tab)| tab.clone())
            .collect();

        let has_settings = current_tabs.contains(&dock::DockTab::Settings);
        let has_ext = current_tabs.contains(&dock::DockTab::Extensions);
        if has_ext || self.show_settings != has_settings {
            self.dock_state = dock::build_dock_state(self.show_settings);
        }
    }

    fn render_treemap(&mut self, ctx: &egui::Context, size: (u32, u32)) {
        let (w, h) = size;
        if w == 0 || h == 0 { return; }

        let t0 = std::time::Instant::now();

        self.viewport.width = w;
        self.viewport.height = h;

        // GPU context should already be initialized from eframe in App::new()
        // Fallback to creating our own if not available
        let needs_gpu = self.render_mode == RenderMode::Mode3D
            || (self.render_mode == RenderMode::Mode2D && self.render_backend == RenderBackend::Gpu);

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
                self.orbit_camera.set_front_view(w as f32, h as f32);
            }
        }

        if self.render_mode == RenderMode::Mode2D && self.render_backend == RenderBackend::Gpu
            && self.renderer_2d_gpu.is_none() {
                if let Some(gpu_ctx) = &self.gpu_context {
                    self.renderer_2d_gpu = Some(GpuRenderer2D::new(gpu_ctx.clone()));
                }
            }

        // TODO: Zero-copy rendering requires using eframe's device for all rendering
        // For now, this is disabled because textures can't be shared between devices
        // if self.render_mode == RenderMode::Mode3D && self.wgpu_render_state.is_some() { ... }

        // Legacy path with CPU readback
        let pixels = match self.render_mode {
            RenderMode::Mode2D => {
                match self.render_backend {
                    RenderBackend::Cpu => {
                        let Some(root) = self.display_root() else { return };
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
                }
            }
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
                    r.render(root, w, h, &self.orbit_camera, &self.render_3d_opts, &self.opts)
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
            self.renderer_3d.as_ref().map(|r| r.pt_samples_per_update()).unwrap_or(0)
        } else {
            0
        };
        self.last_frame_ms = total_ms as f32;
        self.last_fps = if total_ms > 0.0 { (1000.0 / total_ms) as f32 } else { 0.0 };
        self.last_samples_per_frame = samples_per_frame;
        self.last_samples_per_sec = if samples_per_frame > 0 && total_ms > 0.0 {
            samples_per_frame as f32 / (total_ms as f32 / 1000.0)
        } else {
            0.0
        };
    }

    /// Handle automated screenshot capture
    fn handle_screenshot(&mut self, ctx: &egui::Context) {
        if self.screenshot_taken { return; }
        let Some(delay) = self.screenshot_delay else { return; };
        let Some(start) = self.screenshot_start_time else { return; };

        let elapsed = start.elapsed().as_secs_f32();
        if elapsed < delay {
            ctx.request_repaint();
            return;
        }

        // Time to take screenshot
        self.screenshot_taken = true;

        let path = self.screenshot_path.clone()
            .unwrap_or_else(|| {
                let temp = std::env::temp_dir();
                temp.join("dirstat_screenshot.png").to_string_lossy().to_string()
            });

        // Re-render to capture latest state
        let (w, h) = self.last_render_size;
        if w > 0 && h > 0 {
            let pixels = self.capture_viewport(w, h);
            if !pixels.is_empty() {
                info!("Taking screenshot: {}x{} -> {}", w, h, path);
                if let Err(e) = save_png(&path, w, h, pixels) {
                    log::error!("Failed to save screenshot: {}", e);
                } else {
                    info!("Screenshot saved: {}", path);
                }
            }
        } else {
            log::warn!("No render available for screenshot");
        }

        if self.exit_after_screenshot {
            ctx.send_viewport_cmd(egui::ViewportCommand::Close);
        }
    }

    /// Capture viewport pixels for screenshot
    fn capture_viewport(&mut self, w: u32, h: u32) -> Vec<u8> {
        match self.render_mode {
            RenderMode::Mode2D => {
                match self.render_backend {
                    RenderBackend::Cpu => {
                        let root_ptr = match self.display_root() {
                            Some(root) => root as *const _,
                            None => return Vec::new(),
                        };
                        // Safe: root lives in self for duration of this call.
                        let root = unsafe { &*root_ptr };
                        renderer::cpu::render(root, &self.viewport, &self.opts)
                    }
                    RenderBackend::Gpu => {
                        let root_ptr = match self.display_root() {
                            Some(root) => root as *const _,
                            None => return Vec::new(),
                        };
                        let mut renderer = self.renderer_2d_gpu.take();
                        let pixels = if let Some(r) = &mut renderer {
                            // Safe: root lives in self for duration of this call.
                            let root = unsafe { &*root_ptr };
                            r.render(root, &self.viewport, &self.opts)
                        } else {
                            Vec::new()
                        };
                        self.renderer_2d_gpu = renderer;
                        pixels
                    }
                }
            }
            RenderMode::Mode3D => {
                // If we already rendered this frame, just read back.
                if self.last_render_frame_3d == self.frame_count {
                    if let Some(r) = &self.renderer_3d {
                        return r.readback_render_texture();
                    }
                }

                // Otherwise, render once and read back.
                let root_ptr = match self.display_root() {
                    Some(root) => root as *const _,
                    None => return Vec::new(),
                };
                if let Some(r) = &mut self.renderer_3d {
                    // Safe: root lives in self for duration of this call.
                    let root = unsafe { &*root_ptr };
                    r.render_to_view(
                        root, w, h,
                        &self.orbit_camera,
                        &self.render_3d_opts,
                        &self.opts,
                    );
                    self.last_render_frame_3d = self.frame_count;
                    r.readback_render_texture()
                } else {
                    Vec::new()
                }
            }
        }
    }

    fn run_frame(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        let ctx = ui.ctx().clone();
        self.frame_count = self.frame_count.saturating_add(1);
        if self.wgpu_error_flag.swap(false, Ordering::SeqCst) {
            log::warn!("wgpu error flagged; resetting GPU renderers and textures");
            if let Some(r3d) = &mut self.renderer_3d {
                r3d.reset_render_targets();
                r3d.reset_path_tracer();
            }
            if let (Some(render_state), Some(id)) = (&self.wgpu_render_state, self.render_texture_id) {
                let mut renderer = render_state.renderer.write();
                renderer.free_texture(&id);
            }
            self.renderer_3d = None;
            self.renderer_2d_gpu = None;
            self.render_texture_id = None;
            self.needs_layout = true;
            self.last_render_size = (0, 0);
        }

        // Force theme and font size on first frame
        if self.frame_count == 1 {
            ctx.set_visuals(if self.dark_mode {
                egui::Visuals::dark()
            } else {
                egui::Visuals::light()
            });
            self.apply_font_size(&ctx);
        }

        self.poll_scan();
        self.handle_events(&ctx);

        // Mark preset dirty when settings change
        if self.needs_layout || self.needs_render_3d {
            self.preset_dirty = true;
        }

        // Autosave preset if enabled and dirty
        if self.preset_autosave && self.preset_dirty && !self.preset_name.is_empty() {
            let elapsed = self.preset_last_save.elapsed().as_secs_f32();
            if elapsed >= self.autosave_interval_secs {
                self.save_current_preset();
                self.preset_dirty = false;
                self.preset_last_save = std::time::Instant::now();
            }
        }

        if self.progress.scanning {
            ctx.request_repaint();
        }

        // Keyboard shortcuts
        let kb_ok = !ctx.egui_wants_keyboard_input();

        if kb_ok && ctx.input(|i| i.key_pressed(egui::Key::Escape)) {
            if self.zoom_path.is_some() {
                self.events.emit(ZoomResetEvent);
            } else {
                ctx.send_viewport_cmd(egui::ViewportCommand::Close);
            }
        }
        if kb_ok && ctx.input(|i| i.key_pressed(egui::Key::Backspace)) {
            self.events.emit(NavigateUpEvent);
        }
        if kb_ok && ctx.input(|i| i.key_pressed(egui::Key::Delete)) {
            if let Some(sel) = &self.selected_path.clone() {
                shell::shell_trash(sel);
            }
        }
        if kb_ok && ctx.input(|i| i.modifiers.ctrl && i.key_pressed(egui::Key::C)) {
            if let Some(sel) = &self.selected_path {
                ctx.copy_text(sel.to_string_lossy().to_string());
            }
        }
        if kb_ok && ctx.input(|i| i.key_pressed(egui::Key::Enter) && !i.modifiers.alt) {
            if let Some(sel) = &self.selected_path {
                let _ = open::that(sel);
            }
        }
        #[cfg(any(target_os = "windows", target_os = "macos"))]
        if kb_ok && ctx.input(|i| i.key_pressed(egui::Key::Enter) && i.modifiers.alt) {
            if let Some(sel) = &self.selected_path.clone() {
                shell::shell_properties(sel);
            }
        }
        if ctx.input(|i| i.modifiers.ctrl && i.key_pressed(egui::Key::F)) {
            self.show_search = !self.show_search;
        }
        if kb_ok && ctx.input(|i| i.key_pressed(egui::Key::F5))
            && !self.progress.scanning && !self.scan_path.is_empty() {
                self.start_scan();
            }
        if kb_ok && ctx.input(|i| i.key_pressed(egui::Key::N)) {
            self.selected_path = None;
            self.selected_3d_ids.clear();
            self.needs_render_3d = true;
            ctx.request_repaint();
        }
        // Space: toggle animation in 3D mode
        if kb_ok && ctx.input(|i| i.key_pressed(egui::Key::Space))
            && self.render_mode == RenderMode::Mode3D {
                self.render_3d_opts.animate = !self.render_3d_opts.animate;
                self.needs_layout = true;
            }

        // Window title
        let title = if let Some(tree) = &self.tree {
            format!("dirstat-rs  -  {} [{}]", self.scan_path, fmt_size(tree.size))
        } else if self.progress.scanning {
            let engine = self
                .progress
                .scan_engine_label
                .as_deref()
                .unwrap_or("…");
            format!("dirstat-rs  -  [{}] {}…", engine, self.scan_path)
        } else {
            "dirstat-rs".to_string()
        };
        ctx.send_viewport_cmd(egui::ViewportCommand::Title(title));

        // Animation time for 3D
        if self.render_mode == RenderMode::Mode3D {
            let menu_open = self.ctx_menu_path.is_some();
            let dt = ctx.input(|i| i.stable_dt);
            let auto_spp_freeze = self.render_3d_opts.path_tracing
                && (self.render_3d_opts.pt_auto_spp || self.render_3d_opts.pt_camera_snap);
            let allow_anim_tick = if auto_spp_freeze {
                let interval = 1.0 / self.render_3d_opts.pt_target_fps.max(1.0);
                if self.pt_auto_spp_tick.elapsed().as_secs_f32() >= interval {
                    self.pt_auto_spp_tick = std::time::Instant::now();
                    self.events.emit(RenderTick3DEvent);
                    true
                } else {
                    false
                }
            } else {
                true
            };

            // Accumulate animation time only when animate enabled and menu closed
            if self.render_3d_opts.animate && !menu_open && allow_anim_tick {
                self.render_3d_opts.animation_time += dt * self.render_3d_opts.animation_speed;
            }

            // Update camera animation
            if allow_anim_tick && self.orbit_camera.update_animation(dt) {
                self.needs_render_3d = true;
                ctx.request_repaint();
            }

            // Update camera inertia
            if self.render_3d_opts.inertia_enabled
                && allow_anim_tick && self.orbit_camera.update_inertia(
                    dt,
                    self.render_3d_opts.inertia_friction,
                    self.render_3d_opts.inertia_cutoff,
                ) {
                    self.needs_render_3d = true;
                    ctx.request_repaint();
                }

            // Request repaint for 3D mode when animation or path tracing is active
            // Note: PT mode needs repaint but NOT needs_layout (that triggers geometry rebuild)
            if self.render_3d_opts.path_tracing {
                if auto_spp_freeze {
                    let interval = 1.0 / self.render_3d_opts.pt_target_fps.max(1.0);
                    ctx.request_repaint_after(std::time::Duration::from_secs_f32(interval));
                } else {
                    ctx.request_repaint(); // PT accumulates samples continuously
                }
            } else if (!menu_open
                    && self.render_3d_opts.hash_effect != HashTransformEffect::None
                    && self.render_3d_opts.animate
                    && allow_anim_tick)
                || self.orbit_camera.is_animating()
            {
                self.needs_layout = true;
                ctx.request_repaint();
            }
        }

        // UI panels (order matters for egui layout)
        self.ui_toolbar(ui);
        self.ui_search_bar(ui);
        self.ui_status_bar(ui);

        // Sync dock visibility before rendering
        self.sync_dock_tabs_visibility();

        egui::CentralPanel::default().show_inside(ui, |ui| {
            let dock_style = egui_dock::Style::from_egui(ui.global_style().as_ref());
            let mut dock_state = std::mem::replace(&mut self.dock_state, dock::default_dock_state());
            {
                let mut tabs = DockTabs { app: self };
                DockArea::new(&mut dock_state)
                    .style(dock_style)
                    .show_inside(ui, &mut tabs);
            }
            self.dock_state = dock_state;
        });

        // Screenshot handling
        self.handle_screenshot(&ctx);
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

/// Save RGBA pixels as PNG using image crate
fn save_png(path: &str, w: u32, h: u32, pixels: Vec<u8>) -> Result<(), Box<dyn std::error::Error>> {
    // Create parent directory if needed
    if let Some(parent) = std::path::Path::new(path).parent() {
        if !parent.as_os_str().is_empty() && !parent.exists() {
            std::fs::create_dir_all(parent)?;
        }
    }
    let img = image::RgbaImage::from_raw(w, h, pixels)
        .ok_or("Invalid image dimensions")?;
    img.save(path)?;
    Ok(())
}
