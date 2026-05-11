//! Frame loop, event pump, and dock-visibility sync.
//!
//! Extracted from `mod.rs` for review/merge sanity. No behaviour change.

use std::sync::atomic::Ordering;

use eframe::egui;
use egui_dock::DockArea;

use crate::events::{
    downcast, LayoutDirtyEvent, NavigateIntoEvent, NavigateUpEvent, RenderTick3DEvent,
    SelectPathEvent, SettingsChangedEvent, ZoomResetEvent,
};
use crate::renderer::{HashTransformEffect, RenderMode};

use super::dock::{self, DockTabs};
use super::helpers::fmt_size;
use super::shell;
use super::App;

impl App {
    /// Process queued events
    fn handle_events(&mut self, ctx: &egui::Context) {
        if !self.events.has_pending() {
            return;
        }
        for event in self.events.poll() {
            if let Some(e) = downcast::<NavigateIntoEvent>(&event) {
                self.nav_to(e.0.clone());
            } else if downcast::<NavigateUpEvent>(&event).is_some() {
                self.zoom_up();
            } else if downcast::<ZoomResetEvent>(&event).is_some() {
                self.zoom_reset();
            } else if let Some(e) = downcast::<SelectPathEvent>(&event) {
                self.select(e.0.clone());
            } else if downcast::<SettingsChangedEvent>(&event).is_some() {
                self.needs_layout = true;
                ctx.request_repaint();
            } else if downcast::<LayoutDirtyEvent>(&event).is_some() {
                self.needs_layout = true;
            } else if downcast::<RenderTick3DEvent>(&event).is_some() {
                self.render_tick_3d = true;
            }
        }
    }

    /// Sync dock tabs visibility with settings visibility flag.
    /// Rebuilds dock_state if visibility changed.
    fn sync_dock_tabs_visibility(&mut self) {
        let current_tabs: Vec<dock::DockTab> = self
            .dock_state
            .iter_all_tabs()
            .map(|(_, tab)| tab.clone())
            .collect();

        let has_settings = current_tabs.contains(&dock::DockTab::Settings);
        let has_ext = current_tabs.contains(&dock::DockTab::Extensions);
        if has_ext || self.show_settings != has_settings {
            self.dock_state = dock::build_dock_state(self.show_settings);
        }
    }

    pub(super) fn run_frame(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        let ctx = ui.ctx().clone();
        self.frame_count = self.frame_count.saturating_add(1);
        if self.wgpu_error_flag.swap(false, Ordering::SeqCst) {
            log::warn!("wgpu error flagged; resetting GPU renderers and textures");
            if let Some(r3d) = &mut self.renderer_3d {
                r3d.reset_render_targets();
                r3d.reset_path_tracer();
            }
            if let (Some(render_state), Some(id)) =
                (&self.wgpu_render_state, self.render_texture_id)
            {
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
                shell::shell_open(sel);
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
        if kb_ok
            && ctx.input(|i| i.key_pressed(egui::Key::F5))
            && !self.progress.scanning
            && !self.scan_path.is_empty()
        {
            self.start_scan();
        }
        if kb_ok && ctx.input(|i| i.key_pressed(egui::Key::N)) {
            self.selected_path = None;
            self.selected_3d_ids.clear();
            self.needs_render_3d = true;
            ctx.request_repaint();
        }
        // Space: toggle animation in 3D mode
        if kb_ok
            && ctx.input(|i| i.key_pressed(egui::Key::Space))
            && self.render_mode == RenderMode::Mode3D
        {
            self.render_3d_opts.animate = !self.render_3d_opts.animate;
            self.needs_layout = true;
        }

        // Window title
        let title = if let Some(tree) = &self.tree {
            format!(
                "dirstat-rs  -  {} [{}]",
                self.scan_path,
                fmt_size(tree.size)
            )
        } else if self.progress.scanning {
            let engine = self.progress.scan_engine_label.as_deref().unwrap_or("…");
            format!("dirstat-rs  -  [{}] {}…", engine, self.scan_path)
        } else {
            "dirstat-rs".to_string()
        };
        ctx.send_viewport_cmd(egui::ViewportCommand::Title(title));

        // Animation time for 3D
        if self.render_mode == RenderMode::Mode3D {
            let menu_open = self.ctx_menu_path.is_some();
            // Clamp dt to ~33ms so a slow / paused frame doesn't make
            // animation timelines lurch when redraws resume.
            let dt = ctx.input(|i| i.stable_dt).min(0.033);
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

            // Accumulate object-side animation time (cube transforms,
            // hash effects). Gated by `animate`.
            if self.render_3d_opts.animate && !menu_open && allow_anim_tick {
                self.render_3d_opts.animation_time += dt * self.render_3d_opts.animation_speed;
            }
            // Env-side timeline advances independently so the sky /
            // daylight cycle keeps rolling even when object animation
            // is paused. Gated by `env_animate` + its own speed.
            if self.render_3d_opts.env_animate && !menu_open && allow_anim_tick {
                self.render_3d_opts.env_time += dt * self.render_3d_opts.env_speed;
            }

            // Update camera animation
            if allow_anim_tick && self.orbit_camera.update_animation(dt) {
                self.needs_render_3d = true;
                ctx.request_repaint();
            }

            // Update camera inertia
            if self.render_3d_opts.inertia_enabled
                && allow_anim_tick
                && self.orbit_camera.update_inertia(
                    dt,
                    self.render_3d_opts.inertia_friction,
                    self.render_3d_opts.inertia_cutoff,
                )
            {
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
            let mut dock_state =
                std::mem::replace(&mut self.dock_state, dock::default_dock_state());
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
