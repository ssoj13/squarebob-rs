//! Shared egui controls for `RampParams` / `CurveParams`.
//!
//! `ramp_rows` and friends are wired up incrementally as Color / Folder
//! migrations land. Allow dead_code until then.

#![allow(dead_code)]
//!
//! Each helper emits rows that slot into the surrounding `settings_grid`
//! (2-column Grid with fixed label width). Callers stay responsible for
//! the section header and the mode picker — the helpers below only
//! render parameters for a single active mode.

use eframe::egui;
use pt_mats::{MaterialDistribution, Palette};
use render_shared::{CurveParams, RampParams};

use super::{control_label, PT_VALUE_WIDTH};

/// Emit Scale + Scale Exponent rows for a `CurveParams`. Returns `true`
/// when either value changed.
pub fn curve_rows(ui: &mut egui::Ui, params: &mut CurveParams) -> bool {
    let mut changed = false;

    control_label(ui, "Scale");
    if ui
        .add(egui::Slider::new(&mut params.scale, 0.1..=5.0).show_value(true))
        .changed()
    {
        changed = true;
    }
    ui.end_row();

    control_label(ui, "Scale Exponent");
    if ui
        .add(egui::Slider::new(&mut params.exponent, 0.1..=4.0).show_value(true))
        .changed()
    {
        changed = true;
    }
    ui.end_row();

    changed
}

/// Behavioural toggles for [`ramp_rows`]. Hide distribution-specific
/// sub-params when a feature doesn't use them (e.g. Material lights).
#[derive(Debug, Clone, Copy)]
pub struct RampUiCtx {
    /// Show the distribution picker + its conditional sub-params.
    pub with_distribution: bool,
    /// Show the Scale / Scale Exponent rows. Some callers (e.g. raw
    /// gradient where curve doesn't matter) hide them.
    pub with_curve: bool,
    /// Salt for ComboBox ids so multiple ramps on one panel don't alias.
    pub id_salt: &'static str,
}

impl RampUiCtx {
    pub fn full(id_salt: &'static str) -> Self {
        Self {
            with_distribution: true,
            with_curve: true,
            id_salt,
        }
    }

    pub fn compact(id_salt: &'static str) -> Self {
        Self {
            with_distribution: true,
            with_curve: false,
            id_salt,
        }
    }
}

/// Emit Palette + Distribute + sub-params + Scale rows for a
/// [`RampParams`]. Returns `true` when anything changed.
pub fn ramp_rows(ui: &mut egui::Ui, params: &mut RampParams, ctx: RampUiCtx) -> bool {
    let mut changed = false;

    // ---- Palette ----
    control_label(ui, "Palette");
    let cur_label = match params.palette {
        None => "Auto".to_string(),
        Some(p) => p.name().to_string(),
    };
    egui::ComboBox::from_id_salt(format!("ramp_palette_{}", ctx.id_salt))
        .selected_text(cur_label)
        .width(PT_VALUE_WIDTH * 2.0)
        .show_ui(ui, |ui| {
            if ui
                .selectable_value(&mut params.palette, None, "Auto")
                .changed()
            {
                changed = true;
            }
            for &p in Palette::all() {
                if ui
                    .selectable_value(&mut params.palette, Some(p), p.name())
                    .changed()
                {
                    changed = true;
                }
            }
        });
    ui.end_row();

    if ctx.with_distribution {
        control_label(ui, "Distribute");
        ui.horizontal(|ui| {
            for (variant, label) in [
                (MaterialDistribution::Direct, "Direct"),
                (MaterialDistribution::Quantized, "Quant"),
                (MaterialDistribution::Gradient, "Grad"),
                (MaterialDistribution::Spatial, "Spatial"),
                (MaterialDistribution::Bands, "Bands"),
            ] {
                if ui
                    .selectable_value(&mut params.distribution, variant, label)
                    .changed()
                {
                    changed = true;
                }
            }
        });
        ui.end_row();

        // Conditional sub-param row matches the distribution shape so
        // unused knobs stay hidden — keeps the grid compact.
        match params.distribution {
            MaterialDistribution::Quantized => {
                control_label(ui, "Levels");
                if ui
                    .add(egui::Slider::new(&mut params.quant_levels, 2..=14))
                    .changed()
                {
                    changed = true;
                }
                ui.end_row();
            }
            MaterialDistribution::Bands => {
                control_label(ui, "Bands");
                if ui
                    .add(egui::Slider::new(&mut params.band_count, 2..=20))
                    .changed()
                {
                    changed = true;
                }
                ui.end_row();
            }
            MaterialDistribution::Spatial => {
                control_label(ui, "Noise Scale");
                if ui
                    .add(
                        egui::Slider::new(&mut params.spatial_scale, 0.001..=0.1)
                            .logarithmic(true),
                    )
                    .changed()
                {
                    changed = true;
                }
                ui.end_row();
            }
            _ => {}
        }
    }

    if ctx.with_curve && curve_rows(ui, &mut params.curve) {
        changed = true;
    }

    changed
}
