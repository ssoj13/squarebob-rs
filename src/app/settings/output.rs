//! Output / Encoder settings section — inline-rendered Encoder UI.
//!
//! Lives under Settings → Rendering as a tinted section, peer of
//! Denoiser. Reuses the same `EncodeDialog` state as the legacy
//! window-based encoder (toggled by F12) — both views share the same
//! progress / encoding lifecycle, so flipping between them never
//! interrupts a running export.
//!
//! Visibility is opt-in via `App::show_output_section`. When the user
//! disables it, the section header still shows but its body is
//! suppressed; this keeps the encoder discoverable without baking it
//! into the default rendering layout.

use std::sync::Arc;

use eframe::egui;
use media_encoder::Project;

use super::{renderer, tinted_section};
use crate::app::App;
use crate::app::image_sequence::SquarebobEncodeSource;

impl App {
    /// Render the Output / Encoder section inline inside the
    /// Settings → Rendering tab. Mirrors the lifecycle bookkeeping
    /// that `ui_encode_dialog_window` performs (refresh frame source,
    /// poll progress) so the inline UI behaves identically.
    pub(super) fn ui_settings_output(&mut self, ui: &mut egui::Ui, _changed: &mut bool) {
        tinted_section(
            ui,
            "Output",
            false,
            self.settings_tint_mix,
            self.settings_section_header_height,
            |ui| {
                ui.horizontal(|ui| {
                    ui.checkbox(&mut self.show_output_section, "Enabled")
                        .on_hover_text(
                            "Show the inline encoder controls below. The \
                             legacy free-floating window remains available \
                             via the F12 shortcut and shares the same state \
                             — disabling here does not cancel a running \
                             encode.",
                        );
                });

                if !self.show_output_section {
                    ui.label(
                        egui::RichText::new(
                            "Encoder hidden. Press F12 to open the legacy window.",
                        )
                        .small()
                        .weak(),
                    );
                    return;
                }

                ui.separator();

                renderer::compact_section(
                    ui,
                    "Encoder",
                    true,
                    self.settings_section_header_height,
                    |ui| {
                        // Drain progress + refresh the lazy frame
                        // source before painting (matches what the
                        // window path does in `ui_encode_dialog_window`).
                        self.encode_dialog.poll_encoding_state(ui.ctx());
                        if !self.encode_dialog.is_encoding {
                            self.refresh_encode_source_inline();
                        }

                        let project = Project;
                        let active_comp = self.encode_source.clone();
                        let _close_requested = self.encode_dialog.render_inline(
                            ui,
                            &project,
                            active_comp.as_ref(),
                        );
                        // Inline mode ignores the Close request — the
                        // section has its own visibility toggle.
                    },
                );
            },
        );
    }

    /// Same logic as the private `refresh_encode_source` method in
    /// `image_sequence.rs`, just public to the parent module so the
    /// inline output section can prime the lazy source itself without
    /// reaching into the window-only handler. Cheap when nothing
    /// changed (the cache check makes most calls a no-op).
    fn refresh_encode_source_inline(&mut self) {
        if self.encode_dialog.is_encoding {
            return;
        }

        let (w, h) = self.last_render_size;
        if w == 0 || h == 0 {
            self.encode_source = None;
            self.encode_sequence_source = None;
            self.encode_source_size = (0, 0);
            return;
        }

        let frame_start = self.encode_dialog.frame_start;
        let frame_end = self.encode_dialog.frame_end.max(frame_start);
        let fps = self.encode_dialog.fps.max(1.0);

        if let Some(source) = &self.encode_sequence_source
            && source.matches(w, h, frame_start, frame_end, fps)
        {
            self.encode_source_size = (w, h);
            return;
        }

        let source = Arc::new(SquarebobEncodeSource::new(w, h, frame_start, frame_end, fps));
        let comp: media_encoder::Comp = source.clone();
        self.encode_sequence_source = Some(source);
        self.encode_source = Some(comp);
        self.encode_source_size = (w, h);
    }
}
