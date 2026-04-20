//! Bottom status bar: scan progress, file info, hover info.

use eframe::egui;

use crate::cache;
use super::App;
use super::helpers::{fmt_size, disk_free_info};

impl App {
    /// Render bottom status bar
    pub(super) fn ui_status_bar(&mut self, ui: &mut egui::Ui) {
        egui::Panel::bottom("status").show_inside(ui, |ui| {
            ui.horizontal(|ui| {
                if self.progress.scanning {
                    ui.spinner();
                    let elapsed = self.progress.start_time
                        .map(|t| t.elapsed().as_secs_f32())
                        .unwrap_or(0.0);
                    let err_str = if self.progress.errors > 0 {
                        format!(" | {} errors", self.progress.errors)
                    } else {
                        String::new()
                    };
                    ui.label(format!(
                        "Scanning: {} files, {} dirs, {} ({:.1}s){}",
                        self.progress.files, self.progress.dirs,
                        fmt_size(self.progress.bytes), elapsed, err_str,
                    ));
                    let anim = (elapsed * 2.0).sin() * 0.5 + 0.5;
                    ui.add(egui::ProgressBar::new(anim).desired_width(100.0));
                } else if let Some(err) = &self.progress.error {
                    ui.colored_label(egui::Color32::RED, format!("Error: {err}"));
                } else if let Some(tree) = &self.tree {
                    let disk_info = disk_free_info(&self.scan_path);
                    let time_info = if let Some(age) = self.cache_age {
                        ui.colored_label(egui::Color32::from_rgb(180, 180, 80), "\u{25cf}");
                        format!(" cached: {}", cache::format_age(age))
                    } else {
                        format!(" in {:.1}s", self.progress.elapsed_secs)
                    };
                    ui.label(format!(
                        "{} files | {} dirs | {}{}{}",
                        tree.file_count, tree.dir_count, fmt_size(tree.size),
                        time_info, disk_info,
                    ));
                } else {
                    ui.label("Select a folder and click Scan to analyze disk usage");
                }

                // Right side stats + hover
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    let now = std::time::Instant::now();
                    if now.duration_since(self.last_mem_update).as_secs_f32() > 0.5 {
                        self.sys.refresh_memory();
                        let total_kb = self.sys.total_memory();
                        let used_kb = self.sys.used_memory();
                        self.mem_total_mb = (total_kb / 1024).max(1);
                        self.mem_used_mb = used_kb / 1024;
                        self.last_mem_update = now;
                    }
                    if self.mem_total_mb > 0 {
                        ui.label(format!("RAM {} / {} MB", self.mem_used_mb, self.mem_total_mb));
                    }
                    if self.last_frame_ms > 0.0 {
                        let mut stats = format!("{:.1} FPS | {:.1} ms", self.last_fps, self.last_frame_ms);
                        if self.last_samples_per_sec > 0.0 {
                            stats.push_str(&format!(" | {:.0} spp/s", self.last_samples_per_sec));
                        }
                        ui.label(stats);
                    }
                    if let Some(hover) = &self.hovered {
                        ui.label(format!("{} ({})", hover.path, fmt_size(hover.size)));
                    }
                });
            });
        });
    }
}
