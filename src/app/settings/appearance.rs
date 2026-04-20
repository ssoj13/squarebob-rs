//! Appearance settings: font size, brightness, cushion, ambient.

use eframe::egui;
use crate::app::App;
use super::LABEL_WIDTH;

impl App {
    /// Appearance section (2D treemap visual settings)
    pub(super) fn ui_settings_appearance(&mut self, ui: &mut egui::Ui, changed: &mut bool) {
        egui::CollapsingHeader::new("Appearance").default_open(true).show(ui, |ui| {
            egui::Grid::new("appearance_grid")
                .num_columns(2)
                .spacing([8.0, 4.0])
                .min_col_width(LABEL_WIDTH)
                .show(ui, |ui| {
                    ui.label("Font Size:");
                    if ui.add(egui::Slider::new(&mut self.font_size, 8.0..=20.0).suffix("px")).changed() {
                        self.apply_font_size(ui.ctx());
                    }
                    ui.end_row();

                    ui.label("Tint Mix:");
                    if ui.add(egui::Slider::new(&mut self.settings_tint_mix, 0.0..=0.2).show_value(true)).changed() {
                        *changed = true;
                    }
                    ui.end_row();

                    ui.label("Brightness:");
                    *changed |= ui.add(egui::Slider::new(&mut self.opts.brightness, 0.0..=1.0)).changed();
                    ui.end_row();

                    ui.label("Cushion:");
                    *changed |= ui.add(egui::Slider::new(&mut self.opts.height, 0.0..=1.0)).changed();
                    ui.end_row();

                    ui.label("Scale:");
                    *changed |= ui.add(egui::Slider::new(&mut self.opts.scale_factor, 0.0..=1.0)).changed();
                    ui.end_row();

                    ui.label("Ambient:");
                    *changed |= ui.add(egui::Slider::new(&mut self.opts.ambient_light, 0.0..=1.0)).changed();
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
