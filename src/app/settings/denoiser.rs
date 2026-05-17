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
                    // Row 1 — Mode dropdown. Compact label, full-width
                    // selection text. Tooltip explains which AOVs each mode
                    // consumes and which TZA file ships with it.
                    control_label(ui, "Mode:");
                    let mode = &mut self.render_3d_opts.pt_oidn_mode;
                    egui::ComboBox::from_id_salt("oidn_mode_cb")
                        .width(220.0)
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
                                    .on_hover_text(mode_hover(opt))
                                    .clicked()
                                {
                                    *mode = opt;
                                    *changed = true;
                                }
                            }
                        });
                    ui.end_row();

                    // Row 2 — model size selector. Picks which TZA variant
                    // OIDN tries to load (with automatic fallback to the
                    // base model when a size-specific file isn't present).
                    control_label(ui, "Model size:");
                    let q = &mut self.render_3d_opts.pt_oidn_quality;
                    egui::ComboBox::from_id_salt("oidn_quality_cb")
                        .width(220.0)
                        .selected_text(quality_label(*q))
                        .show_ui(ui, |ui| {
                            for opt in [
                                OidnQualityOption::Small,
                                OidnQualityOption::Base,
                                OidnQualityOption::Large,
                            ] {
                                if ui
                                    .selectable_label(*q == opt, quality_label(opt))
                                    .on_hover_text(quality_hover(opt))
                                    .clicked()
                                {
                                    *q = opt;
                                    *changed = true;
                                }
                            }
                        });
                    ui.end_row();

                    // Row 3 — Auto + Denoise-now together, mirrors the
                    // "Auto SPP / Camera Snap" row in the sampling section.
                    control_label(ui, "Trigger:");
                    ui.horizontal(|ui| {
                        let auto_resp = ui
                            .checkbox(&mut self.render_3d_opts.pt_oidn_auto, "Auto")
                            .on_hover_text(
                                "Run OIDN automatically once accumulation reaches \
                                 the global Samples target, AND every N samples \
                                 during accumulation (see Interval below). \
                                 Off → only the manual button fires.",
                            );
                        if auto_resp.changed() {
                            *changed = true;
                        }
                        let off = self.render_3d_opts.pt_oidn_mode == OidnModeOption::Off;
                        let btn = ui
                            .add_enabled(!off, egui::Button::new("Denoise now"))
                            .on_hover_text(
                                "Force a single OIDN pass on the current PT \
                                 accumulator. Latency depends on resolution and \
                                 quality preset (~300 ms at 1080p, balanced).",
                            );
                        if btn.clicked() {
                            self.oidn_run_requested = true;
                        }
                    });
                    ui.end_row();

                    // Row 4 — Periodic re-run interval. 0 disables the
                    // periodic fire and leaves only the final-spp trigger.
                    // Quick-pick buttons sit inline so the user can jump
                    // to common cadences without dragging the value.
                    control_label(ui, "Interval:");
                    ui.horizontal(|ui| {
                        let interval_resp = ui
                            .add(
                                egui::DragValue::new(&mut self.render_3d_opts.pt_oidn_interval)
                                    .range(0..=10_000)
                                    .speed(1)
                                    .suffix(" spp"),
                            )
                            .on_hover_text(
                                "Re-run OIDN every N accumulated samples during the \
                                 render, on top of the final-spp fire. 0 disables \
                                 periodic re-runs. Default 128 — gives smoothed \
                                 intermediate previews without hammering inference.",
                            );
                        if interval_resp.changed() {
                            *changed = true;
                        }
                        for preset in [32_u32, 64, 128, 256, 512, 1024] {
                            let selected = self.render_3d_opts.pt_oidn_interval == preset;
                            if ui
                                .selectable_label(selected, preset.to_string())
                                .on_hover_text(format!("Set interval to {preset} spp"))
                                .clicked()
                            {
                                self.render_3d_opts.pt_oidn_interval = preset;
                                *changed = true;
                            }
                        }
                    });
                    ui.end_row();

                    // Row 5 — HDR firefly clamp on the OIDN input. Caps
                    // luminance before the UNet sees it; the raw PT
                    // accumulator stays unclamped. `0.0` = off.
                    control_label(ui, "Clamp:");
                    ui.horizontal(|ui| {
                        let clamp_resp = ui
                            .add(
                                egui::Slider::new(
                                    &mut self.render_3d_opts.pt_oidn_clamp,
                                    0.0..=100.0,
                                )
                                .logarithmic(true)
                                .clamping(egui::SliderClamping::Never),
                            )
                            .on_hover_text(
                                "Luminance-preserving HDR clamp on the \
                                 OIDN colour input. Hue-preserving \
                                 (scales all 3 channels by the same \
                                 factor). 10.0 is the production \
                                 default; set to 0 to disable. Doesn't \
                                 touch the raw PT accumulator.",
                            );
                        if clamp_resp.changed() {
                            *changed = true;
                        }
                        let adaptive_resp = ui
                            .checkbox(
                                &mut self.render_3d_opts.pt_oidn_adaptive_clamp,
                                "Adaptive",
                            )
                            .on_hover_text(
                                "Smoothly tighten the clamp ceiling at low \
                                 sample counts (smooth-step 0..256 spp), \
                                 then relax to the slider value. Suppresses \
                                 halos around lights in early previews \
                                 without robbing the converged image of \
                                 dynamic range.",
                            );
                        if adaptive_resp.changed() {
                            *changed = true;
                        }
                    });
                    ui.end_row();

                    // Row 6 — NaN/Inf protect. Mirrors reference C++
                    // `nan_to_zero` pre-step in every input kernel.
                    control_label(ui, "NaN protect:");
                    let nan_resp = ui
                        .checkbox(
                            &mut self.render_3d_opts.pt_oidn_nan_protect,
                            "",
                        )
                        .on_hover_text(
                            "Replace non-finite (NaN / ±Inf) samples on \
                             colour / albedo / normal inputs with 0 \
                             before clamp + transfer. Matches the \
                             reference C++ OIDN contract. Without this, \
                             a single bad path-tracer sample can poison \
                             the entire denoised output through the \
                             PU/exp expansion in the inverse transfer.",
                        );
                    if nan_resp.changed() {
                        *changed = true;
                    }
                    ui.end_row();
                });

                ui.add_space(6.0);

                // Status line — coloured by state. Active denoise display →
                // green; pending auto-fire → amber; idle → weak text.
                let visuals = ui.visuals().clone();
                let (status_color, status_text): (egui::Color32, String) =
                    if self.render_3d_opts.pt_oidn_mode == OidnModeOption::Off {
                        (visuals.weak_text_color(), "Disabled".to_string())
                    } else if self.oidn_display_is_denoised {
                        (
                            egui::Color32::from_rgb(140, 200, 140),
                            format!(
                                "Denoised{}",
                                self.oidn_last_latency_ms
                                    .map(|ms| format!(" ({:.0} ms)", ms))
                                    .unwrap_or_default()
                            ),
                        )
                    } else if self.render_3d_opts.pt_oidn_auto {
                        (
                            visuals.text_color(),
                            "Waiting for target Samples".to_string(),
                        )
                    } else {
                        (visuals.weak_text_color(), "Manual mode".to_string())
                    };
                ui.horizontal(|ui| {
                    ui.colored_label(status_color, "●");
                    ui.label(egui::RichText::new(status_text).small());
                });

                ui.label(
                    egui::RichText::new(
                        "OIDN runs on the same wgpu device as PT. Weights load \
                         lazily from data/oidn-weights/ on first use.",
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
        OidnModeOption::Color => "Color",
        OidnModeOption::ColorAlbedo => "Color + Albedo",
        OidnModeOption::ColorAlbedoNormal => "Color + Albedo + Normal",
    }
}

fn mode_hover(m: OidnModeOption) -> &'static str {
    match m {
        OidnModeOption::Off => "Show raw PT output without denoising.",
        OidnModeOption::Color => {
            "Color-only model (rt_hdr). No AOV requirements — fastest start."
        }
        OidnModeOption::ColorAlbedo => {
            "Color + albedo model (rt_hdr_alb). Big quality jump over color-only \
             when primary surfaces have textured albedo."
        }
        OidnModeOption::ColorAlbedoNormal => {
            "Color + albedo + normal model (rt_hdr_alb_nrm). Production target — \
             best edge preservation. Falls back to lower mode if AOVs missing."
        }
    }
}

fn quality_label(q: OidnQualityOption) -> &'static str {
    match q {
        OidnQualityOption::Large => "Large",
        OidnQualityOption::Base => "Base",
        OidnQualityOption::Small => "Small",
    }
}

fn quality_hover(q: OidnQualityOption) -> &'static str {
    match q {
        OidnQualityOption::Large => {
            "Load `_large` model where it exists (prefilter and clean-aux \
             variants ship _large). Color-denoise main models have no _large \
             — silently falls back to Base."
        }
        OidnQualityOption::Base => "Default base model. ~1.8 MB per variant.",
        OidnQualityOption::Small => {
            "Load `_small` model — half the parameters, ~600 KB per variant, \
             noticeably faster inference. Quality drop is mild for primary \
             surfaces; visible on subtle indirect-light noise."
        }
    }
}
