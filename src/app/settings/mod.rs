//! Settings panel modules.

mod appearance;
mod color;
mod denoiser;
mod dirty;
mod exclusions;
pub(super) mod material_presets;
pub(super) mod materials;
mod output;
mod preset_buttons;
mod ramp_widget;
mod renderer;
mod scanner;
mod view;

pub(super) use dirty::SettingsDirty;
pub(super) use ramp_widget::{RampUiCtx, curve_rows, ramp_section};

use super::App;
use super::state::SettingsTab;
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
            let (_icon_rect, icon_resp) = ui.allocate_exact_size(icon_size, egui::Sense::hover());
            let openness = state.openness(ui.ctx());
            // `paint_default_icon` expects a `Response` for the icon
            // rect — we feed it a hover-only one so it can read hover
            // state but the band-level click below owns the toggle.
            let _ = &icon_resp;
            egui::collapsing_header::paint_default_icon(ui, openness, &icon_resp);
            // `Label::selectable(false)` so the heading text doesn't
            // intercept the cursor as an I-beam — the whole tinted
            // band is meant to read as one clickable strip, not a
            // text-selection target.
            ui.add(egui::Label::new(egui::RichText::new(title).heading()).selectable(false));
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
        let mut names: Vec<_> = self.presets.keys().cloned().collect();
        names.sort();
        if let Some(action) = preset_buttons::show(ui, &names, &self.preset_name, self.preset_dirty)
        {
            let result = self.apply_preset_action(action);
            if let Err(error) = &result {
                log::error!("Preset change failed: {error}");
            }
            preset_buttons::finish(ui.ctx(), result);
        }
        ui.horizontal_wrapped(|ui| {
            if ui
                .small_button("Reset")
                .on_hover_text(
                    "Restore defaults: all 3D/render options, treemap layout options, \
                     2D pan/zoom, and orbit camera. Does not switch 2D/3D or CPU/GPU.",
                )
                .clicked()
            {
                self.apply_factory_render_defaults();
            }
            ui.add_enabled(
                self.presets.contains_key(&self.preset_name),
                egui::Checkbox::new(&mut self.preset_autosave, "Auto"),
            )
            .on_hover_text(format!(
                "Auto-save the selected preset every {:.0}s when changed",
                self.autosave_interval_secs
            ));
        });

        ui.horizontal(|ui| self.ui_camera_slots(ui));
    }

    /// Save current render settings as preset.
    ///
    /// The full preset map is rewritten to `presets.json`. Any name is
    /// allowed (including "defaults" — the embedded copy will be
    /// re-injected on next load only if the user later removes it).
    pub(super) fn save_current_preset(&mut self) {
        let result =
            self.apply_preset_action(preset_buttons::Action::Save(self.preset_name.clone()));
        if let Err(error) = &result {
            log::error!("Failed to save preset: {error}");
        }
    }

    fn apply_preset_action(&mut self, action: preset_buttons::Action) -> Result<(), String> {
        use super::presets::PresetChange;
        use preset_buttons::Action;
        match &action {
            Action::Load(name) => {
                if !self.presets.contains_key(name) {
                    return Err(format!("Preset '{name}' no longer exists."));
                }
                self.load_preset(name);
                return Ok(());
            }
            Action::Delete(name) => return self.delete_current_preset(name),
            _ => {}
        }
        let change = match &action {
            Action::Save(name) => PresetChange::Save {
                name,
                create: false,
            },
            Action::Create(name) => PresetChange::Save { name, create: true },
            Action::Rename { from, to } => PresetChange::Rename { from, to },
            Action::Load(_) | Action::Delete(_) => unreachable!(),
        };
        let name = super::presets::change_presets(
            &mut self.presets,
            change,
            &self.render_3d_opts,
            |next| {
                super::presets::save_all_presets(next)
                    .map(|_| ())
                    .map_err(|e| e.to_string())
            },
        )?;
        match action {
            Action::Save(_) | Action::Create(_) => {
                self.preset_name = name;
                self.preset_dirty = false;
                self.preset_last_save = std::time::Instant::now();
            }
            Action::Rename { from, .. } if self.preset_name == from => self.preset_name = name,
            _ => {}
        }
        Ok(())
    }

    /// Load a preset by name from the in-memory map (which already
    /// includes the embedded "defaults" entry).
    fn load_preset(&mut self, name: &str) {
        if let Some(preset) = self.presets.get(name).cloned() {
            self.render_3d_opts = preset.render_3d;
            self.preset_name = name.to_string();
            self.needs_layout = true;
            self.needs_render_3d = true;
            if let Some(renderer) = &mut self.renderer_3d {
                renderer.mark_pt_scene_dirty();
                renderer.reset_pt_accumulation();
            }
            self.preset_dirty = false;
            self.preset_last_save = std::time::Instant::now();
            log::info!("Loaded preset: {}", name);
        }
    }

    /// Delete current preset from the map and persist. Deleting the
    /// built-in "defaults" preset is allowed: it will be re-injected
    /// from the embedded copy on next launch.
    fn delete_current_preset(&mut self, name: &str) -> Result<(), String> {
        super::presets::change_presets(
            &mut self.presets,
            super::presets::PresetChange::Delete(name),
            &self.render_3d_opts,
            |next| {
                super::presets::save_all_presets(next)
                    .map(|_| ())
                    .map_err(|e| e.to_string())
            },
        )?;
        if self.preset_name == name {
            self.preset_name.clear();
            self.preset_dirty = false;
        }
        Ok(())
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

                    let mut dirty = SettingsDirty::default();

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
                            let in_3d = self.render_mode == crate::renderer::RenderMode::Mode3D;
                            tinted_section(ui, "General", false, tint_mix, header_h, |ui| {
                                self.ui_settings_scanner(ui);
                                ui.separator();
                                self.ui_settings_view(ui, &ctx, &mut dirty);
                                ui.separator();
                                self.ui_settings_appearance(ui, &mut dirty);
                                ui.separator();
                                self.ui_settings_panel_chrome(ui, &mut dirty);
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
                            });

                            // Denoiser is emitted INSIDE
                            // `ui_settings_renderer`, right after the
                            // Samples section — see the section-order
                            // doc on `ui_3d_settings`.
                            self.ui_settings_renderer(ui, &mut dirty);
                        }
                        SettingsTab::Exclusions => {
                            self.ui_settings_exclusions(ui, &mut dirty);
                        }
                        SettingsTab::Extensions => {
                            self.ui_ext_stats(ui);
                        }
                    }

                    // Dispatch the typed dirtiness signal to the matching
                    // follow-up actions. Each flag maps to one specific
                    // side-effect, in increasing cost order, so e.g. a
                    // denoise hyper-param change (preset only) doesn't
                    // accidentally trigger a treemap rebuild or a
                    // PT-accumulation reset.
                    //
                    // The `pt_scene` branch supersedes `pt_accum`
                    // because `mark_pt_scene_dirty` already implies a
                    // full re-init that re-zeroes `frame_count`; firing
                    // both would just double-work the renderer.
                    //
                    // The pre-existing material editor in `materials.rs`
                    // and the PT-knob channel in `renderer.rs::ui_pt_*`
                    // still emit `MaterialsChangedEvent` /
                    // `reset_pt_accumulation` directly because they
                    // pre-date this dispatcher; new settings should
                    // prefer the typed `dirty.*()` methods below.
                    if dirty.is_pt_scene() {
                        if let Some(r) = &mut self.renderer_3d {
                            r.mark_pt_scene_dirty();
                        }
                        self.needs_render_3d = true;
                    } else if dirty.is_pt_accum() {
                        if let Some(r) = &mut self.renderer_3d {
                            r.mark_pt_accum_reset();
                        }
                        self.needs_render_3d = true;
                    }
                    if dirty.is_materials() {
                        self.events.emit(crate::events::MaterialsChangedEvent);
                    }
                    if dirty.is_layout() {
                        self.needs_layout = true;
                    }
                    if dirty.is_preset() {
                        self.preset_dirty = true;
                    }
                    if dirty.any() {
                        ctx.request_repaint();
                    }
                });
        });
    }
}
