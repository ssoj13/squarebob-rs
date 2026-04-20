//! Scanner settings.

use eframe::egui;
use crate::app::App;
use crate::app::state::ScannerMode;
use crate::app::helpers::{multibutton_exclusive, MultiButtonAxis};
use super::LABEL_WIDTH;

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
                    multibutton_exclusive(
                        ui,
                        &mut self.scanner_mode,
                        &[
                            (ScannerMode::Standard, "jwalk"),
                            (ScannerMode::Ntfs, "NTFS MFT"),
                        ],
                        MultiButtonAxis::Horizontal,
                    );
                    ui.end_row();
                });
        });
    }
}
