//! Settings panel modules.

mod appearance;
mod denoiser;
mod exclusions;
mod output;
mod ramp_widget;
mod renderer;
mod scanner;
mod view;

pub(super) use ramp_widget::{curve_rows, ramp_section, RampUiCtx};

use super::icons;
use super::state::SettingsTab;
use super::App;
use crate::events::SettingsChangedEvent;
use crate::renderer::OrbitCamera;
use eframe::egui;
use treemap::TreeMapOptions;

pub(super) const LABEL_WIDTH: f32 = 80.0;
pub(super) const SETTINGS_LABEL_WIDTH: f32 = 112.0;
pub(super) const PT_VALUE_WIDTH: f32 = 58.0;

/// Label cell used inside `settings_grid`. Renders the label and wires
/// its hover tooltip to the registry.
pub(super) fn control_label(ui: &mut egui::Ui, label: &'static str) {
    ui.label(label);
}

pub(super) fn settings_grid(
    ui: &mut egui::Ui,
    id: &'static str,
    add_contents: impl FnOnce(&mut egui::Ui),
) {
    egui::Grid::new(id)
        .num_columns(2)
        .spacing([8.0, 4.0])
        .min_col_width(SETTINGS_LABEL_WIDTH)
        .show(ui, add_contents);
}

/// Collapsing-header title for nested subsections (explicit pt, not `TextStyle`).
pub(super) fn section_header_text(title: &str, title_font_pt: f32) -> egui::WidgetText {
    let pt = title_font_pt.clamp(6.0, 48.0);
    egui::RichText::new(title)
        .font(egui::FontId::proportional(pt))
        .into()
}

pub(super) fn tinted_section<R>(
    ui: &mut egui::Ui,
    title: &str,
    default_open: bool,
    mix: f32,
    header_row_height: f32,
    add_contents: impl FnOnce(&mut egui::Ui) -> R,
) -> Option<R> {
    let tint = tint_for_name(ui, title, mix);
    // Thin tinted band that spans the full settings panel width.
    // - `inner_margin` collapsed to 1px top/bottom (was 6×6, too fat) and
    //   3px horizontal so the chevron/title don't kiss the rounded edge.
    let frame = egui::Frame::NONE
        .fill(tint)
        .corner_radius(egui::CornerRadius::same(4))
        .inner_margin(egui::Margin {
            left: 3,
            right: 3,
            top: 1,
            bottom: 1,
        });
    let header_row_height = header_row_height.clamp(8.0, 40.0);

    // Manual `CollapsingState` so the WHOLE tinted band toggles the
    // section, not just the small chevron+label hit-strip that
    // `egui::CollapsingHeader` exposes by default. Persistent id is
    // derived from the title so the open/closed state survives across
    // frames and panel rebuilds.
    let id = ui.make_persistent_id(("tinted_section", title));
    let mut state = egui::collapsing_header::CollapsingState::load_with_default_open(
        ui.ctx(),
        id,
        default_open,
    );

    let header_inner = frame.show(ui, |ui| {
        let spacing = ui.spacing_mut();
        spacing.interact_size.y = header_row_height;
        spacing.item_spacing.y = 1.0;
        spacing.button_padding = egui::vec2(2.0, 1.0);
        ui.set_min_width(ui.available_width());

        // Custom chevron + title row. The chevron rotates with the
        // collapsing-state openness so it matches the look of the
        // standard `CollapsingHeader`.
        ui.horizontal(|ui| {
            let icon_size = egui::Vec2::splat(ui.spacing().icon_width);
            let (_icon_rect, icon_resp) =
                ui.allocate_exact_size(icon_size, egui::Sense::hover());
            let openness = state.openness(ui.ctx());
            // `paint_default_icon` expects a `Response` for the icon
            // rect — we feed it a hover-only one so it can read hover
            // state but the band-level click below owns the toggle.
            let _ = &icon_resp;
            egui::collapsing_header::paint_default_icon(ui, openness, &icon_resp);
            ui.label(egui::RichText::new(title).heading());
        });
    });

    // Re-interact over the FULL tinted band rect with `Sense::click`.
    // egui happily stacks click-sensors at the same area, so the
    // chevron / label inside still draw correctly while the entire
    // bar becomes the toggle target.
    let band_response = ui.interact(
        header_inner.response.rect,
        id.with("__band_toggle"),
        egui::Sense::click(),
    );
    if band_response.clicked() {
        state.toggle(ui);
    }

    state
        .show_body_indented(&band_response, ui, add_contents)
        .map(|r| r.inner)
}

fn tint_for_name(ui: &egui::Ui, name: &str, mix: f32) -> egui::Color32 {
    use std::hash::{Hash, Hasher};
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    name.hash(&mut hasher);
    let h = hasher.finish();
    let r = (h & 0xFF) as u8;
    let g = ((h >> 8) & 0xFF) as u8;
    let b = ((h >> 16) & 0xFF) as u8;
    let base = ui.visuals().widgets.noninteractive.bg_fill;
    let mix = mix.clamp(0.0, 1.0);
    let lerp = |a: u8, b: u8| (a as f32 * (1.0 - mix) + b as f32 * mix) as u8;
    egui::Color32::from_rgb(lerp(base.r(), r), lerp(base.g(), g), lerp(base.b(), b))
}

impl App {
    /// Reset 3D options, treemap layout options, 2D pan/zoom, and orbit camera to
    /// application defaults. Does not change 2D/3D mode or CPU/GPU backend.
    pub(super) fn apply_factory_render_defaults(&mut self) {
        self.render_3d_opts = super::presets::factory_render_3d_options();
        self.opts = TreeMapOptions::default();
        self.viewport.reset();
        self.orbit_camera = OrbitCamera::default();
        self.needs_layout = true;
        self.needs_render_3d = true;
        if let Some(r) = &mut self.renderer_3d {
            r.mark_pt_scene_dirty();
            r.reset_pt_accumulation();
        }
        self.preset_name = super::presets::DEFAULT_PRESET_NAME.to_string();
        self.preset_dirty = false;
        log::info!("Applied default render settings");
    }

    /// Render presets UI (save/load render presets)
    fn ui_presets(&mut self, ui: &mut egui::Ui) {
        ui.horizontal(|ui| {
            ui.label("Preset:");

            // Dropdown button (left of text input)
            let dropdown_resp = ui
                .small_button(icons::CARET_DOWN)
                .on_hover_text("Load preset");
            if dropdown_resp.clicked() {
                self.preset_dropdown_open = !self.preset_dropdown_open;
            }

            // Text input for preset name
            let resp = ui.add(
                egui::TextEdit::singleline(&mut self.preset_name)
                    .desired_width(120.0)
                    .hint_text("name..."),
            );

            // Save on Enter
            if resp.lost_focus()
                && ui.input(|i| i.key_pressed(egui::Key::Enter))
                && !self.preset_name.is_empty()
            {
                self.save_current_preset();
            }

            // Save button
            if ui
                .small_button(icons::FLOPPY_DISK)
                .on_hover_text("Save preset")
                .clicked()
                && !self.preset_name.is_empty()
            {
                self.save_current_preset();
            }

            if ui
                .small_button("Reset")
                .on_hover_text(
                    "Restore defaults: all 3D/render options, treemap layout options, \
                     2D pan/zoom, and orbit camera. Does not switch 2D/3D or CPU/GPU.",
                )
                .clicked()
            {
                self.apply_factory_render_defaults();
                self.preset_dropdown_open = false;
            }

            // Autosave checkbox
            ui.checkbox(&mut self.preset_autosave, "Auto")
                .on_hover_text(format!(
                    "Auto-save preset every {:.0}s when changed",
                    self.autosave_interval_secs
                ));

            // Delete sits at the far-right edge so it's spatially
            // separate from the constructive buttons (save / reset).
            // `right_to_left` consumes the remaining row width and
            // anchors the trash icon to the trailing edge regardless
            // of label/text-input width.
            if self.presets.contains_key(&self.preset_name) {
                ui.with_layout(
                    egui::Layout::right_to_left(egui::Align::Center),
                    |ui| {
                        if ui
                            .small_button(icons::TRASH)
                            .on_hover_text("Delete preset")
                            .clicked()
                        {
                            self.delete_current_preset();
                        }
                    },
                );
            }
        });

        // Camera view bookmarks: six in-memory slots. LMB stores the
        // current orbit-camera state into the slot; RMB recalls the
        // stored state into the live camera. Empty slots default to
        // `OrbitCamera::default()`, so an un-set slot effectively
        // "reset to origin" on recall. Bookmarks are transient (not
        // serialised with presets) — they're quick scratch buffers
        // for camera framing.
        ui.horizontal(|ui| {
            ui.label("Views:");
            for i in 0..self.camera_slots.len() {
                let resp = ui.small_button(format!("{}", i + 1)).on_hover_text(
                    format!(
                        "Camera view {}\nLeft-click: save current view  Right-click: recall",
                        i + 1
                    ),
                );
                if resp.clicked() {
                    // Save: snapshot camera + DoF triple
                    self.camera_slots[i] = super::state::CameraBookmark {
                        camera: self.orbit_camera.clone(),
                        dof_enabled: self.render_3d_opts.pt_dof_enabled,
                        aperture: self.render_3d_opts.pt_aperture,
                        focus_distance: self.render_3d_opts.pt_focus_distance,
                    };
                }
                if resp.secondary_clicked() {
                    // Recall: restore camera + DoF triple. DoF affects
                    // ray gen, so reset PT accumulation to avoid mixing
                    // pre/post-recall samples.
                    let bm = self.camera_slots[i].clone();
                    self.orbit_camera = bm.camera;
                    self.orbit_camera.cancel_animation();
                    self.render_3d_opts.pt_dof_enabled = bm.dof_enabled;
                    self.render_3d_opts.pt_aperture = bm.aperture;
                    self.render_3d_opts.pt_focus_distance = bm.focus_distance;
                    if let Some(r) = &mut self.renderer_3d {
                        r.reset_pt_accumulation();
                    }
                    self.needs_layout = true;
                }
            }
        });

        // Dropdown popup: list every preset from the map (the built-in
        // "defaults" is always present after `load_all_presets`, so no
        // separate row is needed).
        if self.preset_dropdown_open {
            egui::Frame::popup(ui.style()).show(ui, |ui| {
                let mut selected_user: Option<String> = None;

                let mut names: Vec<_> = self.presets.keys().cloned().collect();
                names.sort();
                for name in &names {
                    if ui
                        .selectable_label(self.preset_name == *name, name)
                        .clicked()
                    {
                        selected_user = Some(name.clone());
                    }
                }

                if let Some(name) = selected_user {
                    self.load_preset(&name);
                    self.preset_dropdown_open = false;
                }
            });
        }

        // Close dropdown on click outside
        if self.preset_dropdown_open && ui.input(|i| i.pointer.any_click()) {
            // Will close on next frame if clicked outside popup
        }
    }

    /// Save current render settings as preset.
    ///
    /// The full preset map is rewritten to `presets.json`. Any name is
    /// allowed (including "defaults" — the embedded copy will be
    /// re-injected on next load only if the user later removes it).
    pub(super) fn save_current_preset(&mut self) {
        let name = self.preset_name.trim().to_string();
        if name.is_empty() {
            log::info!("Refusing to save preset with empty name");
            return;
        }
        let preset = super::presets::create_preset(&name, &self.render_3d_opts);
        self.presets.insert(name.clone(), preset);
        match super::presets::save_all_presets(&self.presets) {
            Ok(_) => {
                self.preset_dirty = false;
                self.preset_last_save = std::time::Instant::now();
                log::info!("Saved preset: {}", name);
            }
            Err(e) => {
                log::error!("Failed to save preset: {}", e);
            }
        }
    }

    /// Load a preset by name from the in-memory map (which already
    /// includes the embedded "defaults" entry).
    fn load_preset(&mut self, name: &str) {
        if let Some(preset) = self.presets.get(name).cloned() {
            self.render_3d_opts = preset.render_3d;
            self.preset_name = name.to_string();
            self.needs_layout = true;
            self.preset_dirty = false;
            self.preset_last_save = std::time::Instant::now();
            log::info!("Loaded preset: {}", name);
        }
    }

    /// Delete current preset from the map and persist. Deleting the
    /// built-in "defaults" preset is allowed: it will be re-injected
    /// from the embedded copy on next launch.
    fn delete_current_preset(&mut self) {
        let name = self.preset_name.trim().to_string();
        if name.is_empty() {
            return;
        }
        if self.presets.remove(&name).is_none() {
            return;
        }
        match super::presets::save_all_presets(&self.presets) {
            Ok(_) => {
                self.preset_name.clear();
                log::info!("Deleted preset: {}", name);
            }
            Err(e) => {
                log::error!("Failed to persist presets after delete: {}", e);
            }
        }
    }

    /// Render the settings panel contents
    pub(super) fn ui_settings(&mut self, ui: &mut egui::Ui) {
        let ctx = ui.ctx().clone();
        ui.scope(|ui| {
            self.apply_settings_panel_text_styles(ui);

            // General has been folded into Rendering as the first
            // section, so it no longer has its own tab button.
            let tab_labels = [
                (SettingsTab::Rendering, "Rendering"),
                (SettingsTab::Exclusions, "Exclusions"),
                (SettingsTab::Extensions, "Extensions"),
            ];
            let tab_spacing = ui.spacing().item_spacing.x;
            let tab_count = tab_labels.len().max(1) as f32;
            let tab_width =
                ((ui.available_width() - tab_spacing * (tab_count - 1.0)) / tab_count).max(60.0);
            ui.horizontal(|ui| {
                for (tab, label) in tab_labels {
                    let selected = self.settings_tab == tab;
                    if ui
                        .add_sized(
                            [tab_width, 22.0],
                            egui::Button::new(label).selected(selected),
                        )
                        .clicked()
                    {
                        self.settings_tab = tab;
                    }
                }
            });
            ui.separator();

            egui::ScrollArea::vertical()
                .auto_shrink([false, false])
                .show(ui, |ui| {
                    let w = ui.available_width();
                    ui.set_width(w);

                    let mut changed = false;

                    match self.settings_tab {
                        SettingsTab::Rendering => {
                            // Preset row sits above all sections so the
                            // active preset is the first thing the user
                            // sees and can switch from.
                            self.ui_presets(ui);
                            ui.add_space(4.0);

                            // General sub-sections (scanner/view/
                            // appearance/panel chrome/interaction)
                            // follow as the first collapsible block,
                            // matching the rest of the panel styling
                            // (tinted band, font).
                            let header_h = self.settings_section_header_height;
                            let tint_mix = self.settings_tint_mix;
                            let in_3d = self.render_mode
                                == crate::renderer::RenderMode::Mode3D;
                            tinted_section(
                                ui,
                                "General",
                                false,
                                tint_mix,
                                header_h,
                                |ui| {
                                    self.ui_settings_scanner(ui);
                                    ui.separator();
                                    self.ui_settings_view(ui, &ctx, &mut changed);
                                    ui.separator();
                                    self.ui_settings_appearance(ui, &mut changed);
                                    ui.separator();
                                    self.ui_settings_panel_chrome(ui, &mut changed);
                                    // Interaction lives here as a UX
                                    // preference. Only meaningful in
                                    // 3D mode — hover outline/tint is
                                    // a 3D-only feature.
                                    if in_3d {
                                        ui.separator();
                                        renderer::compact_section(
                                            ui,
                                            "Interaction",
                                            false,
                                            header_h,
                                            |ui| self.ui_interaction_grid(ui),
                                        );
                                    }
                                },
                            );

                            // Denoiser is emitted INSIDE
                            // `ui_settings_renderer`, right after the
                            // Samples section — see the section-order
                            // doc on `ui_3d_settings`.
                            self.ui_settings_renderer(ui, &mut changed);
                        }
                        SettingsTab::Exclusions => {
                            self.ui_settings_exclusions(ui, &mut changed);
                        }
                        SettingsTab::Extensions => {
                            self.ui_ext_stats(ui);
                        }
                    }

                    if changed {
                        self.events.emit(SettingsChangedEvent);
                    }
                });
        });
    }
}
