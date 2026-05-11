//! Appearance settings: font size, brightness, cushion, ambient.

use super::LABEL_WIDTH;
use crate::app::App;
use eframe::egui;

impl App {
    /// Appearance section (2D treemap visual settings)
    pub(super) fn ui_settings_appearance(&mut self, ui: &mut egui::Ui, changed: &mut bool) {
        egui::CollapsingHeader::new("Appearance")
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
                                egui::Slider::new(&mut self.settings_tint_mix, 0.0..=0.2)
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

    /// Panel chrome: collapsing header row height and title font for settings subsections.
    pub(super) fn ui_settings_panel_chrome(&mut self, ui: &mut egui::Ui, changed: &mut bool) {
        egui::CollapsingHeader::new("Settings")
            .default_open(false)
            .show(ui, |ui| {
                egui::Grid::new("panel_settings_chrome_grid")
                    .num_columns(2)
                    .spacing([8.0, 4.0])
                    .min_col_width(LABEL_WIDTH)
                    .show(ui, |ui| {
                        ui.label("Section header height:");
                        if ui
                            .add(
                                egui::Slider::new(
                                    &mut self.settings_section_header_height,
                                    8.0..=40.0,
                                )
                                .suffix("px"),
                            )
                            .changed()
                        {
                            *changed = true;
                        }
                        ui.end_row();

                        ui.label("Section title font:");
                        let title_slider = egui::Slider::new(
                            &mut self.settings_section_title_font_size,
                            0.0..=24.0,
                        )
                        .custom_formatter(|v, _| {
                            if v <= 0.0 {
                                "Auto".to_owned()
                            } else {
                                format!("{v:.0} pt")
                            }
                        });
                        if ui
                            .add(title_slider)
                            .on_hover_text(
                                "0 (Auto) follows the Appearance → Font Size for subsection titles.",
                            )
                            .changed()
                        {
                            *changed = true;
                        }
                        ui.end_row();
                    });
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
