//! View settings: display, layout, size filter.

use eframe::egui;
use treemap::LayoutStyle;
use crate::app::App;
use crate::app::helpers::{fmt_size, multibutton_exclusive, MultiButtonAxis};
use crate::app::filters::count_files_in_range;
use super::LABEL_WIDTH;

impl App {
    /// View section (Display + Layout + Size Filter)
    pub(super) fn ui_settings_view(&mut self, ui: &mut egui::Ui, ctx: &egui::Context, changed: &mut bool) {
        egui::CollapsingHeader::new("View").default_open(true).show(ui, |ui| {
            egui::Grid::new("view_grid")
                .num_columns(2)
                .spacing([8.0, 4.0])
                .min_col_width(LABEL_WIDTH)
                .show(ui, |ui| {
                    ui.label("Options:");
                    ui.horizontal(|ui| {
                        let old = self.show_free_space;
                        ui.checkbox(&mut self.show_free_space, "Free space")
                            .on_hover_text("Show unallocated disk space as gray blocks");
                        if self.show_free_space != old {
                            self.rebuild_display_tree();
                            *changed = true;
                        }
                        *changed |= ui.checkbox(&mut self.opts.grid, "Grid")
                            .on_hover_text("Draw thin border lines between blocks for better visibility")
                            .changed();
                    });
                    ui.end_row();

                    ui.label("Layout:");
                    ui.horizontal(|ui| {
                        let old = self.opts.style;
                        if multibutton_exclusive(
                            ui,
                            &mut self.opts.style,
                            &[
                                (LayoutStyle::KDirStat, "KDirStat"),
                                (LayoutStyle::SequoiaView, "SequoiaView"),
                            ],
                            MultiButtonAxis::Horizontal,
                        ) && self.opts.style != old {
                            *changed = true;
                        }
                    });
                    ui.end_row();
                });
            
            ui.separator();
            
            // Size filter
            self.ui_size_filter(ui, ctx);
        });
    }

    /// Size filter controls
    fn ui_size_filter(&mut self, ui: &mut egui::Ui, _ctx: &egui::Context) {
        let max_val = self.scan_max_size.max(1);

        egui::Grid::new("filter_grid")
            .num_columns(2)
            .spacing([8.0, 4.0])
            .min_col_width(LABEL_WIDTH)
            .show(ui, |ui| {
                ui.label("Min:");
                let min_changed = ui.add(egui::Slider::new(&mut self.filter_min, 0..=max_val)
                    .custom_formatter(|v, _| fmt_size(v as u64))
                    .logarithmic(true))
                    .changed();
                if min_changed {
                    if self.filter_min > self.filter_max {
                        self.filter_max = self.filter_min;
                    }
                    self.needs_filter_rebuild = true;
                    self.filter_changed_at = Some(std::time::Instant::now());
                }
                ui.end_row();

                ui.label("Max:");
                let max_changed = ui.add(egui::Slider::new(&mut self.filter_max, 0..=max_val)
                    .custom_formatter(|v, _| fmt_size(v as u64))
                    .logarithmic(true))
                    .changed();
                if max_changed {
                    if self.filter_max < self.filter_min {
                        self.filter_min = self.filter_max;
                    }
                    self.needs_filter_rebuild = true;
                    self.filter_changed_at = Some(std::time::Instant::now());
                }
                ui.end_row();
            });

        // Show range summary + file count
        let (sel_files, total_files, is_preview) = match (&self.tree, &self.filtered_tree) {
            (Some(root), Some(filtered)) => {
                if self.needs_filter_rebuild && !self.filter_auto_rebuild {
                    let preview = count_files_in_range(root, self.filter_min, self.filter_max, self.filter_invert);
                    (preview, root.file_count, true)
                } else {
                    (filtered.file_count, root.file_count, false)
                }
            }
            (Some(root), None) => {
                if self.needs_filter_rebuild && !self.filter_auto_rebuild {
                    let preview = count_files_in_range(root, self.filter_min, self.filter_max, self.filter_invert);
                    (preview, root.file_count, true)
                } else {
                    (root.file_count, root.file_count, false)
                }
            }
            _ => (0, 0, false),
        };
        let range_label = if self.filter_invert {
            format!("Showing outside: {} - {} ({} / {} files)", fmt_size(self.filter_min), fmt_size(self.filter_max), sel_files, total_files)
        } else {
            format!("Showing {} - {} ({} / {} files)", fmt_size(self.filter_min), fmt_size(self.filter_max), sel_files, total_files)
        };
        if is_preview {
            ui.colored_label(egui::Color32::from_rgb(255, 165, 0), range_label);
        } else {
            ui.small(range_label);
        }

        ui.horizontal(|ui| {
            if ui.checkbox(&mut self.filter_invert, "Invert")
                .on_hover_text("Show files OUTSIDE the size range")
                .changed()
            {
                self.needs_filter_rebuild = true;
                self.filter_changed_at = Some(std::time::Instant::now());
            }
            ui.checkbox(&mut self.filter_auto_rebuild, "Auto")
                .on_hover_text("Auto-apply filter");
            if !self.filter_auto_rebuild && self.needs_filter_rebuild
                && ui.small_button("Apply").clicked() {
                    self.rebuild_filtered_tree();
                }
        });

        // Auto-apply logic
        if self.needs_filter_rebuild && self.filter_auto_rebuild {
            self.rebuild_filtered_tree();
        }
    }
}
