//! OIDN denoiser settings — replaces the previous à-trous panel.
//!
//! Controls live under Settings → Rendering. State lives on `Render3DOptions`
//! as `pt_oidn_mode` (Off / Color / Color+Albedo / Color+Albedo+Normal),
//! `pt_oidn_quality` (High / Balanced / Fast), and `pt_oidn_auto` (run
//! denoiser automatically once `current_spp >= target_spp`). A manual
//! "Denoise now" button forces a single pass regardless of `auto`.

use super::{control_label, settings_grid, tinted_section};
use crate::app::App;
use eframe::egui;
use render_shared::{OidnModeOption, OidnQualityOption};

impl App {
    pub(super) fn ui_settings_denoiser(&mut self, ui: &mut egui::Ui, changed: &mut bool) {
        tinted_section(
            ui,
            "Denoiser (OIDN)",
            true,
            self.settings_tint_mix,
            self.settings_section_header_height,
            |ui| {
                settings_grid(ui, "oidn_grid", |ui| {
                    control_label(ui, "Mode:");
                    let mode = &mut self.render_3d_opts.pt_oidn_mode;
                    egui::ComboBox::from_id_salt("oidn_mode_cb")
                        .selected_text(mode_label(*mode))
                        .show_ui(ui, |ui| {
                            for opt in [
                                OidnModeOption::Off,
                                OidnModeOption::Color,
                                OidnModeOption::ColorAlbedo,
                                OidnModeOption::ColorAlbedoNormal,
                            ] {
                                if ui
                                    .selectable_label(*mode == opt, mode_label(opt))
                                    .clicked()
                                {
                                    *mode = opt;
                                    *changed = true;
                                }
                            }
                        });
                    ui.end_row();

                    control_label(ui, "Quality:");
                    let q = &mut self.render_3d_opts.pt_oidn_quality;
                    egui::ComboBox::from_id_salt("oidn_quality_cb")
                        .selected_text(quality_label(*q))
                        .show_ui(ui, |ui| {
                            for opt in [
                                OidnQualityOption::High,
                                OidnQualityOption::Balanced,
                                OidnQualityOption::Fast,
                            ] {
                                if ui
                                    .selectable_label(*q == opt, quality_label(opt))
                                    .clicked()
                                {
                                    *q = opt;
                                    *changed = true;
                                }
                            }
                        });
                    ui.end_row();

                    control_label(ui, "Auto:");
                    if ui
                        .checkbox(&mut self.render_3d_opts.pt_oidn_auto, "")
                        .on_hover_text(
                            "Run OIDN automatically once the accumulating render reaches \
                             its sample target. Disable to use only the manual button.",
                        )
                        .changed()
                    {
                        *changed = true;
                    }
                    ui.end_row();

                    control_label(ui, "Trigger:");
                    if ui
                        .button("Denoise now")
                        .on_hover_text(
                            "Force a single OIDN pass on the current PT accumulator. \
                             Honors Mode and Quality. Latency depends on resolution and \
                             quality preset (≈300 ms at 1080p, balanced).",
                        )
                        .clicked()
                    {
                        self.oidn_run_requested = true;
                    }
                    ui.end_row();
                });

                ui.add_space(8.0);

                if let Some(ms) = self.oidn_last_latency_ms {
                    ui.label(
                        egui::RichText::new(format!(
                            "Last denoise: {:.1} ms",
                            ms
                        ))
                        .small(),
                    );
                }

                ui.label(
                    egui::RichText::new(
                        "OIDN replaces the legacy à-trous filter with a U-Net trained \
                         by Intel. Model picks scale with Mode + Quality. Weights live \
                         in data/oidn-weights/. Runs on the same wgpu device as PT.",
                    )
                    .small()
                    .weak(),
                );
            },
        );
    }
}

fn mode_label(m: OidnModeOption) -> &'static str {
    match m {
        OidnModeOption::Off => "Off",
        OidnModeOption::Color => "Color (rt_hdr)",
        OidnModeOption::ColorAlbedo => "Color + Albedo (rt_hdr_alb)",
        OidnModeOption::ColorAlbedoNormal => "Color + Albedo + Normal (rt_hdr_alb_nrm)",
    }
}

fn quality_label(q: OidnQualityOption) -> &'static str {
    match q {
        OidnQualityOption::High => "High",
        OidnQualityOption::Balanced => "Balanced",
        OidnQualityOption::Fast => "Fast (small models)",
    }
}
