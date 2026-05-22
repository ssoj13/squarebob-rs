//! Colour pipeline section — sits between Render and Samples.
//!
//! Drives the new OCIO-backed colour management.
//! Two modes — BuiltIn (shader-side tonemap: None / Linear /
//! Reinhard / AgX) or Ocio (full `vfx-ocio` pipeline). State lives
//! on `Render3DOptions.color_pipeline` as a single
//! `ColorPipelineSettings` struct and round-trips through presets.

use super::{settings_grid, tinted_section, SettingsDirty};
use color_pipeline::{BuiltInTonemap, ColorCodepath, ColorMode, ConfigSource};
use crate::app::App;
use eframe::egui;

impl App {
    /// Colour pipeline section. Every control here is a display-side
    /// hyper-param — none of them affect cube layout or PT samples, so
    /// they are uniformly marked `dirty.preset()` only. Treemap is not
    /// rebuilt and PT accumulation is not reset (same contract as the
    /// denoiser section).
    pub(super) fn ui_settings_color(&mut self, ui: &mut egui::Ui, dirty: &mut SettingsDirty) {
        // Keep the live `ColorPipeline` in sync with the settings
        // BEFORE we sample any dropdown lists from it. `ensure` is
        // a hash-compare noop when nothing changed.
        let _ = self.color_pipeline.ensure(&self.render_3d_opts.color_pipeline);

        // Snapshot the dropdown contents up front. This is the
        // only spot we can read `&self.color_pipeline` because the
        // tinted-section closure below takes `&mut self` through
        // `cp` and that re-borrow would block live config access.
        let input_spaces = self.color_pipeline.available_input_spaces();
        let displays = self.color_pipeline.available_displays();
        let views_for_current =
            self.color_pipeline.available_views(&self.render_3d_opts.color_pipeline.ocio_display);
        let looks = self.color_pipeline.available_looks();

        tinted_section(
            ui,
            "Color",
            true,
            self.settings_tint_mix,
            self.settings_section_header_height,
            |ui| {
                let cp = &mut self.render_3d_opts.color_pipeline;

                settings_grid(ui, "color_top_grid", |ui| {
                    ui.label("Mode:").on_hover_text(
                        "Built-in = shader-side tonemap, no colour management.\n\
                         OCIO    = full vfx-ocio pipeline (Config + Processor +\n\
                                   Display + View + Look + optional LUT).",
                    );
                    ui.horizontal(|ui| {
                        if ui
                            .selectable_label(cp.mode == ColorMode::BuiltIn, "Built-in")
                            .on_hover_text("No colour management. Just a curve.")
                            .clicked()
                        {
                            cp.mode = ColorMode::BuiltIn;
                            dirty.preset();
                        }
                        if ui
                            .selectable_label(cp.mode == ColorMode::Ocio, "OCIO")
                            .on_hover_text(
                                "OpenColorIO pipeline. Picks displays / views /\n\
                                 looks from the loaded config.",
                            )
                            .clicked()
                        {
                            cp.mode = ColorMode::Ocio;
                            dirty.preset();
                        }
                    });
                    ui.end_row();

                    ui.label("Codepath:").on_hover_text(
                        "CPU = run vfx-ocio's Processor::apply_rgb on the\n\
                               readback buffer. Slow but bit-exact reference.\n\
                         GPU = bake to a 3D LUT / WGSL stub on the blit pass.\n\
                               Fast — default for normal use.",
                    );
                    ui.horizontal(|ui| {
                        if ui
                            .selectable_label(cp.codepath == ColorCodepath::Gpu, "GPU")
                            .clicked()
                        {
                            cp.codepath = ColorCodepath::Gpu;
                            dirty.preset();
                        }
                        if ui
                            .selectable_label(cp.codepath == ColorCodepath::Cpu, "CPU")
                            .clicked()
                        {
                            cp.codepath = ColorCodepath::Cpu;
                            dirty.preset();
                        }
                    });
                    ui.end_row();
                });

                ui.add_space(4.0);
                ui.separator();

                match cp.mode {
                    ColorMode::BuiltIn => {
                        settings_grid(ui, "color_builtin_grid", |ui| {
                            ui.label("Tonemap:").on_hover_text(
                                "Built-in curve applied to scene-linear HDR.\n\
                                 None    — clamp only (debug).\n\
                                 Linear  — no curve, highlights blow out.\n\
                                 Reinhard — c / (1 + c), washed but cheap.\n\
                                 AgX     — modern open filmic, hue-preserving.",
                            );
                            ui.horizontal(|ui| {
                                for (variant, label) in [
                                    (BuiltInTonemap::None, "None"),
                                    (BuiltInTonemap::Linear, "Linear"),
                                    (BuiltInTonemap::Reinhard, "Reinhard"),
                                    (BuiltInTonemap::AgX, "AgX"),
                                ] {
                                    if ui
                                        .selectable_label(cp.builtin == variant, label)
                                        .clicked()
                                    {
                                        cp.builtin = variant;
                                        dirty.preset();
                                    }
                                }
                            });
                            ui.end_row();
                        });
                    }
                    ColorMode::Ocio => {
                        settings_grid(ui, "color_ocio_grid", |ui| {
                            ui.label("Config:").on_hover_text(
                                "Where the OCIO Config comes from.\n\
                                 BuiltIn  — vfx-ocio's bundled ACES 1.3 (no file).\n\
                                 Bundled  — a .ocio shipped under data/ocio/.\n\
                                 External — user-loaded .ocio / .ocioz / .json.",
                            );
                            ui.horizontal(|ui| {
                                let is_builtin = matches!(cp.ocio_config, ConfigSource::BuiltIn);
                                let is_bundled =
                                    matches!(cp.ocio_config, ConfigSource::Bundled(_));
                                let is_external =
                                    matches!(cp.ocio_config, ConfigSource::External(_));
                                if ui.selectable_label(is_builtin, "BuiltIn").clicked() {
                                    cp.ocio_config = ConfigSource::BuiltIn;
                                    dirty.preset();
                                }
                                if ui.selectable_label(is_bundled, "Bundled").clicked() {
                                    cp.ocio_config = ConfigSource::Bundled(String::new());
                                    dirty.preset();
                                }
                                if ui.selectable_label(is_external, "External").clicked() {
                                    cp.ocio_config = ConfigSource::External(Default::default());
                                    dirty.preset();
                                }
                            });
                            ui.end_row();

                            match &mut cp.ocio_config {
                                ConfigSource::BuiltIn => {}
                                ConfigSource::Bundled(name) => {
                                    ui.label("File:").on_hover_text(
                                        "An OCIO config shipped under data/ocio/. \
                                         Populate with `python bootstrap.py d`.",
                                    );
                                    let bundled = color_pipeline::available_bundled_configs();
                                    if bundled.is_empty() {
                                        ui.label(
                                            egui::RichText::new("no configs in data/ocio/")
                                                .color(ui.visuals().warn_fg_color),
                                        )
                                        .on_hover_text(
                                            "Run `python bootstrap.py d` to fetch the \
                                             pinned ACES Studio + CG configs.",
                                        );
                                    } else {
                                        ocio_dropdown(
                                            ui,
                                            "color_bundled_cb",
                                            name,
                                            &bundled,
                                            false,
                                            dirty,
                                        );
                                    }
                                    ui.end_row();
                                }
                                ConfigSource::External(path) => {
                                    ui.label("Path:").on_hover_text(
                                        "Absolute path to a .ocio / .ocioz / .json \
                                         OCIO config.",
                                    );
                                    ui.horizontal(|ui| {
                                        let mut s = path.display().to_string();
                                        if ui.text_edit_singleline(&mut s).changed() {
                                            *path = std::path::PathBuf::from(&s);
                                            dirty.preset();
                                        }
                                        if ui
                                            .button("Browse…")
                                            .on_hover_text(
                                                "Pick an OCIO config (.ocio / .ocioz / .json).",
                                            )
                                            .clicked()
                                            && let Some(picked) = rfd_pick_ocio_config()
                                        {
                                            *path = picked;
                                            dirty.preset();
                                        }
                                    });
                                    ui.end_row();
                                }
                            }

                            ui.label("Input:").on_hover_text(
                                "OCIO colour space the path tracer writes into. \
                                 Usually the `scene_linear` role.",
                            );
                            ocio_dropdown(
                                ui,
                                "color_input_cb",
                                &mut cp.ocio_input_space,
                                &input_spaces,
                                false,
                                dirty,
                            );
                            ui.end_row();

                            ui.label("Display:").on_hover_text(
                                "Display device from the OCIO config — sRGB / \
                                 Rec.709 / Rec.2020 / P3 / DCI etc.",
                            );
                            ocio_dropdown(
                                ui,
                                "color_display_cb",
                                &mut cp.ocio_display,
                                &displays,
                                false,
                                dirty,
                            );
                            ui.end_row();

                            ui.label("View:").on_hover_text(
                                "View transform for the selected Display. \
                                 Common: 'ACES 1.0 SDR-video', 'Raw', \
                                 'Un-tone-mapped'.",
                            );
                            ocio_dropdown(
                                ui,
                                "color_view_cb",
                                &mut cp.ocio_view,
                                &views_for_current,
                                false,
                                dirty,
                            );
                            ui.end_row();

                            ui.label("Look:").on_hover_text(
                                "Optional named look from the config. \"(none)\" \
                                 disables the look slot. Custom LUTs go in the \
                                 row below.",
                            );
                            let mut look_str = cp.ocio_look.clone().unwrap_or_default();
                            let look_changed = ocio_dropdown(
                                ui,
                                "color_look_cb",
                                &mut look_str,
                                &looks,
                                true,
                                dirty,
                            );
                            if look_changed {
                                cp.ocio_look = if look_str.trim().is_empty() {
                                    None
                                } else {
                                    Some(look_str)
                                };
                            }
                            ui.end_row();

                            ui.label("Custom LUT:").on_hover_text(
                                "Optional user LUT file (.cube / .3dl / .spi1d / \
                                 .spi3d / .csp). Applied AFTER the display/view \
                                 chain.",
                            );
                            ui.horizontal(|ui| {
                                let mut lut_str = cp
                                    .ocio_custom_lut
                                    .as_ref()
                                    .map(|p| p.display().to_string())
                                    .unwrap_or_default();
                                if ui.text_edit_singleline(&mut lut_str).changed() {
                                    cp.ocio_custom_lut = if lut_str.trim().is_empty() {
                                        None
                                    } else {
                                        Some(std::path::PathBuf::from(&lut_str))
                                    };
                                    dirty.preset();
                                }
                                if ui
                                    .button("Browse…")
                                    .on_hover_text(
                                        "Pick a LUT file (.cube / .3dl / .spi1d / .spi3d / .csp).",
                                    )
                                    .clicked()
                                    && let Some(picked) = rfd_pick_lut_file()
                                {
                                    cp.ocio_custom_lut = Some(picked);
                                    dirty.preset();
                                }
                                if cp.ocio_custom_lut.is_some()
                                    && ui
                                        .button("Clear")
                                        .on_hover_text("Drop the custom LUT slot.")
                                        .clicked()
                                {
                                    cp.ocio_custom_lut = None;
                                    dirty.preset();
                                }
                            });
                            ui.end_row();
                        });
                    }
                }
            },
        );
    }
}

/// rfd-driven open dialog for an OCIO config file. Filters cover
/// the three formats `vfx_ocio::Config::from_file` understands.
fn rfd_pick_ocio_config() -> Option<std::path::PathBuf> {
    rfd::FileDialog::new()
        .add_filter("OCIO config", &["ocio", "ocioz", "json"])
        .add_filter("All files", &["*"])
        .pick_file()
}

/// rfd-driven open dialog for a LUT file. The five extensions
/// match the LUT formats vfx-ocio's `FileTransform` accepts.
fn rfd_pick_lut_file() -> Option<std::path::PathBuf> {
    rfd::FileDialog::new()
        .add_filter("LUT", &["cube", "3dl", "spi1d", "spi3d", "csp"])
        .add_filter("All files", &["*"])
        .pick_file()
}

/// Dropdown for OCIO-introspected lists (input spaces / displays /
/// views / looks). Renders a `ComboBox` populated from `options`
/// plus an optional empty `(none)` entry when `allow_none` is true.
/// When the user picks a different option the new value is written
/// back into `current` and `dirty.preset()` fires. Returns true on
/// any actual change so the caller can do follow-up work (e.g. wrap
/// the value in `Option<String>` for the look slot).
///
/// The current value is shown in the closed selector even if it
/// isn't in `options` — that way a stale preset (config swapped
/// after the preset was saved) still displays the chosen name,
/// flagged with a `?` prefix, instead of silently dropping back
/// to the first available entry.
fn ocio_dropdown(
    ui: &mut egui::Ui,
    id_salt: &str,
    current: &mut String,
    options: &[String],
    allow_none: bool,
    dirty: &mut SettingsDirty,
) -> bool {
    let display = if current.trim().is_empty() && allow_none {
        "(none)".to_string()
    } else if options.iter().any(|o| o == current) {
        current.clone()
    } else if current.trim().is_empty() {
        "(empty)".to_string()
    } else {
        format!("? {current}")
    };
    let mut changed = false;
    egui::ComboBox::from_id_salt(id_salt)
        .width(220.0)
        .selected_text(display)
        .show_ui(ui, |ui| {
            if allow_none {
                let is_none = current.trim().is_empty();
                if ui.selectable_label(is_none, "(none)").clicked() && !is_none {
                    current.clear();
                    dirty.preset();
                    changed = true;
                }
            }
            for opt in options {
                let selected = opt == current;
                if ui.selectable_label(selected, opt).clicked() && !selected {
                    *current = opt.clone();
                    dirty.preset();
                    changed = true;
                }
            }
        });
    changed
}
