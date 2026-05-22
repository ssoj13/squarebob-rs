//! Colour pipeline section — sits between Render and Samples.
//!
//! Controls the display-side colour chain: working space, tonemap
//! kind, ACES Full transforms (IDT / LMT / RRT / ODT), exposure, white
//! balance, and gamut compression. State lives on `Render3DOptions` as
//! the `color_*` fields and round-trips through presets.
//!
//! Phase C-1: UI + state + preset autosave only. The actual GPU lane
//! still uses the existing blit-shader ACES Filmic path, so default
//! settings produce a bit-exact image. Phases C-2 / C-3 / C-4 wire the
//! new lanes to the blit shader and vfx-rs matrices — see
//! `docs/aces-color-pipeline-plan.md`.

use super::{settings_grid, tinted_section, SettingsDirty};
use color_pipeline::{BuiltInTonemap, ColorCodepath, ColorMode, ConfigSource};
use crate::app::App;
use eframe::egui;
use render_shared::{AcesIdt, AcesLmt, AcesOdt, AcesRrt, ColorWorkingSpace, TonemapKind};

/// Whether the active ODT targets an HDR display. Used to surface a
/// surface-format warning in the status line — the eframe-managed
/// swapchain is always Rgba8UnormSrgb today, so HDR ODTs produce
/// mathematically correct codewords that get destroyed by the 8-bit
/// framebuffer. Will go away once TaskList #8 (wgpu surface
/// negotiation) lands.
fn odt_targets_hdr(odt: AcesOdt) -> bool {
    matches!(odt, AcesOdt::Rec2020_1000nits | AcesOdt::SrgbHdrSim)
}

/// Row label with an attached hover tooltip explaining the stage.
/// Used instead of bare `control_label` so every row in the Color
/// section spells out what the parameter does without forcing the
/// user to mouse over each individual dropdown item.
fn labeled_row(ui: &mut egui::Ui, label: &'static str, tooltip: &'static str) {
    ui.label(label).on_hover_text(tooltip);
}

impl App {
    /// Colour pipeline section. Every control here is a display-side
    /// hyper-param — none of them affect cube layout or PT samples, so
    /// they are uniformly marked `dirty.preset()` only. Treemap is not
    /// rebuilt and PT accumulation is not reset (same contract as the
    /// denoiser section).
    pub(super) fn ui_settings_color(&mut self, ui: &mut egui::Ui, dirty: &mut SettingsDirty) {
        tinted_section(
            ui,
            "Color",
            true,
            self.settings_tint_mix,
            self.settings_section_header_height,
            |ui| {
                let aces_full = self.render_3d_opts.color_tonemap == TonemapKind::AcesFull;

                settings_grid(ui, "color_grid", |ui| {
                    // Row 1 — working colour space. Drives the space the
                    // ACES filmic curve runs in (LinearSRGB / AP1 /
                    // AP0). Greyed unless tonemap == AcesFull because
                    // every other tonemap path operates in linear-sRGB
                    // directly and ignores the chosen working space.
                    labeled_row(
                        ui,
                        "Working space:",
                        "Colour space the ACES filmic curve runs in. Only takes \
                         effect when Tonemap = ACES Full. Linear sRGB = no ACES \
                         conversion (curve in sRGB, equivalent to ACES Filmic). \
                         ACEScg (AP1) = canonical ACES 1.x. ACES2065-1 (AP0) = \
                         widest gamut, softer highlight rolloff.",
                    );
                    let ws = &mut self.render_3d_opts.color_working;
                    ui.add_enabled_ui(aces_full, |ui| {
                        egui::ComboBox::from_id_salt("color_working_cb")
                            .width(220.0)
                            .selected_text(working_label(*ws))
                            .show_ui(ui, |ui| {
                                for opt in [
                                    ColorWorkingSpace::LinearSRGB,
                                    ColorWorkingSpace::ACEScg,
                                    ColorWorkingSpace::ACES2065_1,
                                ] {
                                    if ui
                                        .selectable_label(*ws == opt, working_label(opt))
                                        .on_hover_text(working_hover(opt))
                                        .clicked()
                                    {
                                        *ws = opt;
                                        dirty.preset();
                                    }
                                }
                            });
                    });
                    ui.end_row();

                    // Row 2 — tonemap kind. This is the master switch: the
                    // ACES Full lanes below are greyed unless this is set
                    // to `AcesFull`. Default = `AcesFilmic` (current blit
                    // shader behaviour) so flipping the section doesn't
                    // change the rendered image until the user opts in.
                    labeled_row(
                        ui,
                        "Tonemap:",
                        "How HDR light gets squeezed to your monitor's 0–1 range.\n\
                         Path tracer can output any brightness; the monitor can't show > 1.\n\
                         \n\
                         · None — just clamp. Bright emissives become flat-white blobs (debug).\n\
                         · Linear — no curve, highlights blow out the same way.\n\
                         · Reinhard — soft, slightly washed out.\n\
                         · ACES Filmic — single-formula approximation, fast, the default.\n\
                         · ACES Full — full 4-stage cinema pipeline (IDT/LMT/RRT/ODT)\n\
                           with the best highlight roll-off. Unlocks Working space\n\
                           and the four rows below.\n\
                         \n\
                         Recommended: ACES Full for final renders, Filmic for quick scrub.",
                    );
                    let tm = &mut self.render_3d_opts.color_tonemap;
                    egui::ComboBox::from_id_salt("color_tonemap_cb")
                        .width(220.0)
                        .selected_text(tonemap_label(*tm))
                        .show_ui(ui, |ui| {
                            for opt in [
                                TonemapKind::None,
                                TonemapKind::Linear,
                                TonemapKind::Reinhard,
                                TonemapKind::AcesFilmic,
                                TonemapKind::AcesFull,
                            ] {
                                if ui
                                    .selectable_label(*tm == opt, tonemap_label(opt))
                                    .on_hover_text(tonemap_hover(opt))
                                    .clicked()
                                {
                                    *tm = opt;
                                    dirty.preset();
                                }
                            }
                        });
                    ui.end_row();
                });

                // ── ACES Full chain ───────────────────────────────────
                // The four-stage chain is laid out in its own grid below
                // the master switch so it visually reads as a sub-block,
                // not a peer of the tonemap selector. Disabled wholesale
                // when the master mode isn't `AcesFull`.
                ui.add_space(4.0);
                ui.separator();
                ui.label(
                    egui::RichText::new("ACES Full chain")
                        .small()
                        .color(if aces_full {
                            ui.visuals().text_color()
                        } else {
                            ui.visuals().weak_text_color()
                        }),
                );

                ui.add_enabled_ui(aces_full, |ui| {
                    settings_grid(ui, "color_aces_grid", |ui| {
                        // IDT — input to working space (scene-linear → AP1).
                        labeled_row(
                            ui,
                            "IDT:",
                            "Input Device Transform — \"what colour space did the renderer write\".\n\
                             First step of the ACES chain. A 3×3 matrix that converts the\n\
                             PT's output (linear sRGB primaries) into ACES wide-gamut working\n\
                             space (AP1). Wrong IDT = wrong hues throughout the rest of the chain.\n\
                             \n\
                             Recommended: sRGB → AP1 (matches the path tracer).",
                        );
                        let idt = &mut self.render_3d_opts.color_idt;
                        egui::ComboBox::from_id_salt("color_idt_cb")
                            .width(220.0)
                            .selected_text(idt_label(*idt))
                            .show_ui(ui, |ui| {
                                for opt in [
                                    AcesIdt::None,
                                    AcesIdt::SrgbToAp1,
                                    AcesIdt::Rec709ToAp1,
                                    AcesIdt::Ap1Passthrough,
                                ] {
                                    if ui
                                        .selectable_label(*idt == opt, idt_label(opt))
                                        .on_hover_text(idt_hover(opt))
                                        .clicked()
                                    {
                                        *idt = opt;
                                        dirty.preset();
                                    }
                                }
                            });
                        ui.end_row();

                        // LMT — optional creative grade between IDT + RRT.
                        labeled_row(
                            ui,
                            "LMT (look):",
                            "Look Modification Transform — optional creative \"filter\".\n\
                             Runs between IDT and RRT in ACES working space.\n\
                             Think Instagram preset, but inside the color pipeline.\n\
                             \n\
                             · None — no creative grade (honest rendering).\n\
                             · Neutral — gentle +5 % saturation lift.\n\
                             · Punchy — +15 % saturation, cinematic feel.\n\
                             \n\
                             Recommended: None for technical work, Neutral for screenshots.",
                        );
                        let lmt = &mut self.render_3d_opts.color_lmt;
                        egui::ComboBox::from_id_salt("color_lmt_cb")
                            .width(220.0)
                            .selected_text(lmt_label(*lmt))
                            .show_ui(ui, |ui| {
                                for opt in [
                                    AcesLmt::None,
                                    AcesLmt::Neutral,
                                    AcesLmt::Punchy,
                                    AcesLmt::Warm,
                                    AcesLmt::Cool,
                                    AcesLmt::Bleach,
                                    AcesLmt::Vintage,
                                ] {
                                    if ui
                                        .selectable_label(*lmt == opt, lmt_label(opt))
                                        .on_hover_text(lmt_hover(opt))
                                        .clicked()
                                    {
                                        *lmt = opt;
                                        dirty.preset();
                                    }
                                }
                            });
                        ui.end_row();

                        // RRT — Reference Rendering Transform.
                        labeled_row(
                            ui,
                            "RRT:",
                            "Reference Rendering Transform — the HEART of ACES.\n\
                             The famous filmic S-curve that softly rolls off highlights\n\
                             instead of clipping them. Bright emissives become saturated\n\
                             coloured glow rather than flat-white blobs.\n\
                             \n\
                             · Standard — ACES 1.0 reference. The default everyone uses.\n\
                             · RRT.a1.1 — ACES 1.1 update, slightly better highlights.\n\
                             · Off — skip the curve entirely (debug only).\n\
                             \n\
                             Recommended: Standard.",
                        );
                        let rrt = &mut self.render_3d_opts.color_rrt;
                        egui::ComboBox::from_id_salt("color_rrt_cb")
                            .width(220.0)
                            .selected_text(rrt_label(*rrt))
                            .show_ui(ui, |ui| {
                                for opt in [AcesRrt::Standard, AcesRrt::A1_1, AcesRrt::Off] {
                                    if ui
                                        .selectable_label(*rrt == opt, rrt_label(opt))
                                        .on_hover_text(rrt_hover(opt))
                                        .clicked()
                                    {
                                        *rrt = opt;
                                        dirty.preset();
                                    }
                                }
                            });
                        ui.end_row();

                        // ODT — Output Device Transform.
                        labeled_row(
                            ui,
                            "ODT:",
                            "Output Device Transform — \"what monitor are you looking at\".\n\
                             Final step. Converts ACES working space into the colour\n\
                             space your display speaks. Wrong ODT = picture-correct math,\n\
                             but the screen shows it in wrong hues.\n\
                             \n\
                             · sRGB 100 nits — regular SDR monitor / laptop. Default.\n\
                             · Rec.709 — same gamut as sRGB but for video pipelines.\n\
                             · P3-D65 / DCI-P3 — Apple displays, cinema projectors.\n\
                             · Rec.2020 1000 nits — true HDR display (HDR10 / PQ-encoded).\n\
                             · sRGB HDR-Sim — \"how it would look on HDR\" preview on SDR.\n\
                             \n\
                             Recommended: sRGB 100nits unless you have an HDR display.",
                        );
                        let odt = &mut self.render_3d_opts.color_odt;
                        egui::ComboBox::from_id_salt("color_odt_cb")
                            .width(220.0)
                            .selected_text(odt_label(*odt))
                            .show_ui(ui, |ui| {
                                for opt in [
                                    AcesOdt::Srgb100nits,
                                    AcesOdt::Rec709,
                                    AcesOdt::Rec2020_1000nits,
                                    AcesOdt::P3D65,
                                    AcesOdt::DciP3,
                                    AcesOdt::SrgbHdrSim,
                                ] {
                                    if ui
                                        .selectable_label(*odt == opt, odt_label(opt))
                                        .on_hover_text(odt_hover(opt))
                                        .clicked()
                                    {
                                        *odt = opt;
                                        dirty.preset();
                                    }
                                }
                            });
                        ui.end_row();
                    });
                });

                // ── Analog post-controls ──────────────────────────────
                // Exposure / WB / gamut-compress are valid for every
                // tonemap kind (even `None` — they multiply / shift
                // before the curve), so they live outside the
                // `AcesFull`-gated block.
                ui.add_space(4.0);
                ui.separator();
                settings_grid(ui, "color_post_grid", |ui| {
                    labeled_row(
                        ui,
                        "Exposure (EV):",
                        "Display-side exposure in EV stops. Multiplies scene-linear \
                         RGB by 2^ev before tonemap. Independent of the physical-\
                         camera exposure (which is baked into PT integration). \
                         0 = no change; +1 = twice as bright; -1 = half.",
                    );
                    let ev_resp = ui
                        .add(
                            egui::Slider::new(
                                &mut self.render_3d_opts.color_exposure_ev,
                                -8.0..=8.0,
                            )
                            .clamping(egui::SliderClamping::Always)
                            .suffix(" EV"),
                        )
                        .on_hover_text(
                            "Display-side exposure in EV stops. Multiplies scene-linear \
                             RGB by 2^ev before tonemap. 0.0 keeps the image unchanged. \
                             Independent of the physical-camera exposure (which is \
                             baked into PT integration).",
                        );
                    if ev_resp.changed() {
                        dirty.preset();
                    }
                    ui.end_row();

                    labeled_row(
                        ui,
                        "White balance:",
                        "White-point target in Kelvin. 6500 K ≈ D65 (no tint, \
                         reference white). Lower warms the image (tungsten, \
                         sunset). Higher cools it (overcast, shade). Implemented \
                         as a cheap R/B-gain — full Planckian locus is C-5+.",
                    );
                    let wb_resp = ui
                        .add(
                            egui::Slider::new(
                                &mut self.render_3d_opts.color_white_balance_k,
                                3200.0..=10_000.0,
                            )
                            .clamping(egui::SliderClamping::Always)
                            .suffix(" K"),
                        )
                        .on_hover_text(
                            "White-balance target in Kelvin. 6500 K ≈ D65 (no tint). \
                             Lower values warm the image (tungsten/sunset look), higher \
                             cool it (overcast/shade look).",
                        );
                    if wb_resp.changed() {
                        dirty.preset();
                    }
                    ui.end_row();

                    labeled_row(
                        ui,
                        "Gamut compress:",
                        "ACES Reference Gamut Compression — pulls out-of-gamut \
                         samples back inside the display gamut with a soft \
                         rolloff. Useful for Rec.709 / sRGB displays receiving \
                         wide-gamut PT output. Auto = full strength on narrow-\
                         gamut targets, off on wide-gamut. Only active when \
                         Tonemap = ACES Full.",
                    );
                    ui.horizontal(|ui| {
                        let auto = self.render_3d_opts.color_gamut_compress_auto;
                        let gc_resp = ui
                            .add_enabled(
                                !auto,
                                egui::Slider::new(
                                    &mut self.render_3d_opts.color_gamut_compress,
                                    0.0..=1.0,
                                )
                                .clamping(egui::SliderClamping::Always),
                            )
                            .on_hover_text(
                                "Pulls out-of-gamut samples back inside the target ODT \
                                 gamut with a soft rolloff. 0.0 disables. Useful for \
                                 Rec.709 / sRGB displays receiving wide-gamut PT output. \
                                 Greyed when 'Auto' is on.",
                            );
                        if gc_resp.changed() {
                            dirty.preset();
                        }
                        let auto_resp = ui
                            .checkbox(
                                &mut self.render_3d_opts.color_gamut_compress_auto,
                                "Auto",
                            )
                            .on_hover_text(
                                "Apply the gamut compressor at the strength implied by \
                                 the ODT (Rec.709 / sRGB only). Overrides the manual \
                                 slider when on.",
                            );
                        if auto_resp.changed() {
                            dirty.preset();
                        }
                    });
                    ui.end_row();
                });

                ui.add_space(6.0);

                // Status line — green when ACES Full is active, weak
                // otherwise. Frame-budget number is a placeholder until
                // C-2 wires actual timing.
                let visuals = ui.visuals().clone();
                let (status_color, status_text): (egui::Color32, String) = match self
                    .render_3d_opts
                    .color_tonemap
                {
                    TonemapKind::AcesFull => (
                        egui::Color32::from_rgb(140, 200, 140),
                        format!(
                            "ACES Full @ {} → {}",
                            working_label(self.render_3d_opts.color_working),
                            odt_label(self.render_3d_opts.color_odt),
                        ),
                    ),
                    TonemapKind::AcesFilmic => (
                        visuals.text_color(),
                        "ACES Filmic (Narkowicz fit)".to_string(),
                    ),
                    TonemapKind::Reinhard => {
                        (visuals.text_color(), "Reinhard x/(1+x)".to_string())
                    }
                    TonemapKind::Linear => {
                        (visuals.weak_text_color(), "Linear (no curve)".to_string())
                    }
                    TonemapKind::None => (
                        visuals.weak_text_color(),
                        "Bypass (clamp only)".to_string(),
                    ),
                };
                ui.horizontal(|ui| {
                    ui.colored_label(status_color, "●");
                    ui.label(egui::RichText::new(status_text).small());
                });

                ui.label(
                    egui::RichText::new(
                        "ACES Full applies IDT → filmic RRT → ODT matrices on GPU. \
                         LMT and IDT lanes round-trip through presets.",
                    )
                    .small()
                    .weak(),
                );

                // SDR-surface warning: the eframe swapchain is always
                // Rgba8UnormSrgb today, so picking an HDR ODT produces
                // correct codewords that the framebuffer cannot encode.
                // Will be replaced with the actual surface format once
                // TaskList #8 (wgpu HDR surface negotiation) lands.
                if odt_targets_hdr(self.render_3d_opts.color_odt)
                    && self.render_3d_opts.color_tonemap == TonemapKind::AcesFull
                {
                    ui.label(
                        egui::RichText::new(
                            "⚠ HDR ODT selected, but surface is SDR Rgba8UnormSrgb. \
                             Output will clip — full HDR needs surface negotiation \
                             (TaskList #8).",
                        )
                        .small()
                        .color(egui::Color32::from_rgb(220, 180, 90)),
                    );
                }
            },
        );
    }

    /// New OCIO-backed colour section. Rendered ALONGSIDE the
    /// legacy `ui_settings_color` while the migration is in
    /// flight — that way the user can flip between the two and
    /// spot-check the new pipeline against the matrix-baked
    /// reference. Phase 5 removes the legacy section.
    ///
    /// All widgets here are display-side hyper-params; same
    /// `dirty.preset()` contract as the legacy section.
    pub(super) fn ui_settings_color_v2(
        &mut self,
        ui: &mut egui::Ui,
        dirty: &mut SettingsDirty,
    ) {
        tinted_section(
            ui,
            "Color v2 (OCIO)",
            true,
            self.settings_tint_mix,
            self.settings_section_header_height,
            |ui| {
                let cp = &mut self.render_3d_opts.color_pipeline;

                settings_grid(ui, "color_v2_top_grid", |ui| {
                    // Top-level toggle: Built-in vs OCIO.
                    ui.label("Mode:")
                        .on_hover_text(
                            "Built-in = shader-side tonemap, no colour management.\n\
                             Ocio    = full vfx-ocio pipeline (Config + Processor +\n\
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

                    // Codepath: CPU (Processor::apply_rgb on readback) vs
                    // GPU (baked shader or 3D LUT in the blit pass).
                    ui.label("Codepath:")
                        .on_hover_text(
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
                        // Built-in tonemap selector. Pure shader-side.
                        settings_grid(ui, "color_v2_builtin_grid", |ui| {
                            ui.label("Tonemap:")
                                .on_hover_text(
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
                        settings_grid(ui, "color_v2_ocio_grid", |ui| {
                            // Config source — BuiltIn / Bundled / External.
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

                            // Conditional rows for the active config source.
                            match &mut cp.ocio_config {
                                ConfigSource::BuiltIn => {}
                                ConfigSource::Bundled(name) => {
                                    ui.label("File:").on_hover_text(
                                        "Filename under data/ocio/ (e.g. \
                                         studio-config-v2.ocio).",
                                    );
                                    if ui.text_edit_singleline(name).changed() {
                                        dirty.preset();
                                    }
                                    ui.end_row();
                                }
                                ConfigSource::External(path) => {
                                    ui.label("Path:").on_hover_text(
                                        "Absolute path to a .ocio / .ocioz / .json \
                                         OCIO config.",
                                    );
                                    let mut s = path.display().to_string();
                                    if ui.text_edit_singleline(&mut s).changed() {
                                        *path = std::path::PathBuf::from(&s);
                                        dirty.preset();
                                    }
                                    ui.end_row();
                                }
                            }

                            ui.label("Input:").on_hover_text(
                                "OCIO colour space the path tracer writes into. \
                                 Usually the `scene_linear` role.",
                            );
                            if ui.text_edit_singleline(&mut cp.ocio_input_space).changed() {
                                dirty.preset();
                            }
                            ui.end_row();

                            ui.label("Display:").on_hover_text(
                                "Display device from the OCIO config — sRGB / \
                                 Rec.709 / Rec.2020 / P3 / DCI etc.",
                            );
                            if ui.text_edit_singleline(&mut cp.ocio_display).changed() {
                                dirty.preset();
                            }
                            ui.end_row();

                            ui.label("View:").on_hover_text(
                                "View transform for the selected Display. \
                                 Common: 'ACES 1.0 SDR-video', 'Raw', \
                                 'Un-tone-mapped'.",
                            );
                            if ui.text_edit_singleline(&mut cp.ocio_view).changed() {
                                dirty.preset();
                            }
                            ui.end_row();

                            ui.label("Look:").on_hover_text(
                                "Optional named look from the config. Leave empty \
                                 for no look. Custom LUTs go in the row below.",
                            );
                            let mut look_str = cp.ocio_look.clone().unwrap_or_default();
                            if ui.text_edit_singleline(&mut look_str).changed() {
                                cp.ocio_look = if look_str.trim().is_empty() {
                                    None
                                } else {
                                    Some(look_str)
                                };
                                dirty.preset();
                            }
                            ui.end_row();

                            ui.label("Custom LUT:").on_hover_text(
                                "Optional user LUT file (.cube / .3dl / .spi1d / \
                                 .spi3d / .csp). Applied AFTER the display/view \
                                 chain.",
                            );
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
                            ui.end_row();
                        });

                        ui.add_space(4.0);
                        ui.label(
                            egui::RichText::new(
                                "Note: text-edit fields are Phase 4 wireframe. \
                                 Phase 7 replaces them with dropdowns populated \
                                 from the loaded Config.",
                            )
                            .small()
                            .color(ui.visuals().weak_text_color()),
                        );
                    }
                }
            },
        );
    }
}

fn working_label(w: ColorWorkingSpace) -> &'static str {
    match w {
        ColorWorkingSpace::LinearSRGB => "Linear sRGB",
        ColorWorkingSpace::ACEScg => "ACEScg (AP1)",
        ColorWorkingSpace::ACES2065_1 => "ACES2065-1 (AP0)",
    }
}

fn working_hover(w: ColorWorkingSpace) -> &'static str {
    match w {
        ColorWorkingSpace::LinearSRGB => {
            "Plain linear-sRGB. Today PT writes in this space directly — \
             no IDT applied. Recommended until the ACES pipeline ships."
        }
        ColorWorkingSpace::ACEScg => {
            "ACEScg (AP1 primaries, linear). Wide-gamut working space — \
             standard for VFX compositing. Forward-looking knob; engaged \
             once C-3 wires the IDT into PT integration."
        }
        ColorWorkingSpace::ACES2065_1 => {
            "ACES2065-1 (AP0 primaries, linear). Archival/interchange \
             space. Wider gamut than ACEScg but worse for shading math."
        }
    }
}

fn tonemap_label(t: TonemapKind) -> &'static str {
    match t {
        TonemapKind::None => "None (clamp)",
        TonemapKind::Linear => "Linear",
        TonemapKind::Reinhard => "Reinhard",
        TonemapKind::AcesFilmic => "ACES Filmic",
        TonemapKind::AcesFull => "ACES Full",
    }
}

fn tonemap_hover(t: TonemapKind) -> &'static str {
    match t {
        TonemapKind::None => "No curve. Clamp scene-linear RGB to [0,1]. Useful for AOV inspection.",
        TonemapKind::Linear => {
            "Exposure + white-balance only, no curve. Highlights blow out — debug-only."
        }
        TonemapKind::Reinhard => "Legacy `x / (1 + x)` curve. Soft rolloff, washed-out highlights.",
        TonemapKind::AcesFilmic => {
            "Narkowicz fit of the ACES filmic curve. Single-shader, no IDT/ODT — \
             matches the current blit-shader behaviour exactly. Default."
        }
        TonemapKind::AcesFull => {
            "Full ACES chain via IDT / LMT / RRT / ODT. Unlocks the dropdowns below. \
             Backed by vfx-rs matrices (C-3+). Today still falls back to ACES Filmic \
             at the GPU lane while C-2/C-3 land."
        }
    }
}

fn idt_label(i: AcesIdt) -> &'static str {
    match i {
        AcesIdt::None => "None (passthrough)",
        AcesIdt::SrgbToAp1 => "sRGB → AP1",
        AcesIdt::Rec709ToAp1 => "Rec.709 → AP1",
        AcesIdt::Ap1Passthrough => "AP1 (already ACEScg)",
    }
}

fn idt_hover(i: AcesIdt) -> &'static str {
    match i {
        AcesIdt::None => "Skip IDT — feed the working-space matrix raw RGB.",
        AcesIdt::SrgbToAp1 => {
            "Most PT output today: linear sRGB / Rec.709 primaries. Standard ACES IDT \
             from sRGB to AP1 (ACEScg)."
        }
        AcesIdt::Rec709ToAp1 => {
            "Identical primaries to sRGB, but with the Rec.709 transfer (gamma 2.4) \
             assumed at input. Use when feeding from a video pipeline."
        }
        AcesIdt::Ap1Passthrough => {
            "Skip — PT output is already in ACEScg. Reserved for the future when PT \
             integrates in AP1 directly."
        }
    }
}

fn lmt_label(l: AcesLmt) -> &'static str {
    match l {
        AcesLmt::None => "None",
        AcesLmt::Neutral => "Neutral",
        AcesLmt::Punchy => "Punchy",
        AcesLmt::Warm => "Warm",
        AcesLmt::Cool => "Cool",
        AcesLmt::Bleach => "Bleach Bypass",
        AcesLmt::Vintage => "Vintage",
    }
}

fn lmt_hover(l: AcesLmt) -> &'static str {
    match l {
        AcesLmt::None => "No look applied — neutral grade.",
        AcesLmt::Neutral => {
            "+5 % saturation lift. Subtle production-safe bump."
        }
        AcesLmt::Punchy => {
            "+15 % saturation. Cinematic, more colour pop."
        }
        AcesLmt::Warm => {
            "−5 % saturation + warm tint (R↑, B↓). Sunset / candlelight feel."
        }
        AcesLmt::Cool => {
            "−5 % saturation + cool tint (B↑, R↓). Moonlight / night feel."
        }
        AcesLmt::Bleach => {
            "−30 % saturation + slight luma lift. Bleach-bypass / \
             desaturated high-contrast look."
        }
        AcesLmt::Vintage => {
            "−20 % saturation + heavy warm tint + green damp. \
             Pulled-back vintage film look."
        }
    }
}

fn rrt_label(r: AcesRrt) -> &'static str {
    match r {
        AcesRrt::Standard => "Standard (v1.0)",
        AcesRrt::A1_1 => "RRT.a1.1",
        AcesRrt::Off => "Off (debug)",
    }
}

fn rrt_hover(r: AcesRrt) -> &'static str {
    match r {
        AcesRrt::Standard => {
            "Reference RRT v1.0 — the original ACES filmic look. Pairs with any ODT."
        }
        AcesRrt::A1_1 => {
            "RRT.a1.1 — minor adjustments to the highlight roll-off. Production target \
             for ACES 1.1+."
        }
        AcesRrt::Off => {
            "Skip RRT. Useful for verifying IDT + ODT matrices in isolation — never \
             ship with this off."
        }
    }
}

fn odt_label(o: AcesOdt) -> &'static str {
    match o {
        AcesOdt::Srgb100nits => "sRGB / 100 nits",
        AcesOdt::Rec709 => "Rec.709",
        AcesOdt::Rec2020_1000nits => "Rec.2020 / 1000 nits HDR",
        AcesOdt::P3D65 => "P3-D65",
        AcesOdt::DciP3 => "DCI-P3",
        AcesOdt::SrgbHdrSim => "sRGB HDR sim",
    }
}

fn odt_hover(o: AcesOdt) -> &'static str {
    match o {
        AcesOdt::Srgb100nits => "Standard sRGB display, 100 nits peak. Default desktop output.",
        AcesOdt::Rec709 => "Rec.709 video. Same primaries as sRGB, 2.4 gamma. TV/broadcast.",
        AcesOdt::Rec2020_1000nits => {
            "HDR10-style output to a Rec.2020 / 1000 nits display. Requires a real HDR \
             swapchain — C-6 work."
        }
        AcesOdt::P3D65 => {
            "P3 primaries with D65 white. Modern wide-gamut desktops (Apple Display P3, \
             most OLEDs)."
        }
        AcesOdt::DciP3 => {
            "DCI-P3 (D60-ish white, 2.6 gamma). Theatrical projection target."
        }
        AcesOdt::SrgbHdrSim => {
            "Simulate HDR on an SDR sRGB display by squeezing the HDR rolloff into \
             100 nits. Useful for previewing HDR grades without HDR hardware."
        }
    }
}
