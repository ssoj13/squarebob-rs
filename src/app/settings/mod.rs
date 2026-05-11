//! Settings panel modules.

mod appearance;
mod denoiser;
mod exclusions;
mod ramp_widget;
mod renderer;
mod scanner;
mod view;

pub(super) use ramp_widget::{curve_rows, ramp_section, RampUiCtx};

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

/// Collapsing-header title for settings subsections (tinted / compact).
pub(super) fn section_header_text(title: &str, title_font_pt: f32) -> egui::WidgetText {
    if title_font_pt > 0.0 {
        egui::RichText::new(title)
            .font(egui::FontId::proportional(title_font_pt))
            .into()
    } else {
        title.into()
    }
}

pub(super) fn tinted_section<R>(
    ui: &mut egui::Ui,
    title: &str,
    default_open: bool,
    mix: f32,
    header_row_height: f32,
    title_font_pt: f32,
    add_contents: impl FnOnce(&mut egui::Ui) -> R,
) -> Option<R> {
    let tint = tint_for_name(ui, title, mix);
    // Thin tinted band that spans the full settings panel width.
    // - `inner_margin` collapsed to 1px top/bottom (was 6×6, too fat) and
    //   3px horizontal so the chevron/title don't kiss the rounded edge.
    // - The Frame is drawn at full available width below.
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
    frame
        .show(ui, |ui| {
            // Tighten the click strip so the title row is one compact line
            // and stretches to the panel edge.
            let spacing = ui.spacing_mut();
            spacing.interact_size.y = header_row_height;
            spacing.item_spacing.y = 1.0;
            spacing.button_padding = egui::vec2(2.0, 1.0);
            ui.set_min_width(ui.available_width());
            egui::CollapsingHeader::new(section_header_text(title, title_font_pt))
                .default_open(default_open)
                .show(ui, add_contents)
                .body_returned
        })
        .inner
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
            let dropdown_resp = ui.small_button("\u{25BC}").on_hover_text("Load preset");
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
                .small_button("\u{1F4BE}")
                .on_hover_text("Save preset")
                .clicked()
                && !self.preset_name.is_empty()
            {
                self.save_current_preset();
            }

            // Delete button (only if preset exists)
            if self.presets.contains_key(&self.preset_name)
                && ui
                    .small_button("\u{1F5D1}")
                    .on_hover_text("Delete preset")
                    .clicked()
            {
                self.delete_current_preset();
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
        });

        // Dropdown popup: built-in defaults + saved presets
        if self.preset_dropdown_open {
            egui::Frame::popup(ui.style()).show(ui, |ui| {
                let mut selected: Option<&'static str> = None;
                let mut selected_user: Option<String> = None;

                let builtin = super::presets::DEFAULT_PRESET_NAME;
                if ui
                    .selectable_label(self.preset_name == builtin, builtin)
                    .clicked()
                {
                    selected = Some(builtin);
                }

                let mut names: Vec<_> = self.presets.keys().cloned().collect();
                names.sort();
                for name in &names {
                    if super::presets::is_builtin_default_preset(name) {
                        continue;
                    }
                    if ui
                        .selectable_label(self.preset_name == *name, name)
                        .clicked()
                    {
                        selected_user = Some(name.clone());
                    }
                }

                if selected.is_some() {
                    self.apply_factory_render_defaults();
                    self.preset_dropdown_open = false;
                } else if let Some(name) = selected_user {
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

    /// Save current render settings as preset
    pub(super) fn save_current_preset(&mut self) {
        if super::presets::is_builtin_default_preset(self.preset_name.trim()) {
            log::warn!("Cannot save under reserved name \"{}\"", super::presets::DEFAULT_PRESET_NAME);
            return;
        }
        let preset = super::presets::create_preset(&self.preset_name, &self.render_3d_opts);
        match super::presets::save_preset(&preset) {
            Ok(_) => {
                self.presets.insert(preset.name.clone(), preset);
                self.preset_dirty = false;
                self.preset_last_save = std::time::Instant::now();
                log::info!("Saved preset: {}", self.preset_name);
            }
            Err(e) => {
                log::error!("Failed to save preset: {}", e);
            }
        }
    }

    /// Load a preset by name
    fn load_preset(&mut self, name: &str) {
        if super::presets::is_builtin_default_preset(name) {
            self.apply_factory_render_defaults();
            return;
        }
        if let Some(preset) = self.presets.get(name).cloned() {
            self.render_3d_opts = preset.render_3d;
            self.preset_name = name.to_string();
            self.needs_layout = true;
            self.preset_dirty = false; // Just loaded, not dirty
            self.preset_last_save = std::time::Instant::now();
            log::info!("Loaded preset: {}", name);
        }
    }

    /// Delete current preset
    fn delete_current_preset(&mut self) {
        if super::presets::is_builtin_default_preset(self.preset_name.trim()) {
            log::warn!("Cannot delete built-in \"{}\"", super::presets::DEFAULT_PRESET_NAME);
            return;
        }
        if let Err(e) = super::presets::delete_preset(&self.preset_name) {
            log::error!("Failed to delete preset: {}", e);
        } else {
            self.presets.remove(&self.preset_name);
            self.preset_name.clear();
            log::info!("Deleted preset");
        }
    }

    /// Render the settings panel contents
    pub(super) fn ui_settings(&mut self, ui: &mut egui::Ui) {
        let ctx = ui.ctx().clone();

        let tab_labels = [
            (SettingsTab::General, "General"),
            (SettingsTab::Rendering, "Rendering"),
            (SettingsTab::Denoiser, "Denoise"),
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
                    SettingsTab::General => {
                        self.ui_settings_scanner(ui);
                        ui.separator();
                        self.ui_settings_view(ui, &ctx, &mut changed);
                        ui.separator();
                        self.ui_settings_appearance(ui, &mut changed);
                        ui.separator();
                        self.ui_settings_panel_chrome(ui, &mut changed);
                    }
                    SettingsTab::Rendering => {
                        self.ui_presets(ui);
                        ui.add_space(4.0);
                        self.ui_settings_renderer(ui);
                    }
                    SettingsTab::Denoiser => {
                        self.ui_settings_denoiser(ui, &mut changed);
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
    }
}
