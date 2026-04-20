//! Top toolbar: path input, scan controls, view toggles, 2D/3D + dark/light buttons.

use eframe::egui;

use crate::events::{NavigateUpEvent, ZoomResetEvent};
use crate::renderer::{RenderBackend, RenderMode};
use super::App;
use super::helpers::rfd_pick_folder;

impl App {
    /// Render top toolbar panel
    pub(super) fn ui_toolbar(&mut self, ctx: &egui::Context) {
        egui::TopBottomPanel::top("toolbar").show(ctx, |ui| {
            ui.horizontal(|ui| {
                // === PATH SECTION ===
                let mut start_scan = false;

                // Path history dropdown
                egui::ComboBox::from_id_salt("path_combo")
                    .width(40.0)
                    .selected_text("\u{25be}")
                    .show_ui(ui, |ui| {
                        let mut sorted = self.path_history.clone();
                        sorted.sort_by_key(|a| a.to_lowercase());
                        for p in &sorted {
                            if ui.selectable_label(p == &self.scan_path, p).clicked() {
                                self.scan_path = p.clone();
                                start_scan = true;
                            }
                        }
                    });

                // Path text field
                let resp = ui.add(
                    egui::TextEdit::singleline(&mut self.scan_path).desired_width(280.0),
                );
                if resp.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter)) {
                    start_scan = true;
                }

                // Browse button
                if ui.button("...").on_hover_text("Browse for folder").clicked() {
                    if let Some(path) = rfd_pick_folder() {
                        self.scan_path = path;
                        start_scan = true;
                    }
                }

                ui.separator();

                // === SCAN CONTROLS ===
                if self.progress.scanning {
                    if ui.button("\u{23f9} Stop").clicked() {
                        self.stop_scan();
                    }
                } else if ui.button("\u{25b6} Scan").clicked() {
                    start_scan = true;
                }

                if start_scan {
                    self.start_scan();
                }

                // Zoom controls (only when zoomed)
                if self.zoom_path.is_some() {
                    if ui.button("\u{2b06} Up").on_hover_text("Zoom out (Backspace)").clicked() {
                        self.events.emit(NavigateUpEvent);
                    }
                    if ui.button("\u{23cf} Reset").on_hover_text("Reset zoom (Escape)").clicked() {
                        self.events.emit(ZoomResetEvent);
                    }
                }

                ui.separator();

                // === VIEW OPTIONS ===
                ui.checkbox(&mut self.show_settings, "Settings");

                // === RIGHT-ALIGNED: 2D/3D + CPU/GPU + Dark/Light toggle buttons ===
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    // Dark/Light toggle
                    let theme_label = if self.dark_mode { "\u{263d}" } else { "\u{2600}" };
                    let theme_hover = if self.dark_mode { "Switch to Light" } else { "Switch to Dark" };
                    if ui.button(theme_label).on_hover_text(theme_hover).clicked() {
                        self.dark_mode = !self.dark_mode;
                        ctx.set_visuals(if self.dark_mode {
                            egui::Visuals::dark()
                        } else {
                            egui::Visuals::light()
                        });
                    }

                    // 2D/3D toggle
                    let mode_label = match self.render_mode {
                        RenderMode::Mode2D => "2D",
                        RenderMode::Mode3D => "3D",
                    };
                    let mode_hover = match self.render_mode {
                        RenderMode::Mode2D => "Switch to 3D",
                        RenderMode::Mode3D => "Switch to 2D",
                    };
                    if ui.button(mode_label).on_hover_text(mode_hover).clicked() {
                        let old_mode = self.render_mode;
                        self.render_mode = match old_mode {
                            RenderMode::Mode2D => RenderMode::Mode3D,
                            RenderMode::Mode3D => RenderMode::Mode2D,
                        };
                        self.on_render_mode_changed(old_mode);
                    }

                    // CPU/GPU toggle (only in 2D mode)
                    if self.render_mode == RenderMode::Mode2D {
                        let backend_label = match self.render_backend {
                            RenderBackend::Cpu => "CPU",
                            RenderBackend::Gpu => "GPU",
                        };
                        let backend_hover = match self.render_backend {
                            RenderBackend::Cpu => "Switch to GPU rendering",
                            RenderBackend::Gpu => "Switch to CPU rendering",
                        };
                        if ui.button(backend_label).on_hover_text(backend_hover).clicked() {
                            self.render_backend = match self.render_backend {
                                RenderBackend::Cpu => RenderBackend::Gpu,
                                RenderBackend::Gpu => RenderBackend::Cpu,
                            };
                            self.treemap_tex = None;
                            self.needs_layout = true;
                            if let Some(r) = &mut self.renderer_2d_gpu {
                                r.reset_render_targets();
                            }
                        }
                    }
                });
            });
        });
    }

    /// Render search bar (Ctrl+F)
    pub(super) fn ui_search_bar(&mut self, ctx: &egui::Context) {
        if !self.show_search {
            return;
        }
        egui::TopBottomPanel::top("search_bar").show(ctx, |ui| {
            ui.horizontal(|ui| {
                ui.label("Search:");
                let resp = ui.add(
                    egui::TextEdit::singleline(&mut self.search_text)
                        .desired_width(300.0)
                        .hint_text("filename filter..."),
                );
                if resp.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Escape)) {
                    self.show_search = false;
                    self.search_text.clear();
                }
                if ui.small_button("x").clicked() {
                    self.show_search = false;
                    self.search_text.clear();
                }
            });
        });
    }

    /// Handle render mode change side-effects
    pub(super) fn on_render_mode_changed(&mut self, old_mode: RenderMode) {
        if self.render_mode == old_mode {
            return;
        }
        self.treemap_tex = None;
        if self.render_mode == RenderMode::Mode2D {
            // Drop any 3D zero-copy texture id to avoid stale display in 2D.
            self.render_texture_id = None;
        }
        self.needs_layout = true;

        if let Some(renderer) = &mut self.renderer_3d {
            renderer.reset_render_targets();
        }
        if let Some(renderer) = &mut self.renderer_2d_gpu {
            renderer.reset_render_targets();
        }

        // Set front view matching 2D layout when switching to 3D
        if self.render_mode == RenderMode::Mode3D {
            let (w, h) = self.last_render_size;
            if w > 0 && h > 0 {
                self.orbit_camera.set_front_view(w as f32, h as f32);
            } else {
                self.orbit_camera = crate::renderer::OrbitCamera::default();
            }
        }
    }
}
