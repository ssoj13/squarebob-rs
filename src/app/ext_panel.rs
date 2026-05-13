//! Right extension statistics panel.

use eframe::egui;

use super::helpers::fmt_size;
use super::icons;
use super::App;

impl App {
    pub(super) fn ui_ext_stats(&mut self, ui: &mut egui::Ui) {
        ui.set_min_width(ui.available_width());

        if self.ext_stats.is_empty() {
            ui.centered_and_justified(|ui| {
                ui.label("No extension stats yet. Scan a folder.");
            });
            return;
        }

        // Header with search + invert filter
        ui.horizontal(|ui| {
            ui.strong("Ext");
            ui.add(
                egui::TextEdit::singleline(&mut self.ext_search_text)
                    .hint_text(icons::MAGNIFYING_GLASS)
                    .desired_width(ui.available_width() - 120.0),
            );
            if ui
                .checkbox(&mut self.ext_filter_invert, "Invert")
                .on_hover_text("Invert extension filter selection")
                .changed()
            {
                self.rebuild_display_tree();
                self.needs_layout = true;
            }
        });

        let mut sorted = self.ext_stats.clone();
        let total: u64 = sorted.iter().map(|(_, s, _)| *s).sum();

        // Filter by search
        let search_lc = self.ext_search_text.to_lowercase();
        if !search_lc.is_empty() {
            sorted.retain(|(ext, _, _)| ext.to_lowercase().contains(&search_lc));
        }

        let (col, asc) = self.ext_sort;
        sorted.sort_by(|a, b| {
            let cmp = match col {
                0 => a.0.cmp(&b.0),
                2 => a.2.cmp(&b.2),
                _ => a.1.cmp(&b.1),
            };
            if asc {
                cmp
            } else {
                cmp.reverse()
            }
        });

        // Calculate column width and count
        let panel_width = ui.available_width();
        let item_width = 90.0_f32;
        let num_cols = ((panel_width / item_width) as usize).max(1);
        let mut filter_set: std::collections::HashSet<String> =
            self.ext_filter.iter().map(|e| e.to_lowercase()).collect();
        let mut filter_changed = false;

        egui::ScrollArea::vertical().show(ui, |ui| {
            ui.style_mut().spacing.item_spacing = egui::vec2(4.0, 2.0);

            egui::Grid::new("ext_grid")
                .num_columns(num_cols)
                .spacing([4.0, 2.0])
                .show(ui, |ui| {
                    for (i, (ext, size, count)) in sorted.iter().enumerate() {
                        let ext_key = ext.to_lowercase();
                        let selected = filter_set.contains(&ext_key);
                        let color = treemap::ext_color(if ext == "<none>" { "" } else { ext });
                        let (r, g, b) = (color[0], color[1], color[2]);
                        let pct = if total > 0 {
                            *size as f64 / total as f64 * 100.0
                        } else {
                            0.0
                        };
                        let label = format!("{} {:.0}%", ext, pct);
                        let resp = ui.add_sized(
                            [item_width, 0.0],
                            egui::Button::new(
                                egui::RichText::new(label).color(egui::Color32::from_rgb(r, g, b)),
                            )
                            .selected(selected),
                        );
                        if resp.clicked() {
                            if selected {
                                filter_set.remove(&ext_key);
                            } else {
                                filter_set.insert(ext_key);
                            }
                            filter_changed = true;
                        }
                        resp.on_hover_text(format!(
                            "{}\n{} ({:.1}%)\n{} files",
                            ext,
                            fmt_size(*size),
                            pct,
                            count
                        ));

                        if (i + 1) % num_cols == 0 {
                            ui.end_row();
                        }
                    }

                    let rem = sorted.len() % num_cols;
                    if rem != 0 {
                        for _ in 0..(num_cols - rem) {
                            ui.add_sized([item_width, 0.0], egui::Label::new(""));
                        }
                        ui.end_row();
                    }
                });
        });

        if filter_changed {
            let mut next: Vec<String> = filter_set.into_iter().collect();
            next.sort();
            self.ext_filter = next;
            self.rebuild_display_tree();
            self.needs_layout = true;
        }
    }
}
