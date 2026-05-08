//! Exclusions settings.

use super::LABEL_WIDTH;
use crate::app::App;
use crate::exclusions;
use eframe::egui;
use std::path::PathBuf;

impl App {
    /// Exclusions section
    pub(super) fn ui_settings_exclusions(&mut self, ui: &mut egui::Ui, changed: &mut bool) {
        egui::CollapsingHeader::new("Exclusions")
            .default_open(false)
            .show(ui, |ui| {
                egui::Grid::new("exclusions_grid")
                    .num_columns(2)
                    .spacing([8.0, 4.0])
                    .min_col_width(LABEL_WIDTH)
                    .show(ui, |ui| {
                        ui.label("Options:");
                        ui.horizontal(|ui| {
                            let old = self.show_excluded;
                            ui.checkbox(&mut self.show_excluded, "Show excluded");
                            if self.show_excluded != old {
                                self.rebuild_display_tree();
                                *changed = true;
                            }
                            if !self.exclusions.is_empty() && ui.small_button("Clear all").clicked()
                            {
                                self.exclusions.clear();
                                exclusions::save(&self.exclusions);
                                self.rebuild_display_tree();
                                self.needs_layout = true;
                            }
                        });
                        ui.end_row();
                    });

                if self.exclusions.is_empty() {
                    ui.small("Right-click to exclude items");
                } else {
                    let mut to_remove: Option<PathBuf> = None;
                    egui::ScrollArea::vertical()
                        .max_height(100.0)
                        .show(ui, |ui| {
                            for path_str in self.exclusions.sorted_list() {
                                ui.horizontal(|ui| {
                                    if ui.small_button("x").clicked() {
                                        to_remove = Some(PathBuf::from(&path_str));
                                    }
                                    let path = PathBuf::from(&path_str);
                                    let name = path
                                        .file_name()
                                        .map(|n| n.to_string_lossy().to_string())
                                        .unwrap_or_else(|| path_str.clone());
                                    ui.small(&name).on_hover_text(&path_str);
                                });
                            }
                        });
                    if let Some(path) = to_remove {
                        self.include_path(&path);
                    }
                }
            });
    }
}
