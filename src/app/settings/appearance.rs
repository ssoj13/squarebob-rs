//! Appearance settings: font size, brightness, cushion, ambient.

use super::LABEL_WIDTH;
use crate::app::App;
use eframe::egui;

impl App {
    /// Per-widget `TextStyle` sizes for the settings sidebar only (see `ui_settings` scope).
    pub(super) fn apply_settings_panel_text_styles(&self, ui: &mut egui::Ui) {
        let style = ui.style_mut();
        style.text_styles.insert(
            egui::TextStyle::Body,
            egui::FontId::proportional(self.settings_panel_font_body),
        );
        style.text_styles.insert(
            egui::TextStyle::Heading,
            egui::FontId::proportional(self.settings_panel_font_heading),
        );
        style.text_styles.insert(
            egui::TextStyle::Small,
            egui::FontId::proportional(self.settings_panel_font_small),
        );
        style.text_styles.insert(
            egui::TextStyle::Button,
            egui::FontId::proportional(self.settings_panel_font_button),
        );
        style.text_styles.insert(
            egui::TextStyle::Monospace,
            egui::FontId::monospace(self.settings_panel_font_monospace),
        );
    }

    /// Appearance section (2D treemap visual settings)
    pub(super) fn ui_settings_appearance(&mut self, ui: &mut egui::Ui, changed: &mut bool) {
        egui::CollapsingHeader::new(egui::RichText::new("Appearance").heading())
            .default_open(true)
            .show(ui, |ui| {
                egui::Grid::new("appearance_grid")
                    .num_columns(2)
                    .spacing([8.0, 4.0])
                    .min_col_width(LABEL_WIDTH)
                    .show(ui, |ui| {
                        ui.label("Font Size:");
                        if ui
                            .add(egui::Slider::new(&mut self.font_size, 8.0..=20.0).suffix("px"))
                            .changed()
                        {
                            self.apply_font_size(ui.ctx());
                        }
                        ui.end_row();

                        ui.label("Tint Mix:");
                        if ui
                            .add(
                                egui::Slider::new(&mut self.settings_tint_mix, 0.0..=0.3)
                                    .show_value(true),
                            )
                            .changed()
                        {
                            *changed = true;
                        }
                        ui.end_row();

                        ui.label("Brightness:");
                        *changed |= ui
                            .add(egui::Slider::new(&mut self.opts.brightness, 0.0..=1.0))
                            .changed();
                        ui.end_row();

                        ui.label("Cushion:");
                        *changed |= ui
                            .add(egui::Slider::new(&mut self.opts.height, 0.0..=1.0))
                            .changed();
                        ui.end_row();

                        ui.label("Scale:");
                        *changed |= ui
                            .add(egui::Slider::new(&mut self.opts.scale_factor, 0.0..=1.0))
                            .changed();
                        ui.end_row();

                        ui.label("Ambient:");
                        *changed |= ui
                            .add(egui::Slider::new(&mut self.opts.ambient_light, 0.0..=1.0))
                            .changed();
                        ui.end_row();
                    });
            });
    }

    /// Typography / chrome for this settings sidebar (General → Settings).
    pub(super) fn ui_settings_panel_chrome(&mut self, ui: &mut egui::Ui, changed: &mut bool) {
        egui::CollapsingHeader::new(egui::RichText::new("Settings").heading())
            .default_open(false)
            .show(ui, |ui| {
                egui::Grid::new("panel_settings_chrome_grid")
                    .num_columns(2)
                    .spacing([8.0, 4.0])
                    .min_col_width(LABEL_WIDTH)
                    .show(ui, |ui| {
                        ui.label("Header row height:");
                        if ui
                            .add(
                                egui::Slider::new(
                                    &mut self.settings_section_header_height,
                                    8.0..=40.0,
                                )
                                .suffix("px"),
                            )
                            .on_hover_text(
                                "Click row height for tinted Path Tracer blocks and compact PT sub-panels.",
                            )
                            .changed()
                        {
                            *changed = true;
                        }
                        ui.end_row();

                        ui.label("Body (labels):");
                        if ui
                            .add(
                                egui::Slider::new(&mut self.settings_panel_font_body, 8.0..=22.0)
                                    .suffix("pt"),
                            )
                            .on_hover_text("Default text and control labels in this sidebar.")
                            .changed()
                        {
                            *changed = true;
                        }
                        ui.end_row();

                        ui.label("Heading:");
                        if ui
                            .add(
                                egui::Slider::new(
                                    &mut self.settings_panel_font_heading,
                                    10.0..=26.0,
                                )
                                .suffix("pt"),
                            )
                            .on_hover_text(
                                "Top-level section titles (Scanner, Geometry, Path Tracer, …).",
                            )
                            .changed()
                        {
                            *changed = true;
                        }
                        ui.end_row();

                        ui.label("Subheading:");
                        if ui
                            .add(
                                egui::Slider::new(
                                    &mut self.settings_panel_font_subheading,
                                    9.0..=22.0,
                                )
                                .suffix("pt"),
                            )
                            .on_hover_text(
                                "Nested groups inside Geometry (Height, Color, …) and palette ramps.",
                            )
                            .changed()
                        {
                            *changed = true;
                        }
                        ui.end_row();

                        ui.label("Small:");
                        if ui
                            .add(
                                egui::Slider::new(&mut self.settings_panel_font_small, 7.0..=16.0)
                                    .suffix("pt"),
                            )
                            .on_hover_text("Widgets that use TextStyle::Small (hints, captions).")
                            .changed()
                        {
                            *changed = true;
                        }
                        ui.end_row();

                        ui.label("Button / tabs:");
                        if ui
                            .add(
                                egui::Slider::new(
                                    &mut self.settings_panel_font_button,
                                    8.0..=22.0,
                                )
                                .suffix("pt"),
                            )
                            .on_hover_text("Tab strip and button labels in this sidebar.")
                            .changed()
                        {
                            *changed = true;
                        }
                        ui.end_row();

                        ui.label("Monospace:");
                        if ui
                            .add(
                                egui::Slider::new(
                                    &mut self.settings_panel_font_monospace,
                                    8.0..=22.0,
                                )
                                .suffix("pt"),
                            )
                            .changed()
                        {
                            *changed = true;
                        }
                        ui.end_row();
                    });
                ui.add_space(6.0);
                if ui
                    .button("Defaults")
                    .on_hover_text(
                        "Reset header row height and all sidebar font sizes to application defaults.",
                    )
                    .clicked()
                {
                    self.settings_section_header_height =
                        crate::app::state::default_settings_section_header_height();
                    self.settings_panel_font_body =
                        crate::app::state::default_settings_panel_font_body();
                    self.settings_panel_font_heading =
                        crate::app::state::default_settings_panel_font_heading();
                    self.settings_panel_font_subheading =
                        crate::app::state::default_settings_panel_font_subheading();
                    self.settings_panel_font_small =
                        crate::app::state::default_settings_panel_font_small();
                    self.settings_panel_font_button =
                        crate::app::state::default_settings_panel_font_button();
                    self.settings_panel_font_monospace =
                        crate::app::state::default_settings_panel_font_monospace();
                    *changed = true;
                }
            });
    }

    /// Apply font size to egui context
    pub(crate) fn apply_font_size(&self, ctx: &egui::Context) {
        let mut style = (*ctx.global_style()).clone();
        // Scale all text styles
        for (_text_style, font_id) in style.text_styles.iter_mut() {
            font_id.size = self.font_size;
        }
        ctx.set_global_style(style);
    }
}
