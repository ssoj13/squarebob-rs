//! Output / Encoder settings section — inline-rendered Encoder UI.
//!
//! Lives under Settings → Rendering as a tinted section, peer of
//! Denoiser. Reuses the same `EncodeDialog` state as the legacy
//! window-based encoder (F12 shortcut) — both views share the same
//! progress / encoding lifecycle, so flipping between them never
//! interrupts a running export.

use eframe::egui;
use media_encoder::Project;

use super::{SettingsDirty, tinted_section};
use crate::app::App;

/// Inner content width for the inline encoder, in logical egui points.
/// Matches the visual rhythm of other Settings sections (Denoiser,
/// Camera). The encoder's per-row widgets (file path, slider, progress
/// bar) automatically wrap / clip to this width.
const OUTPUT_INNER_WIDTH: f32 = 320.0;

impl App {
    /// Render the Output / Encoder section inline inside the
    /// Settings → Rendering tab. Mirrors the lifecycle bookkeeping
    /// that `ui_encode_dialog_window` performs (refresh frame source,
    /// poll progress) so the inline UI behaves identically.
    /// Output / Encoder inline UI. The encoder owns its own progress
    /// lifecycle and does not feed the preset/layout dirty signal
    /// — the `_dirty` parameter is kept for sig parity with the
    /// other `ui_settings_*` callbacks so a future encoder knob
    /// (e.g. a preset-tracked default codec) can opt in without
    /// touching call sites.
    pub(super) fn ui_settings_output(&mut self, ui: &mut egui::Ui, _dirty: &mut SettingsDirty) {
        tinted_section(
            ui,
            "Output",
            false,
            self.settings_tint_mix,
            self.settings_section_header_height,
            |ui| {
                // Cap the encoder content width so it matches the
                // visual rhythm of peer sections (Denoiser, Camera,
                // etc.) — the standalone window UI was designed for
                // 600px and looks bloated when allowed to span the
                // whole Settings panel.
                let inner_w = ui.available_width().min(OUTPUT_INNER_WIDTH);
                ui.scope(|ui| {
                    ui.set_max_width(inner_w);
                    ui.set_min_width(inner_w);

                    let project = Project;
                    let active_comp = self.encode_source.clone();
                    // `with_close_button = false`: inline mode suppresses
                    // Close (section is collapsible via header chevron)
                    // and stretches Encode/Stop full-width.
                    let _close_requested =
                        self.encode_dialog
                            .render_inline(ui, &project, active_comp.as_ref(), false);
                });
            },
        );
    }
}
