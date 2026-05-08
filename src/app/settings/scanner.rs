//! Scanner settings.

use super::LABEL_WIDTH;
use crate::app::helpers::{multibutton_exclusive, MultiButtonAxis};
use crate::app::state::ScannerMode;
use crate::app::App;
use crate::cache;
use eframe::egui;

impl App {
    /// Scanner section
    pub(super) fn ui_settings_scanner(&mut self, ui: &mut egui::Ui) {
        egui::CollapsingHeader::new("Scanner").default_open(true).show(ui, |ui| {
            egui::Grid::new("scanner_grid")
                .num_columns(2)
                .spacing([8.0, 4.0])
                .min_col_width(LABEL_WIDTH)
                .show(ui, |ui| {
                    ui.label("Engine:");
                    if multibutton_exclusive(
                        ui,
                        &mut self.scanner_mode,
                        &[
                            (ScannerMode::Standard, "jwalk"),
                            (ScannerMode::Ntfs, "NTFS MFT"),
                        ],
                        MultiButtonAxis::Horizontal,
                    ) && self.progress.scanning {
                        self.stop_scan();
                        self.start_scan();
                    }
                    if self.progress.scanning {
                        if let Some(ref eng) = self.progress.scan_engine_label {
                            ui.label(egui::RichText::new(format!("Active: {eng}")).small().weak());
                        }
                    }
                    ui.end_row();
                    ui.label("Disk cache:");
                    if ui.button("Clear cache for current path").on_hover_text(
                        "Removes the saved scan snapshot from disk for this root. Rescan to rebuild."
                    ).clicked() {
                        if let Err(e) = cache::delete_cache(&self.scan_path) {
                            log::warn!("Failed to delete cache: {e}");
                        } else {
                            self.cache_age = None;
                        }
                    }
                    ui.end_row();
                });
        });
    }
}
