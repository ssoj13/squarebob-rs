//! PT denoiser settings panel — Stage D.2 of the TODO4 roadmap.
//!
//! This is a dedicated tab so the denoiser controls are not buried
//! inside the Rendering tab (which is already crowded with PT, ReSTIR,
//! path-guiding, materials, hover, etc.).
//!
//! MVP variant: color-only edge stopping (à-trous filter). G-buffer
//! guidance (normal/depth) is deferred — see
//! `crates/pt-megakernel/src/denoiser/atrous.wgsl` and CHANGELOG.md.

use super::LABEL_WIDTH;
use crate::app::App;
use eframe::egui;

impl App {
    /// Render the Denoiser settings tab.
    pub(super) fn ui_settings_denoiser(&mut self, ui: &mut egui::Ui, changed: &mut bool) {
        egui::CollapsingHeader::new(egui::RichText::new("Denoiser (À-trous)").heading())
            .default_open(true)
            .show(ui, |ui| {
                egui::Grid::new("denoiser_grid")
                    .num_columns(2)
                    .spacing([8.0, 4.0])
                    .min_col_width(LABEL_WIDTH)
                    .show(ui, |ui| {
                        // Master enable toggle.
                        ui.label("Enabled:");
                        ui.horizontal(|ui| {
                            *changed |= ui
                                .checkbox(&mut self.render_3d_opts.pt_denoise_enabled, "")
                                .on_hover_text(
                                    "Run an edge-aware à-trous filter on every PT sample. \
                                     Reduces noise dramatically at low spp; loses some \
                                     fine detail. Cost: ~1 ms / iteration at 1080p.",
                                )
                                .changed();
                            ui.label(if self.render_3d_opts.pt_denoise_enabled {
                                "ON"
                            } else {
                                "off"
                            });
                        });
                        ui.end_row();

                        // Iteration count (1..=5). Each iteration doubles the
                        // effective filter footprint via the à-trous trick.
                        ui.label("Iterations:");
                        ui.horizontal(|ui| {
                            let resp = ui.add_enabled(
                                self.render_3d_opts.pt_denoise_enabled,
                                egui::Slider::new(
                                    &mut self.render_3d_opts.pt_denoise_iterations,
                                    1..=5,
                                )
                                .clamping(egui::SliderClamping::Always)
                                .text("passes"),
                            );
                            if resp.changed() {
                                *changed = true;
                            }
                            resp.on_hover_text(
                                "More iterations = larger smoothing radius (1, 2, 4, 8, 16 \
                                 pixels). 3 is a good default; 5 for very low-spp renders.",
                            );
                        });
                        ui.end_row();

                        // Color edge-stop sigma. Smaller = more smoothing, less
                        // edge preservation.
                        ui.label("Color sigma:");
                        ui.horizontal(|ui| {
                            let resp = ui.add_enabled(
                                self.render_3d_opts.pt_denoise_enabled,
                                egui::Slider::new(
                                    &mut self.render_3d_opts.pt_denoise_sigma_color,
                                    0.05..=2.0,
                                )
                                .clamping(egui::SliderClamping::Always)
                                .logarithmic(true)
                                .text("σ_c"),
                            );
                            if resp.changed() {
                                *changed = true;
                            }
                            resp.on_hover_text(
                                "Edge-stop strength: lower = more smoothing (less edge \
                                 preservation). 0.3 is a good starting point. \
                                 Bump up if denoised images look blurry; reduce if noise \
                                 still leaks through.",
                            );
                        });
                        ui.end_row();
                    });

                ui.add_space(8.0);

                // Quick-apply preset buttons.
                ui.horizontal(|ui| {
                    ui.label("Presets:");
                    if ui.button("Conservative").on_hover_text(
                        "Light smoothing — preserves detail but leaves some noise.",
                    ).clicked() {
                        self.render_3d_opts.pt_denoise_enabled = true;
                        self.render_3d_opts.pt_denoise_iterations = 2;
                        self.render_3d_opts.pt_denoise_sigma_color = 0.6;
                        *changed = true;
                    }
                    if ui.button("Balanced").on_hover_text(
                        "Default — good visual quality at low spp.",
                    ).clicked() {
                        self.render_3d_opts.pt_denoise_enabled = true;
                        self.render_3d_opts.pt_denoise_iterations = 3;
                        self.render_3d_opts.pt_denoise_sigma_color = 0.3;
                        *changed = true;
                    }
                    if ui.button("Aggressive").on_hover_text(
                        "Maximum smoothing — for very-low-spp interactive previews. \
                         Loses fine detail.",
                    ).clicked() {
                        self.render_3d_opts.pt_denoise_enabled = true;
                        self.render_3d_opts.pt_denoise_iterations = 5;
                        self.render_3d_opts.pt_denoise_sigma_color = 0.15;
                        *changed = true;
                    }
                    if ui.button("Off").clicked() {
                        self.render_3d_opts.pt_denoise_enabled = false;
                        *changed = true;
                    }
                });

                ui.add_space(8.0);

                // Architectural note for the user.
                ui.label(
                    egui::RichText::new(
                        "MVP: color-only edge stopping. Future versions will add \
                         normal/depth guidance from the wavefront PT's G-buffer for \
                         better edge preservation.",
                    )
                    .small()
                    .weak(),
                );

                ui.label(
                    egui::RichText::new(
                        "Tip: the denoiser only runs when PT is enabled. With \
                         path_tracing=false, this section is dormant.",
                    )
                    .small()
                    .weak(),
                );
            });
    }
}
