//! Encode dialog adapter for the reusable `media-encoder` crate.

use eframe::egui;

use super::App;

impl App {
    pub(super) fn ui_encode_dialog_window(&mut self, ctx: &egui::Context) {
        if !self.show_encode_panel {
            return;
        }

        let project = media_encoder::Project::default();
        if !self.encode_dialog.render(ctx, &project, None) {
            self.show_encode_panel = false;
        }
    }

    pub(super) fn handle_image_sequence(&mut self, _ctx: &egui::Context) {}
}
