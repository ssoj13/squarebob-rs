//! Renderer settings: 2D backend, 3D options.

use super::tinted_section;
use crate::app::helpers::{multibutton_exclusive, MultiButtonAxis};
use crate::app::App;
use crate::renderer::{
    AdaptivePreset, ColorMode, CubeHeightMode, FolderColorMode, GlassPreset, HashTransformEffect,
    HoverMode, PtSamplerMode, RenderMode, SpectralMode,
};
use eframe::egui;
use pt_mats::{MaterialDistribution, MaterialSource, MaterializeMode, Palette};

/// Maximum absolute PT light/glass cube count in the UI and when persisting settings.
///
/// When `total_cubes == 0` (scan not finished), the drag range must **not** fall back to `0..=1`:
/// [`egui::DragValue`] clamps existing values every frame (`clamp_existing_to_range(true)` default),
/// which was resetting saved counts to **1** on every launch before the tree existed.
const MAX_PT_MAT_CUBE_COUNT: u32 = 5000;

use super::{
    curve_rows, ramp_section, settings_grid, RampUiCtx, PT_VALUE_WIDTH, SETTINGS_LABEL_WIDTH,
};

fn control_label(ui: &mut egui::Ui, label: &'static str) {
    ui.label(label)
        .on_hover_text(renderer_control_tooltip(label.trim_end_matches(':')));
}

fn renderer_control_tooltip(label: &str) -> &'static str {
    match label {
        "Mode" => "Renderer mode used for the current 3D view.",
        "Height" => "Controls which file or tree property drives cube height.",
        "Scale" => "Multiplies the visual strength of this setting.",
        "Color" => "Controls how cubes are colored.",
        "Folder tint" => "Blends file colors with their parent folder color.",
        "LOD" => "Collapses distant/small geometry to reduce GPU work.",
        "Min px" => "Minimum projected size before LOD collapses a subtree.",
        "Effect" => "Hash-based transform effect applied to cubes.",
        "Strength" => "Intensity of the selected transform effect.",
        "Animate" => "Animates the selected transform effect over time.",
        "Slice Plane" => "Cuts the 3D scene with a configurable plane.",
        "Normal" => "Custom slice plane normal vector.",
        "Axis" => "Axis used by the slice plane in axis mode.",
        "Distance" => "Slice plane offset through the scene.",
        "Invert" => "Flips which side of the slice plane remains visible.",
        "Source" => "Data source used to assign material classes.",
        "Distribute" => "How material classes are distributed across cubes.",
        "Levels" => "Number of quantization levels for material assignment.",
        "Bands" => "Number of bands for banded material assignment.",
        "Seed" => "Deterministic seed for randomized material assignment.",
        "Mix" => "Blend between base color and materialized color.",
        "Roughness" => "Surface roughness used by shaded rendering or glass materials.",
        "Metalness" => "Metallic response for shaded rendering.",
        "Specular IOR" => "Specular index-of-refraction approximation for shaded rendering.",
        "Shading" => "Surface shading options for the raster renderer.",
        "Light Cubes" => "How many cubes are assigned emissive light materials.",
        "Warm Bias" => "Biases emissive materials toward warm colors.",
        "Cool Bias" => "Biases emissive materials toward cool colors.",
        "Light Power" => "Global intensity multiplier for emissive cube materials.",
        "Light Rand" => "Randomizes emissive cube colors.",
        "Glass Cubes" => "How many cubes are assigned glass materials.",
        "Env MIS" => "Importance-samples the environment map for lower path tracing noise.",
        "Emissive NEE" => "Directly samples emissive cubes for path tracing lighting.",
        "Light SPP" => "Number of direct-light samples per path tracing step.",
        "Light Min" => "Minimum emissive weight considered for direct light sampling.",
        "Max Samples" => "Target path tracing sample count.",
        "SPP/frame" => "Path tracing samples accumulated per rendered frame.",
        "Auto SPP" => "Automatically adjusts samples per frame toward the target FPS.",
        "Target FPS" => "Frame rate target used by Auto SPP and camera snap.",
        "Sampler" => "Random sequence used by the path tracer.",
        "Adaptive" => "Allocates more samples to noisy pixels.",
        "Preset" => "Preset values for the controls in this section.",
        "SPP Range" => "Minimum and maximum samples used by adaptive sampling.",
        "Variance" => "Noise threshold used by adaptive sampling.",
        "Interval" => "How often adaptive sampling updates its allocation.",
        "Bounces" => "Maximum path tracing bounce count.",
        "Transmission" => "Maximum transparent/refractive bounce depth.",
        "Russian Roulette" => "Probabilistically terminates low-energy paths.",
        "Transparency" => "Global glass/transparency blend for path tracing.",
        "Thin" => "Treats glass as thin surfaces instead of solid volumes.",
        "Specular" => "Specular contribution for glass materials.",
        "Base" => "Base color contribution for glass materials.",
        "IoR" => "Index of refraction for glass transmission.",
        "Dispersion" => "Amount of spectral spread for glass.",
        "Temperature" => "Color temperature tint for glass transmission.",
        "DOF" => "Enables depth of field in path tracing.",
        "Aperture" => "Depth-of-field aperture size.",
        "Focus" => "Depth-of-field focus distance.",
        "Backend" => "Path tracing backend selection.",
        "WF Tile" => "Wavefront tile size; zero means full frame.",
        "WF Scope" => "Which features currently use wavefront rendering.",
        "GPU BVH" => "Builds path tracing acceleration data on the GPU.",
        "BVH Refit" => "Updates BVH bounds quickly for animated geometry.",
        "Spectral" => "Spectral sampling mode for path tracing.",
        "Spectral SPP" => "Samples per pixel used by spectral mode.",
        "ReSTIR" => "Reservoir resampling controls for direct/global illumination.",
        "Reuse" => "Temporal and spatial ReSTIR reuse modes.",
        "M max" => "Maximum candidate count for reservoir sampling.",
        "Path Guide" => "Learns preferred light directions for path sampling.",
        "SVO" => "Sparse voxel resolution used by path guiding.",
        "Background" => "Solid background color used by the renderer.",
        "Env Map" => "Environment map lighting controls.",
        "Intensity" => "Environment map lighting intensity.",
        "Rotation" => "Environment map rotation around the scene.",
        "Env Anim" => "Animates environment map rotation.",
        "Hover" => "3D hover highlight mode.",
        "Width" => "Outline width for hover/selection highlight.",
        "Alpha" => "Opacity of hover/selection highlight.",
        "Inertia" => "Smooth camera momentum after dragging.",
        "Friction" => "How quickly camera inertia slows down.",
        "Cutoff" => "Velocity threshold where camera inertia stops.",
        _ => "Renderer setting.",
    }
}

/// Compact collapsing section — thinner header bar, full panel width.
///
/// Default egui `CollapsingHeader` sizes the click strip by
/// `interact_size.y` (18 px) and adds `item_spacing.y` above and below,
/// which on a tall settings panel looks fat. We scope the spacing tweak
/// to the header line only (the body uses the parent ui style) and force
/// the click strip to span the available width so the chevron + label
/// align with the rest of the panel.
fn compact_section(
    ui: &mut egui::Ui,
    title: &'static str,
    default_open: bool,
    add_contents: impl FnOnce(&mut egui::Ui),
) {
    ui.scope(|ui| {
        let spacing = ui.spacing_mut();
        spacing.interact_size.y = 14.0;
        spacing.item_spacing.y = 1.0;
        spacing.button_padding = egui::vec2(2.0, 1.0);
        // Make the click strip span the full panel width.
        ui.set_min_width(ui.available_width());
        egui::CollapsingHeader::new(title)
            .default_open(default_open)
            .show_unindented(ui, add_contents);
    });
}

impl App {
    /// Renderer section (2D/3D mode-specific settings)
    pub(super) fn ui_settings_renderer(&mut self, ui: &mut egui::Ui) {
        egui::CollapsingHeader::new("Renderer")
            .default_open(true)
            .show(ui, |ui| {
                // Mode-specific settings
                if self.render_mode == RenderMode::Mode2D {
                    self.ui_2d_settings(ui);
                }
                if self.render_mode == RenderMode::Mode3D {
                    self.ui_3d_settings(ui);
                }
            });
    }

    /// 2D renderer settings
    fn ui_2d_settings(&mut self, ui: &mut egui::Ui) {
        // CPU/GPU toggle moved to toolbar
        if self.viewport.zoom != 1.0 || self.viewport.pan != [0.0, 0.0] {
            ui.horizontal(|ui| {
                ui.small(format!("Zoom: {:.0}%", self.viewport.zoom * 100.0));
                if ui.small_button("Reset").clicked() {
                    self.viewport.reset();
                    self.needs_layout = true;
                }
            });
        }
    }

    /// 3D renderer settings - reorganized with clear subsections
    fn ui_3d_settings(&mut self, ui: &mut egui::Ui) {
        // Shading mode tabs at the top
        self.ui_3d_shading_mode(ui);

        ui.separator();

        // Geometry settings
        self.ui_3d_geometry(ui);

        // Effects settings
        self.ui_3d_effects(ui);

        // Animation: master + per-timeline knobs in one place.
        self.ui_3d_animation(ui);

        // Mode-specific panels
        let is_shaded = !self.render_3d_opts.show_wireframe && !self.render_3d_opts.path_tracing;
        if is_shaded {
            self.ui_3d_material(ui);
        }
        if self.render_3d_opts.path_tracing {
            self.ui_3d_pathtracer(ui);
        }

        // Environment and interaction
        self.ui_3d_environment(ui);
        self.ui_3d_interaction(ui);

        // Camera controls
        self.ui_3d_camera(ui);
    }

    /// Shading mode selection (Shaded/Wireframe/Path Tracing)
    fn ui_3d_shading_mode(&mut self, ui: &mut egui::Ui) {
        let is_shaded = !self.render_3d_opts.show_wireframe && !self.render_3d_opts.path_tracing;
        egui::Grid::new("shading_mode_grid")
            .num_columns(2)
            .spacing([8.0, 4.0])
            .min_col_width(SETTINGS_LABEL_WIDTH)
            .show(ui, |ui| {
                control_label(ui, "Mode:");
                ui.horizontal(|ui| {
                    if ui.selectable_label(is_shaded, "Shaded").clicked() {
                        self.render_3d_opts.show_wireframe = false;
                        self.render_3d_opts.path_tracing = false;
                        self.needs_layout = true;
                    }
                    if ui
                        .selectable_label(self.render_3d_opts.show_wireframe, "Wireframe")
                        .clicked()
                    {
                        self.render_3d_opts.show_wireframe = true;
                        self.render_3d_opts.path_tracing = false;
                        self.needs_layout = true;
                    }
                    if ui
                        .selectable_label(self.render_3d_opts.path_tracing, "Path Trace")
                        .clicked()
                    {
                        self.render_3d_opts.path_tracing = true;
                        self.render_3d_opts.show_wireframe = false;
                        if let Some(r) = &mut self.renderer_3d {
                            r.mark_pt_scene_dirty();
                        }
                        self.needs_layout = true;
                    }
                });
                ui.end_row();
            });
    }

    /// Geometry settings (height / color / folder / LOD).
    /// Each subsection is a collapsible group so the panel stays tidy
    /// when several modes are configured.
    fn ui_3d_geometry(&mut self, ui: &mut egui::Ui) {
        tinted_section(ui, "Geometry", true, self.settings_tint_mix, |ui| {
            // --- Height -----------------------------------------------
            let height_header = format!("Height: {}", self.render_3d_opts.height_mode.name());
            egui::CollapsingHeader::new(height_header)
                .id_salt("geom_height_section")
                .default_open(true)
                .show(ui, |ui| {
                    settings_grid(ui, "geom_height_grid", |ui| {
                        control_label(ui, "Mode");
                        let old_mode = self.render_3d_opts.height_mode;
                        ui.vertical(|ui| {
                            ui.horizontal(|ui| {
                                multibutton_exclusive(
                                    ui,
                                    &mut self.render_3d_opts.height_mode,
                                    &[
                                        (CubeHeightMode::Constant, "Const"),
                                        (CubeHeightMode::FileSize, "Size"),
                                        (CubeHeightMode::OwnSize, "Own"),
                                        (CubeHeightMode::FileCount, "Files"),
                                    ],
                                    MultiButtonAxis::Horizontal,
                                );
                            });
                            ui.horizontal(|ui| {
                                multibutton_exclusive(
                                    ui,
                                    &mut self.render_3d_opts.height_mode,
                                    &[
                                        (CubeHeightMode::DirCount, "Dirs"),
                                        (CubeHeightMode::Age, "Age"),
                                        (CubeHeightMode::Depth, "Depth"),
                                    ],
                                    MultiButtonAxis::Horizontal,
                                );
                            });
                        });
                        if self.render_3d_opts.height_mode != old_mode {
                            self.needs_layout = true;
                        }
                        ui.end_row();

                        // Per-mode Scale + Scale Exponent.
                        let active = self.render_3d_opts.height_mode as usize;
                        let curve = self.render_3d_opts.height_curves.get_mut(active);
                        if curve_rows(ui, curve) {
                            self.needs_layout = true;
                        }
                    });
                });

            // --- Color ------------------------------------------------
            let color_header = format!("Color: {}", self.render_3d_opts.color_mode.name());
            egui::CollapsingHeader::new(color_header)
                .id_salt("geom_color_section")
                .default_open(false)
                .show(ui, |ui| {
                    settings_grid(ui, "geom_color_grid", |ui| {
                        control_label(ui, "Mode");
                        let old = self.render_3d_opts.color_mode;
                        if multibutton_exclusive(
                            ui,
                            &mut self.render_3d_opts.color_mode,
                            &[
                                (ColorMode::Treemap, "Treemap"),
                                (ColorMode::FileType, "Type"),
                                (ColorMode::FileAge, "Age"),
                                (ColorMode::FileSize, "Size"),
                                (ColorMode::Depth, "Depth"),
                            ],
                            MultiButtonAxis::Horizontal,
                        ) {
                            self.needs_layout = true;
                        }
                        if self.render_3d_opts.color_mode != old {
                            self.needs_layout = true;
                        }
                        ui.end_row();
                    });

                    let cidx = self.render_3d_opts.color_mode as usize;
                    if ramp_section(
                        ui,
                        "Ramp",
                        self.render_3d_opts.color_ramps.get_mut(cidx),
                        RampUiCtx::full("color"),
                    ) {
                        self.needs_layout = true;
                    }
                });

            // --- Folder tint -----------------------------------------
            let folder_header = format!(
                "Folder tint: {}",
                self.render_3d_opts.folder_color_mode.name()
            );
            egui::CollapsingHeader::new(folder_header)
                .id_salt("geom_folder_section")
                .default_open(false)
                .show(ui, |ui| {
                    settings_grid(ui, "geom_folder_grid", |ui| {
                        control_label(ui, "Strength");
                        if ui
                            .add(
                                egui::Slider::new(&mut self.render_3d_opts.folder_tint, 0.0..=1.0)
                                    .show_value(true),
                            )
                            .changed()
                        {
                            self.needs_layout = true;
                        }
                        ui.end_row();

                        control_label(ui, "Mode");
                        multibutton_exclusive(
                            ui,
                            &mut self.render_3d_opts.folder_color_mode,
                            &[
                                (FolderColorMode::Depth, "Depth"),
                                (FolderColorMode::NameHash, "Name"),
                                (FolderColorMode::PathHash, "Path"),
                            ],
                            MultiButtonAxis::Horizontal,
                        );
                        ui.end_row();
                    });

                    let fidx = self.render_3d_opts.folder_color_mode as usize;
                    if ramp_section(
                        ui,
                        "Ramp",
                        self.render_3d_opts.folder_ramps.get_mut(fidx),
                        RampUiCtx::full("folder"),
                    ) {
                        self.needs_layout = true;
                    }
                });

            // --- LOD --------------------------------------------------
            egui::CollapsingHeader::new("LOD")
                .id_salt("geom_lod_section")
                .default_open(false)
                .show(ui, |ui| {
                    settings_grid(ui, "geom_lod_grid", |ui| {
                        control_label(ui, "Enable");
                        if ui
                            .checkbox(&mut self.render_3d_opts.lod_enabled, "")
                            .on_hover_text(
                                "Level of Detail: skip rendering cubes smaller than threshold",
                            )
                            .changed()
                        {
                            self.needs_layout = true;
                        }
                        ui.end_row();

                        if self.render_3d_opts.lod_enabled {
                            control_label(ui, "Min px");
                            if ui
                                .add(egui::Slider::new(
                                    &mut self.render_3d_opts.lod_min_screen_size,
                                    0.5..=10.0,
                                ))
                                .changed()
                            {
                                self.needs_layout = true;
                            }
                            ui.end_row();
                        }
                    });
                });
        });
    }

    /// Effects settings (hash transforms, animation)
    fn ui_3d_effects(&mut self, ui: &mut egui::Ui) {
        tinted_section(ui, "Effects", false, self.settings_tint_mix, |ui| {
            egui::Grid::new("effects_grid")
                .num_columns(2)
                .spacing([8.0, 4.0])
                .min_col_width(SETTINGS_LABEL_WIDTH)
                .show(ui, |ui| {
                    control_label(ui, "Effect:");
                    let old = self.render_3d_opts.hash_effect;
                    egui::ComboBox::from_id_salt("effect")
                        .selected_text(self.render_3d_opts.hash_effect.name())
                        .show_ui(ui, |ui| {
                            for e in HashTransformEffect::all() {
                                ui.selectable_value(
                                    &mut self.render_3d_opts.hash_effect,
                                    *e,
                                    e.name(),
                                );
                            }
                        });
                    if self.render_3d_opts.hash_effect != old {
                        self.needs_layout = true;
                    }
                    ui.end_row();

                    if self.render_3d_opts.hash_effect != HashTransformEffect::None {
                        // Per-effect Strength + Speed: switching effects
                        // preserves each variant's tuning so "Wave" can
                        // shimmer fast and "Pulse" can breathe slow
                        // without re-tuning when you swap.
                        let effect_idx = self.render_3d_opts.hash_effect as usize;
                        let params = self
                            .render_3d_opts
                            .effects
                            .hash_per_variant
                            .get_mut(effect_idx);
                        control_label(ui, "Strength");
                        if ui
                            .add(egui::Slider::new(&mut params.strength, 0.0..=10.0))
                            .changed()
                        {
                            self.needs_layout = true;
                        }
                        ui.end_row();
                        control_label(ui, "Speed");
                        if ui
                            .add(egui::Slider::new(&mut params.speed, 0.0..=5.0))
                            .changed()
                        {
                            self.needs_layout = true;
                        }
                        ui.end_row();
                    }

                });

            // Slice plane controls
            ui.separator();
            egui::Grid::new("slice_enable_grid")
                .num_columns(2)
                .spacing([8.0, 4.0])
                .min_col_width(SETTINGS_LABEL_WIDTH)
                .show(ui, |ui| {
                    control_label(ui, "Slice Plane:");
                    if ui
                        .checkbox(&mut self.render_3d_opts.slice_enabled, "")
                        .changed()
                    {
                        self.needs_layout = true;
                    }
                    ui.end_row();
                });
            if self.render_3d_opts.slice_enabled {
                egui::Grid::new("slice_grid")
                    .num_columns(2)
                    .spacing([8.0, 4.0])
                    .min_col_width(SETTINGS_LABEL_WIDTH)
                    .show(ui, |ui| {
                        control_label(ui, "Mode:");
                        ui.horizontal(|ui| {
                            if ui
                                .selectable_label(!self.render_3d_opts.slice_use_vector, "Axis")
                                .clicked()
                            {
                                self.render_3d_opts.slice_use_vector = false;
                                self.needs_layout = true;
                            }
                            if ui
                                .selectable_label(self.render_3d_opts.slice_use_vector, "Vector")
                                .clicked()
                            {
                                self.render_3d_opts.slice_use_vector = true;
                                self.needs_layout = true;
                            }
                        });
                        ui.end_row();

                        if self.render_3d_opts.slice_use_vector {
                            control_label(ui, "Normal:");
                            let mut changed = false;
                            ui.horizontal(|ui| {
                                changed |= ui
                                    .add(
                                        egui::DragValue::new(
                                            &mut self.render_3d_opts.slice_normal[0],
                                        )
                                        .speed(0.01)
                                        .range(-1.0..=1.0)
                                        .prefix("X:"),
                                    )
                                    .changed();
                                changed |= ui
                                    .add(
                                        egui::DragValue::new(
                                            &mut self.render_3d_opts.slice_normal[1],
                                        )
                                        .speed(0.01)
                                        .range(-1.0..=1.0)
                                        .prefix("Y:"),
                                    )
                                    .changed();
                                changed |= ui
                                    .add(
                                        egui::DragValue::new(
                                            &mut self.render_3d_opts.slice_normal[2],
                                        )
                                        .speed(0.01)
                                        .range(-1.0..=1.0)
                                        .prefix("Z:"),
                                    )
                                    .changed();
                            });
                            if changed {
                                let n = &mut self.render_3d_opts.slice_normal;
                                let len = (n[0] * n[0] + n[1] * n[1] + n[2] * n[2]).sqrt();
                                if len > 0.001 {
                                    n[0] /= len;
                                    n[1] /= len;
                                    n[2] /= len;
                                }
                                self.needs_layout = true;
                            }
                            ui.end_row();
                        } else {
                            control_label(ui, "Axis:");
                            let axes = [(0_u32, "X"), (1_u32, "Y"), (2_u32, "Z")];
                            if multibutton_exclusive(
                                ui,
                                &mut self.render_3d_opts.slice_axis,
                                &axes,
                                MultiButtonAxis::Horizontal,
                            ) {
                                self.needs_layout = true;
                            }
                            ui.end_row();
                        }

                        control_label(ui, "Distance:");
                        let range = if self.render_3d_opts.slice_use_vector {
                            -500.0..=500.0
                        } else {
                            -2000.0..=2000.0
                        };
                        let pos_ref = if self.render_3d_opts.slice_use_vector {
                            &mut self.render_3d_opts.slice_position_vector
                        } else {
                            &mut self.render_3d_opts.slice_position
                        };
                        if ui.add(egui::Slider::new(pos_ref, range)).changed() {
                            self.needs_layout = true;
                        }
                        ui.end_row();
                    });

                egui::Grid::new("slice_invert_grid")
                    .num_columns(2)
                    .spacing([8.0, 4.0])
                    .min_col_width(SETTINGS_LABEL_WIDTH)
                    .show(ui, |ui| {
                        control_label(ui, "Invert:");
                        if ui
                            .checkbox(&mut self.render_3d_opts.slice_invert, "")
                            .changed()
                        {
                            self.needs_layout = true;
                        }
                        ui.end_row();
                    });
            }
        });
    }

    /// Animation panel: master Animate + global speed for the object
    /// timeline, plus the env-timeline toggle + speed. Env runs
    /// independently (own gate) so the sky can keep rolling when cubes
    /// are paused with Space.
    fn ui_3d_animation(&mut self, ui: &mut egui::Ui) {
        tinted_section(ui, "Animation", false, self.settings_tint_mix, |ui| {
            settings_grid(ui, "animation_grid", |ui| {
                control_label(ui, "Animate");
                ui.horizontal(|ui| {
                    if ui
                        .checkbox(&mut self.render_3d_opts.animate, "")
                        .changed()
                    {
                        self.needs_layout = true;
                    }
                    ui.add_enabled(
                        self.render_3d_opts.animate,
                        egui::Slider::new(
                            &mut self.render_3d_opts.animation_speed,
                            0.0..=5.0,
                        )
                        .show_value(true),
                    );
                });
                ui.end_row();

                control_label(ui, "Env");
                ui.horizontal(|ui| {
                    ui.checkbox(&mut self.render_3d_opts.env_animate, "Animate");
                    ui.add_enabled(
                        self.render_3d_opts.env_animate,
                        egui::Slider::new(&mut self.render_3d_opts.env_speed, 0.0..=5.0)
                            .show_value(true),
                    );
                });
                ui.end_row();
            });
        });
    }

    /// Material settings (PBR properties)
    fn ui_3d_material(&mut self, ui: &mut egui::Ui) {
        tinted_section(ui, "Material", false, self.settings_tint_mix, |ui| {
            egui::Grid::new("material_grid")
                .num_columns(2)
                .spacing([8.0, 4.0])
                .min_col_width(SETTINGS_LABEL_WIDTH)
                .show(ui, |ui| {
                    // Source: what data determines the material
                    control_label(ui, "Source:");
                    let old_source = self.render_3d_opts.mat_source;
                    if multibutton_exclusive(
                        ui,
                        &mut self.render_3d_opts.mat_source,
                        &[
                            (MaterialSource::None, "None"),
                            (MaterialSource::Extension, "Ext"),
                            (MaterialSource::Path, "Path"),
                            (MaterialSource::Size, "Size"),
                            (MaterialSource::Depth, "Depth"),
                            (MaterialSource::Random, "Rand"),
                        ],
                        MultiButtonAxis::Horizontal,
                    ) {
                        // Sync legacy mode
                        self.render_3d_opts.materialize_mode = match self.render_3d_opts.mat_source
                        {
                            MaterialSource::None => MaterializeMode::None,
                            MaterialSource::Extension => MaterializeMode::ByExtension,
                            MaterialSource::Path => MaterializeMode::ByPath,
                            MaterialSource::Size => MaterializeMode::BySize,
                            MaterialSource::Age | MaterialSource::Depth => MaterializeMode::ByAge,
                            MaterialSource::Random => MaterializeMode::Random,
                        };
                        if let Some(r) = &mut self.renderer_3d {
                            r.mark_pt_scene_dirty();
                        }
                    }
                    if self.render_3d_opts.mat_source != old_source {
                        if let Some(r) = &mut self.renderer_3d {
                            r.mark_pt_scene_dirty();
                        }
                    }
                    ui.end_row();

                    if self.render_3d_opts.mat_source != MaterialSource::None {
                        // Palette: smooth gradient ramp. `None` lets pt-mats
                        // auto-pick a palette appropriate for the source
                        // (Viridis for Size, Sunset for Age, etc.).
                        control_label(ui, "Palette:");
                        let mut palette_changed = false;
                        let cur_label = match self.render_3d_opts.mat_palette {
                            None => "Auto".to_string(),
                            Some(p) => p.name().to_string(),
                        };
                        egui::ComboBox::from_id_salt("mat_palette")
                            .selected_text(cur_label)
                            .show_ui(ui, |ui| {
                                if ui
                                    .selectable_value(
                                        &mut self.render_3d_opts.mat_palette,
                                        None,
                                        "Auto",
                                    )
                                    .changed()
                                {
                                    palette_changed = true;
                                }
                                for &p in Palette::all() {
                                    if ui
                                        .selectable_value(
                                            &mut self.render_3d_opts.mat_palette,
                                            Some(p),
                                            p.name(),
                                        )
                                        .changed()
                                    {
                                        palette_changed = true;
                                    }
                                }
                            });
                        if palette_changed {
                            if let Some(r) = &mut self.renderer_3d {
                                r.mark_pt_scene_dirty();
                            }
                        }
                        ui.end_row();

                        // Path-hierarchical: only meaningful for Source=Path.
                        // Siblings cluster into nearby palette colours when
                        // enabled, scatter randomly when disabled.
                        if self.render_3d_opts.mat_source == MaterialSource::Path {
                            control_label(ui, "Cluster siblings:");
                            if ui
                                .checkbox(
                                    &mut self.render_3d_opts.mat_path_hierarchical,
                                    "Hierarchical",
                                )
                                .changed()
                            {
                                if let Some(r) = &mut self.renderer_3d {
                                    r.mark_pt_scene_dirty();
                                }
                            }
                            ui.end_row();
                        }

                        // Distribution: how values map to materials
                        control_label(ui, "Distribute:");
                        let old_dist = self.render_3d_opts.mat_distribution;
                        if multibutton_exclusive(
                            ui,
                            &mut self.render_3d_opts.mat_distribution,
                            &[
                                (MaterialDistribution::Direct, "Direct"),
                                (MaterialDistribution::Quantized, "Quant"),
                                (MaterialDistribution::Gradient, "Grad"),
                                (MaterialDistribution::Spatial, "Spatial"),
                                (MaterialDistribution::Bands, "Bands"),
                            ],
                            MultiButtonAxis::Horizontal,
                        ) {
                            if let Some(r) = &mut self.renderer_3d {
                                r.mark_pt_scene_dirty();
                            }
                        }
                        if self.render_3d_opts.mat_distribution != old_dist {
                            if let Some(r) = &mut self.renderer_3d {
                                r.mark_pt_scene_dirty();
                            }
                        }
                        ui.end_row();

                        // Distribution-specific parameters
                        match self.render_3d_opts.mat_distribution {
                            MaterialDistribution::Quantized => {
                                control_label(ui, "Levels:");
                                if ui
                                    .add(egui::Slider::new(
                                        &mut self.render_3d_opts.mat_quant_levels,
                                        2..=14,
                                    ))
                                    .changed()
                                {
                                    if let Some(r) = &mut self.renderer_3d {
                                        r.mark_pt_scene_dirty();
                                    }
                                }
                                ui.end_row();
                            }
                            MaterialDistribution::Bands => {
                                control_label(ui, "Bands:");
                                if ui
                                    .add(egui::Slider::new(
                                        &mut self.render_3d_opts.mat_band_count,
                                        2..=20,
                                    ))
                                    .changed()
                                {
                                    if let Some(r) = &mut self.renderer_3d {
                                        r.mark_pt_scene_dirty();
                                    }
                                }
                                ui.end_row();
                            }
                            MaterialDistribution::Spatial => {
                                control_label(ui, "Scale:");
                                if ui
                                    .add(
                                        egui::Slider::new(
                                            &mut self.render_3d_opts.mat_spatial_scale,
                                            0.001..=0.1,
                                        )
                                        .logarithmic(true),
                                    )
                                    .changed()
                                {
                                    if let Some(r) = &mut self.renderer_3d {
                                        r.mark_pt_scene_dirty();
                                    }
                                }
                                ui.end_row();
                            }
                            _ => {}
                        }

                        // Seed
                        control_label(ui, "Seed:");
                        if ui
                            .add(
                                egui::Slider::new(&mut self.render_3d_opts.mat_seed, 1..=u32::MAX)
                                    .logarithmic(true),
                            )
                            .changed()
                        {
                            if let Some(r) = &mut self.renderer_3d {
                                r.mark_pt_scene_dirty();
                            }
                        }
                        ui.end_row();

                        // Mix
                        control_label(ui, "Mix:");
                        if ui
                            .add(egui::Slider::new(
                                &mut self.render_3d_opts.materialize_mix,
                                0.0..=1.0,
                            ))
                            .changed()
                        {
                            self.needs_layout = true;
                        }
                        ui.end_row();
                    }

                    control_label(ui, "Roughness:");
                    if ui
                        .add(egui::Slider::new(
                            &mut self.render_3d_opts.roughness,
                            0.04..=1.0,
                        ))
                        .changed()
                    {
                        self.needs_layout = true;
                    }
                    ui.end_row();

                    control_label(ui, "Metalness:");
                    if ui
                        .add(egui::Slider::new(
                            &mut self.render_3d_opts.metalness,
                            0.0..=1.0,
                        ))
                        .changed()
                    {
                        self.needs_layout = true;
                    }
                    ui.end_row();

                    control_label(ui, "Specular IOR:");
                    if ui
                        .add(egui::Slider::new(
                            &mut self.render_3d_opts.specular_ior,
                            1.0..=3.0,
                        ))
                        .changed()
                    {
                        self.needs_layout = true;
                    }
                    ui.end_row();
                });

            egui::Grid::new("material_flags_grid")
                .num_columns(2)
                .spacing([8.0, 4.0])
                .min_col_width(SETTINGS_LABEL_WIDTH)
                .show(ui, |ui| {
                    control_label(ui, "Shading:");
                    ui.horizontal(|ui| {
                        ui.checkbox(&mut self.render_3d_opts.flat_shading, "Flat");
                        ui.checkbox(&mut self.render_3d_opts.double_sided, "Double Sided");
                    });
                    ui.end_row();
                });
        });
    }

    /// Path tracer settings
    fn ui_3d_pathtracer(&mut self, ui: &mut egui::Ui) {
        tinted_section(ui, "Path Tracer", true, self.settings_tint_mix, |ui| {
            let mut pt_changed = false;

            compact_section(ui, "Materials", true, |ui| self.ui_pt_materials(ui));
            compact_section(ui, "Lighting", true, |ui| {
                self.ui_pt_lighting(ui, &mut pt_changed)
            });
            compact_section(ui, "Sampling", true, |ui| {
                self.ui_pt_sampling(ui, &mut pt_changed)
            });
            compact_section(ui, "Paths", false, |ui| {
                self.ui_pt_paths(ui, &mut pt_changed)
            });
            compact_section(ui, "Glass", false, |ui| {
                self.ui_pt_glass(ui, &mut pt_changed)
            });
            compact_section(ui, "Camera", false, |ui| {
                self.ui_pt_camera(ui, &mut pt_changed)
            });
            compact_section(ui, "Advanced", false, |ui| {
                self.ui_pt_advanced(ui, &mut pt_changed)
            });

            if pt_changed {
                if let Some(r) = &mut self.renderer_3d {
                    r.reset_pt_accumulation();
                }
            }
        });
    }

    fn mark_pt_scene_dirty(&mut self) {
        if let Some(r) = &mut self.renderer_3d {
            r.mark_pt_scene_dirty();
        }
    }

    fn pt_total_cubes(&self) -> u32 {
        self.filtered_tree
            .as_ref()
            .or(self.tree.as_ref())
            .map(|t| t.file_count as u32)
            .unwrap_or(0)
    }

    fn ui_pt_materials(&mut self, ui: &mut egui::Ui) {
        let total_cubes = self.pt_total_cubes();
        self.backfill_pt_material_counts(total_cubes);

        settings_grid(ui, "pt_materials_grid", |ui| {
            control_label(ui, "Source:");
            let old_source = self.render_3d_opts.mat_source;
            if multibutton_exclusive(
                ui,
                &mut self.render_3d_opts.mat_source,
                &[
                    (MaterialSource::None, "None"),
                    (MaterialSource::Extension, "Ext"),
                    (MaterialSource::Path, "Path"),
                    (MaterialSource::Size, "Size"),
                    (MaterialSource::Depth, "Depth"),
                    (MaterialSource::Random, "Rand"),
                ],
                MultiButtonAxis::Horizontal,
            ) {
                self.render_3d_opts.materialize_mode = match self.render_3d_opts.mat_source {
                    MaterialSource::None => MaterializeMode::None,
                    MaterialSource::Extension => MaterializeMode::ByExtension,
                    MaterialSource::Path => MaterializeMode::ByPath,
                    MaterialSource::Size => MaterializeMode::BySize,
                    MaterialSource::Age | MaterialSource::Depth => MaterializeMode::ByAge,
                    MaterialSource::Random => MaterializeMode::Random,
                };
                self.mark_pt_scene_dirty();
            }
            if self.render_3d_opts.mat_source != old_source {
                self.mark_pt_scene_dirty();
            }
            ui.end_row();

            if self.render_3d_opts.mat_source == MaterialSource::None {
                return;
            }

            control_label(ui, "Distribute:");
            let old_dist = self.render_3d_opts.mat_distribution;
            if multibutton_exclusive(
                ui,
                &mut self.render_3d_opts.mat_distribution,
                &[
                    (MaterialDistribution::Direct, "Direct"),
                    (MaterialDistribution::Quantized, "Quant"),
                    (MaterialDistribution::Gradient, "Grad"),
                    (MaterialDistribution::Spatial, "Spatial"),
                    (MaterialDistribution::Bands, "Bands"),
                ],
                MultiButtonAxis::Horizontal,
            ) {
                self.mark_pt_scene_dirty();
            }
            if self.render_3d_opts.mat_distribution != old_dist {
                self.mark_pt_scene_dirty();
            }
            ui.end_row();

            match self.render_3d_opts.mat_distribution {
                MaterialDistribution::Quantized => {
                    control_label(ui, "Levels:");
                    if ui
                        .add(egui::Slider::new(
                            &mut self.render_3d_opts.mat_quant_levels,
                            2..=14,
                        ))
                        .changed()
                    {
                        self.mark_pt_scene_dirty();
                    }
                    ui.end_row();
                }
                MaterialDistribution::Bands => {
                    control_label(ui, "Bands:");
                    if ui
                        .add(egui::Slider::new(
                            &mut self.render_3d_opts.mat_band_count,
                            2..=20,
                        ))
                        .changed()
                    {
                        self.mark_pt_scene_dirty();
                    }
                    ui.end_row();
                }
                MaterialDistribution::Spatial => {
                    control_label(ui, "Scale:");
                    if ui
                        .add(
                            egui::Slider::new(
                                &mut self.render_3d_opts.mat_spatial_scale,
                                0.001..=0.1,
                            )
                            .logarithmic(true),
                        )
                        .changed()
                    {
                        self.mark_pt_scene_dirty();
                    }
                    ui.end_row();
                }
                _ => {}
            }

            control_label(ui, "Seed:");
            if ui
                .add(
                    egui::Slider::new(&mut self.render_3d_opts.mat_seed, 1..=u32::MAX)
                        .logarithmic(true),
                )
                .changed()
            {
                self.mark_pt_scene_dirty();
            }
            ui.end_row();

            control_label(ui, "Mix:");
            if ui
                .add(
                    egui::Slider::new(&mut self.render_3d_opts.materialize_mix, 0.0..=1.0)
                        .show_value(true),
                )
                .changed()
            {
                self.mark_pt_scene_dirty();
            }
            ui.end_row();

            self.ui_pt_material_counts(ui, total_cubes);
        });
    }

    fn backfill_pt_material_counts(&mut self, total_cubes: u32) {
        if self.render_3d_opts.mat_allow_lights
            && self.render_3d_opts.mat_light_count == 0
            && self.render_3d_opts.mat_light_prob > 0.0
            && total_cubes > 0
        {
            self.render_3d_opts.mat_light_count =
                (self.render_3d_opts.mat_light_prob * total_cubes as f32).round() as u32;
        }

        if self.render_3d_opts.mat_allow_glass
            && self.render_3d_opts.mat_glass_count == 0
            && self.render_3d_opts.mat_glass_prob > 0.0
            && total_cubes > 0
        {
            self.render_3d_opts.mat_glass_count =
                (self.render_3d_opts.mat_glass_prob * total_cubes as f32).round() as u32;
        }
    }

    fn pt_count_drag_max(total_cubes: u32) -> u32 {
        if total_cubes > 0 {
            total_cubes.clamp(1, MAX_PT_MAT_CUBE_COUNT)
        } else {
            MAX_PT_MAT_CUBE_COUNT
        }
    }

    fn ui_pt_material_counts(&mut self, ui: &mut egui::Ui, total_cubes: u32) {
        control_label(ui, "Light Cubes:");
        ui.horizontal(|ui| {
            if ui
                .checkbox(&mut self.render_3d_opts.mat_allow_lights, "")
                .on_hover_text("Enable PT light materials")
                .changed()
            {
                self.mark_pt_scene_dirty();
            }
            if self.render_3d_opts.mat_allow_lights {
                if ui
                    .add(
                        egui::DragValue::new(&mut self.render_3d_opts.mat_light_count)
                            .range(0..=Self::pt_count_drag_max(total_cubes))
                            .clamp_existing_to_range(false)
                            .speed(1.0)
                            .suffix(" cubes"),
                    )
                    .on_hover_text("Number of cubes to receive a light material")
                    .changed()
                {
                    let total = total_cubes.max(1) as f32;
                    self.render_3d_opts.mat_light_prob =
                        (self.render_3d_opts.mat_light_count as f32 / total).clamp(0.0, 1.0);
                    self.mark_pt_scene_dirty();
                }
                if total_cubes > 0 {
                    ui.small(format!(
                        "/{} ({:.1}%)",
                        total_cubes,
                        self.render_3d_opts.mat_light_prob * 100.0
                    ));
                }
            }
        });
        ui.end_row();

        if self.render_3d_opts.mat_allow_lights {
            control_label(ui, "Warm Bias:");
            if ui
                .add(egui::Slider::new(
                    &mut self.render_3d_opts.mat_light_warm,
                    0.0..=1.0,
                ))
                .changed()
            {
                self.mark_pt_scene_dirty();
            }
            ui.end_row();

            control_label(ui, "Cool Bias:");
            if ui
                .add(egui::Slider::new(
                    &mut self.render_3d_opts.mat_light_cool,
                    0.0..=1.0,
                ))
                .changed()
            {
                self.mark_pt_scene_dirty();
            }
            ui.end_row();

            control_label(ui, "Light Power:");
            if ui
                .add(egui::Slider::new(
                    &mut self.render_3d_opts.mat_light_intensity,
                    0.0..=10.0,
                ))
                .changed()
            {
                self.mark_pt_scene_dirty();
            }
            ui.end_row();

            control_label(ui, "Light Rand:");
            if ui
                .add(egui::Slider::new(
                    &mut self.render_3d_opts.mat_light_color_randomness,
                    0.0..=1.0,
                ))
                .changed()
            {
                self.mark_pt_scene_dirty();
            }
            ui.end_row();
        }

        control_label(ui, "Glass Cubes:");
        ui.horizontal(|ui| {
            if ui
                .checkbox(&mut self.render_3d_opts.mat_allow_glass, "")
                .on_hover_text("Enable glass/transparent materials")
                .changed()
            {
                self.mark_pt_scene_dirty();
            }
            if self.render_3d_opts.mat_allow_glass {
                if ui
                    .add(
                        egui::DragValue::new(&mut self.render_3d_opts.mat_glass_count)
                            .range(0..=Self::pt_count_drag_max(total_cubes))
                            .clamp_existing_to_range(false)
                            .speed(1.0)
                            .suffix(" cubes"),
                    )
                    .on_hover_text("Number of cubes to receive a glass material")
                    .changed()
                {
                    let total = total_cubes.max(1) as f32;
                    self.render_3d_opts.mat_glass_prob =
                        (self.render_3d_opts.mat_glass_count as f32 / total).clamp(0.0, 1.0);
                    self.mark_pt_scene_dirty();
                }
                if total_cubes > 0 {
                    ui.small(format!(
                        "/{} ({:.1}%)",
                        total_cubes,
                        self.render_3d_opts.mat_glass_prob * 100.0
                    ));
                }
            }
        });
        ui.end_row();
    }

    fn ui_pt_lighting(&mut self, ui: &mut egui::Ui, pt_changed: &mut bool) {
        settings_grid(ui, "pt_lighting_grid", |ui| {
            control_label(ui, "Env MIS:");
            if ui
                .checkbox(&mut self.render_3d_opts.pt_env_importance_sampling, "")
                .on_hover_text("Use HDR CDF importance sampling + MIS")
                .changed()
            {
                *pt_changed = true;
                self.mark_pt_scene_dirty();
            }
            ui.end_row();

            control_label(ui, "Emissive NEE:");
            if ui
                .checkbox(&mut self.render_3d_opts.pt_emissive_sampling, "")
                .on_hover_text("Directly sample emissive cubes")
                .changed()
            {
                *pt_changed = true;
            }
            ui.end_row();

            if self.render_3d_opts.pt_emissive_sampling {
                control_label(ui, "Light SPP:");
                *pt_changed |= ui
                    .add(
                        egui::DragValue::new(&mut self.render_3d_opts.pt_emissive_samples)
                            .range(1..=8)
                            .speed(1),
                    )
                    .changed();
                ui.end_row();

                control_label(ui, "Light Min:");
                if ui
                    .add(
                        egui::Slider::new(
                            &mut self.render_3d_opts.pt_emissive_min_weight,
                            1e-5..=1.0,
                        )
                        .logarithmic(true),
                    )
                    .changed()
                {
                    *pt_changed = true;
                    self.mark_pt_scene_dirty();
                }
                ui.end_row();
            }
        });
    }

    fn ui_pt_sampling(&mut self, ui: &mut egui::Ui, pt_changed: &mut bool) {
        settings_grid(ui, "pt_sampling_grid", |ui| {
            control_label(ui, "Max Samples:");
            ui.horizontal(|ui| {
                if ui
                    .add(
                        egui::Slider::new(&mut self.render_3d_opts.pt_max_samples, 16..=32768)
                            .logarithmic(true),
                    )
                    .changed()
                {
                    if self.render_3d_opts.pt_adaptive_sampling
                        && self.render_3d_opts.pt_adaptive_max_spp
                            < self.render_3d_opts.pt_max_samples
                    {
                        self.render_3d_opts.pt_adaptive_max_spp =
                            self.render_3d_opts.pt_max_samples;
                    }
                    *pt_changed = true;
                }
                for samples in [512_u32, 2048, 4096, 8192, 16384] {
                    if ui
                        .selectable_label(
                            self.render_3d_opts.pt_max_samples == samples,
                            samples.to_string(),
                        )
                        .clicked()
                    {
                        self.render_3d_opts.pt_max_samples = samples;
                        if self.render_3d_opts.pt_adaptive_sampling {
                            self.render_3d_opts.pt_adaptive_max_spp = samples;
                        }
                        *pt_changed = true;
                    }
                }
            });
            ui.end_row();

            control_label(ui, "SPP/frame:");
            if self.render_3d_opts.pt_auto_spp {
                if let Some(r) = &self.renderer_3d {
                    ui.label(format!("{}", r.pt_samples_per_update().max(1)));
                } else {
                    ui.label("-");
                }
            } else {
                *pt_changed |= ui
                    .add(egui::Slider::new(
                        &mut self.render_3d_opts.pt_samples_per_update,
                        1..=64,
                    ))
                    .changed();
            }
            ui.end_row();

            control_label(ui, "Auto SPP:");
            ui.horizontal(|ui| {
                *pt_changed |= ui
                    .checkbox(&mut self.render_3d_opts.pt_auto_spp, "")
                    .changed();
                if ui
                    .checkbox(&mut self.render_3d_opts.pt_camera_snap, "Camera Snap")
                    .changed()
                {
                    *pt_changed = true;
                }
            });
            ui.end_row();

            if self.render_3d_opts.pt_auto_spp || self.render_3d_opts.pt_camera_snap {
                control_label(ui, "Target FPS:");
                *pt_changed |= ui
                    .add(
                        egui::Slider::new(&mut self.render_3d_opts.pt_target_fps, 1.0..=120.0)
                            .integer(),
                    )
                    .changed();
                ui.end_row();
            }

            control_label(ui, "Sampler:");
            if multibutton_exclusive(
                ui,
                &mut self.render_3d_opts.pt_sampler_mode,
                &[(PtSamplerMode::Pcg, "PCG"), (PtSamplerMode::R2, "R2")],
                MultiButtonAxis::Horizontal,
            ) {
                *pt_changed = true;
            }
            ui.end_row();
        });

        if let Some(r) = &self.renderer_3d {
            let current = r.pt_frame_count();
            let max = if self.render_3d_opts.pt_adaptive_sampling {
                self.render_3d_opts
                    .pt_max_samples
                    .min(self.render_3d_opts.pt_adaptive_max_spp.max(1))
            } else {
                self.render_3d_opts.pt_max_samples
            };
            let progress = current as f32 / max as f32;
            let done = current >= max;
            ui.add(egui::ProgressBar::new(progress.min(1.0)).text(if done {
                format!("{} samples (done)", current)
            } else {
                format!("{} / {} samples", current, max)
            }));
            if self.render_3d_opts.pt_adaptive_sampling {
                ui.small(format!(
                    "Adaptive cap: {} (Max {}, Adaptive {})",
                    max,
                    self.render_3d_opts.pt_max_samples,
                    self.render_3d_opts.pt_adaptive_max_spp
                ));
            }
        }

        self.ui_pt_adaptive(ui, pt_changed);
    }

    fn ui_pt_adaptive(&mut self, ui: &mut egui::Ui, pt_changed: &mut bool) {
        settings_grid(ui, "pt_adaptive_grid", |ui| {
            control_label(ui, "Adaptive:");
            *pt_changed |= ui
                .checkbox(&mut self.render_3d_opts.pt_adaptive_sampling, "")
                .on_hover_text("More samples on high-variance areas")
                .changed();
            ui.end_row();

            if !self.render_3d_opts.pt_adaptive_sampling {
                return;
            }

            control_label(ui, "Preset:");
            if multibutton_exclusive(
                ui,
                &mut self.render_3d_opts.pt_adaptive_preset,
                &[
                    (AdaptivePreset::Custom, "Custom"),
                    (AdaptivePreset::Conservative, "Low"),
                    (AdaptivePreset::Balanced, "Medium"),
                    (AdaptivePreset::Aggressive, "High"),
                ],
                MultiButtonAxis::Horizontal,
            ) {
                match self.render_3d_opts.pt_adaptive_preset {
                    AdaptivePreset::Conservative => {
                        self.render_3d_opts.pt_adaptive_min_spp = 64;
                        self.render_3d_opts.pt_adaptive_max_spp = 512;
                        self.render_3d_opts.pt_adaptive_variance = 0.002;
                        self.render_3d_opts.pt_adaptive_interval = 6;
                    }
                    AdaptivePreset::Balanced => {
                        self.render_3d_opts.pt_adaptive_min_spp = 96;
                        self.render_3d_opts.pt_adaptive_max_spp = 1024;
                        self.render_3d_opts.pt_adaptive_variance = 0.001;
                        self.render_3d_opts.pt_adaptive_interval = 4;
                    }
                    AdaptivePreset::Aggressive => {
                        self.render_3d_opts.pt_adaptive_min_spp = 128;
                        self.render_3d_opts.pt_adaptive_max_spp = 2048;
                        self.render_3d_opts.pt_adaptive_variance = 0.0005;
                        self.render_3d_opts.pt_adaptive_interval = 2;
                    }
                    AdaptivePreset::Custom => {}
                }
                *pt_changed = true;
            }
            ui.end_row();

            control_label(ui, "SPP Range:");
            ui.horizontal(|ui| {
                ui.small("Min");
                let min_changed = ui
                    .add_sized(
                        [PT_VALUE_WIDTH, ui.spacing().interact_size.y],
                        egui::DragValue::new(&mut self.render_3d_opts.pt_adaptive_min_spp)
                            .range(32..=1024)
                            .speed(1),
                    )
                    .changed();
                ui.small("Max");
                let max_changed = ui
                    .add_sized(
                        [PT_VALUE_WIDTH, ui.spacing().interact_size.y],
                        egui::DragValue::new(&mut self.render_3d_opts.pt_adaptive_max_spp)
                            .range(1..=16384)
                            .speed(1),
                    )
                    .changed();
                if min_changed || max_changed {
                    self.render_3d_opts.pt_adaptive_preset = AdaptivePreset::Custom;
                    *pt_changed = true;
                }
            });
            ui.end_row();

            if self.render_3d_opts.pt_adaptive_max_spp < self.render_3d_opts.pt_adaptive_min_spp {
                self.render_3d_opts.pt_adaptive_max_spp = self.render_3d_opts.pt_adaptive_min_spp;
            }

            control_label(ui, "Variance:");
            let variance_changed = ui
                .add(
                    egui::Slider::new(&mut self.render_3d_opts.pt_adaptive_variance, 1e-5..=1e-2)
                        .logarithmic(true),
                )
                .changed();
            if variance_changed {
                self.render_3d_opts.pt_adaptive_preset = AdaptivePreset::Custom;
                *pt_changed = true;
            }
            ui.end_row();

            control_label(ui, "Interval:");
            let interval_changed = ui
                .add(
                    egui::DragValue::new(&mut self.render_3d_opts.pt_adaptive_interval)
                        .range(1..=60)
                        .speed(1),
                )
                .changed();
            if interval_changed {
                self.render_3d_opts.pt_adaptive_preset = AdaptivePreset::Custom;
                *pt_changed = true;
            }
            ui.end_row();
        });
    }

    fn ui_pt_paths(&mut self, ui: &mut egui::Ui, pt_changed: &mut bool) {
        settings_grid(ui, "pt_paths_grid", |ui| {
            control_label(ui, "Bounces:");
            *pt_changed |= ui
                .add(egui::Slider::new(
                    &mut self.render_3d_opts.pt_max_bounces,
                    1..=64,
                ))
                .changed();
            ui.end_row();

            control_label(ui, "Transmission:");
            *pt_changed |= ui
                .add(egui::Slider::new(
                    &mut self.render_3d_opts.pt_max_transmission_depth,
                    1..=64,
                ))
                .changed();
            ui.end_row();

            control_label(ui, "Russian Roulette:");
            *pt_changed |= ui
                .checkbox(&mut self.render_3d_opts.pt_russian_roulette, "")
                .on_hover_text("Probabilistic path termination")
                .changed();
            ui.end_row();
        });
    }

    fn ui_pt_glass(&mut self, ui: &mut egui::Ui, pt_changed: &mut bool) {
        settings_grid(ui, "pt_glass_grid", |ui| {
            control_label(ui, "Transparency:");
            let mut transparency_ui = self.render_3d_opts.pt_global_transparency * 64.0;
            if ui
                .add(egui::Slider::new(&mut transparency_ui, 0.0..=64.0))
                .changed()
            {
                self.render_3d_opts.pt_global_transparency =
                    (transparency_ui / 64.0).clamp(0.0, 1.0);
                *pt_changed = true;
                self.mark_pt_scene_dirty();
            }
            ui.end_row();

            control_label(ui, "Preset:");
            let old_glass = self.render_3d_opts.pt_global_glass;
            if multibutton_exclusive(
                ui,
                &mut self.render_3d_opts.pt_global_glass,
                &[
                    (GlassPreset::Clear, "Clear"),
                    (GlassPreset::Blue, "Blue"),
                    (GlassPreset::Green, "Green"),
                    (GlassPreset::Amber, "Amber"),
                    (GlassPreset::Pink, "Pink"),
                ],
                MultiButtonAxis::Horizontal,
            ) {
                *pt_changed = true;
                self.mark_pt_scene_dirty();
            }
            if self.render_3d_opts.pt_global_glass != old_glass {
                *pt_changed = true;
                self.mark_pt_scene_dirty();
            }
            ui.end_row();

            control_label(ui, "Thin:");
            if ui
                .checkbox(&mut self.render_3d_opts.pt_glass_thin, "")
                .changed()
            {
                *pt_changed = true;
                self.mark_pt_scene_dirty();
            }
            ui.end_row();

            control_label(ui, "Specular:");
            if ui
                .add(egui::Slider::new(
                    &mut self.render_3d_opts.pt_glass_specular,
                    0.0..=1.0,
                ))
                .changed()
            {
                *pt_changed = true;
                self.mark_pt_scene_dirty();
            }
            ui.end_row();

            control_label(ui, "Base:");
            if ui
                .add(egui::Slider::new(
                    &mut self.render_3d_opts.pt_glass_base,
                    0.0..=1.0,
                ))
                .changed()
            {
                *pt_changed = true;
                self.mark_pt_scene_dirty();
            }
            ui.end_row();

            control_label(ui, "Roughness:");
            if ui
                .add(egui::Slider::new(
                    &mut self.render_3d_opts.pt_glass_roughness,
                    0.0..=1.0,
                ))
                .changed()
            {
                *pt_changed = true;
                self.mark_pt_scene_dirty();
            }
            ui.end_row();

            control_label(ui, "IoR:");
            if ui
                .add(egui::Slider::new(
                    &mut self.render_3d_opts.pt_glass_ior,
                    1.0..=3.0,
                ))
                .changed()
            {
                *pt_changed = true;
                self.mark_pt_scene_dirty();
            }
            ui.end_row();

            control_label(ui, "Dispersion:");
            if ui
                .add(egui::Slider::new(
                    &mut self.render_3d_opts.pt_glass_dispersion,
                    0.0..=1.0,
                ))
                .changed()
            {
                *pt_changed = true;
                self.mark_pt_scene_dirty();
            }
            ui.end_row();

            control_label(ui, "Temperature:");
            if ui
                .add(
                    egui::Slider::new(&mut self.render_3d_opts.pt_glass_temp, 1000.0..=12000.0)
                        .integer()
                        .text("K"),
                )
                .changed()
            {
                *pt_changed = true;
                self.mark_pt_scene_dirty();
            }
            ui.end_row();
        });
    }

    fn ui_pt_camera(&mut self, ui: &mut egui::Ui, pt_changed: &mut bool) {
        settings_grid(ui, "pt_camera_grid", |ui| {
            control_label(ui, "DOF:");
            *pt_changed |= ui
                .checkbox(&mut self.render_3d_opts.pt_dof_enabled, "")
                .changed();
            ui.end_row();

            if self.render_3d_opts.pt_dof_enabled {
                control_label(ui, "Aperture:");
                *pt_changed |= ui
                    .add(egui::Slider::new(
                        &mut self.render_3d_opts.pt_aperture,
                        0.01..=2.0,
                    ))
                    .changed();
                ui.end_row();

                control_label(ui, "Focus:");
                *pt_changed |= ui
                    .add(
                        egui::Slider::new(&mut self.render_3d_opts.pt_focus_distance, 0.1..=500.0)
                            .logarithmic(true),
                    )
                    .changed();
                ui.end_row();
            }
        });
    }

    fn ui_pt_advanced(&mut self, ui: &mut egui::Ui, pt_changed: &mut bool) {
        settings_grid(ui, "pt_backend_grid", |ui| {
            control_label(ui, "Backend:");
            *pt_changed |= ui
                .checkbox(&mut self.render_3d_opts.pt_wavefront, "Wavefront")
                .on_hover_text("Split path tracing into separate passes")
                .changed();
            ui.end_row();

            if self.render_3d_opts.pt_wavefront {
                control_label(ui, "WF Tile:");
                ui.horizontal(|ui| {
                    let resp = ui.add(
                        egui::DragValue::new(&mut self.render_3d_opts.pt_wavefront_tile_size)
                            .range(0..=8192)
                            .speed(16),
                    );
                    if resp.changed() {
                        // Clamp to {0} ∪ [64, 8192]. 0 = full frame (no
                        // tiling). Non-zero values below 64 produce so many
                        // tiles that prepare_tiles trips its MAX_TILE_CAPACITY
                        // (4096) assertion and panics — type 0 directly to
                        // disable tiling. Drag-down halfway split (< 32 → 0,
                        // 32..64 → 64) so 0 stays reachable via drag.
                        let v = self.render_3d_opts.pt_wavefront_tile_size;
                        if v != 0 && v < 64 {
                            self.render_3d_opts.pt_wavefront_tile_size =
                                if v < 32 { 0 } else { 64 };
                        } else if v > 8192 {
                            self.render_3d_opts.pt_wavefront_tile_size = 8192;
                        }
                        *pt_changed = true;
                    }
                    ui.small("0 = full frame, else 64..8192");
                });
                ui.end_row();

                control_label(ui, "WF Scope:");
                ui.small("R2/NEE direct use megakernel; ReSTIR uses wavefront.");
                ui.end_row();
            }

            control_label(ui, "GPU BVH:");
            *pt_changed |= ui
                .checkbox(&mut self.render_3d_opts.pt_gpu_bvh, "")
                .on_hover_text("Build BVH on GPU")
                .changed();
            ui.end_row();

            if self.render_3d_opts.pt_gpu_bvh {
                control_label(ui, "BVH Refit:");
                *pt_changed |= ui
                    .checkbox(&mut self.render_3d_opts.pt_bvh_refit, "")
                    .on_hover_text("Fast AABB update for animation")
                    .changed();
                ui.end_row();
            }
        });

        settings_grid(ui, "pt_spectral_grid", |ui| {
            control_label(ui, "Spectral:");
            let old_spectral = self.render_3d_opts.pt_spectral_mode;
            if multibutton_exclusive(
                ui,
                &mut self.render_3d_opts.pt_spectral_mode,
                &[
                    (SpectralMode::Off, "Off"),
                    (SpectralMode::Hero, "Hero"),
                    (SpectralMode::Multi, "Multi"),
                ],
                MultiButtonAxis::Horizontal,
            ) {
                *pt_changed = true;
                self.mark_pt_scene_dirty();
            }
            if self.render_3d_opts.pt_spectral_mode != old_spectral {
                *pt_changed = true;
                self.mark_pt_scene_dirty();
            }
            ui.end_row();

            if self.render_3d_opts.pt_spectral_mode != SpectralMode::Off {
                control_label(ui, "Spectral SPP:");
                *pt_changed |= ui
                    .add(egui::Slider::new(
                        &mut self.render_3d_opts.pt_spectral_samples,
                        1..=8,
                    ))
                    .changed();
                ui.end_row();

                control_label(ui, "Dispersion:");
                *pt_changed |= ui
                    .checkbox(&mut self.render_3d_opts.pt_spectral_dispersion, "")
                    .changed();
                ui.end_row();
            }
        });

        self.ui_pt_restir(ui, pt_changed);
        self.ui_pt_path_guiding(ui, pt_changed);
    }

    fn ui_pt_restir(&mut self, ui: &mut egui::Ui, pt_changed: &mut bool) {
        settings_grid(ui, "pt_restir_grid", |ui| {
            control_label(ui, "ReSTIR:");
            ui.horizontal(|ui| {
                *pt_changed |= ui
                    .checkbox(&mut self.render_3d_opts.pt_restir_di, "DI")
                    .on_hover_text("Direct illumination resampling")
                    .changed();
                *pt_changed |= ui
                    .checkbox(&mut self.render_3d_opts.pt_restir_gi, "GI")
                    .on_hover_text("Global illumination resampling")
                    .changed();
            });
            ui.end_row();

            if self.render_3d_opts.pt_restir_di || self.render_3d_opts.pt_restir_gi {
                control_label(ui, "Reuse:");
                ui.horizontal(|ui| {
                    *pt_changed |= ui
                        .checkbox(&mut self.render_3d_opts.pt_restir_temporal, "Temporal")
                        .changed();
                    *pt_changed |= ui
                        .checkbox(&mut self.render_3d_opts.pt_restir_spatial, "Spatial")
                        .changed();
                });
                ui.end_row();

                control_label(ui, "M max:");
                *pt_changed |= ui
                    .add(
                        egui::DragValue::new(&mut self.render_3d_opts.pt_restir_m_max)
                            .range(1..=100)
                            .speed(1),
                    )
                    .changed();
                ui.end_row();
            }
        });
    }

    fn ui_pt_path_guiding(&mut self, ui: &mut egui::Ui, pt_changed: &mut bool) {
        settings_grid(ui, "pt_pg_grid", |ui| {
            control_label(ui, "Path Guide:");
            *pt_changed |= ui
                .checkbox(&mut self.render_3d_opts.pt_path_guiding, "")
                .on_hover_text("Learn where light comes from")
                .changed();
            ui.end_row();

            if self.render_3d_opts.pt_path_guiding {
                control_label(ui, "SVO:");
                if multibutton_exclusive(
                    ui,
                    &mut self.render_3d_opts.pt_svo_resolution,
                    &[
                        (32_u32, "32"),
                        (64_u32, "64"),
                        (128_u32, "128"),
                        (256_u32, "256"),
                    ],
                    MultiButtonAxis::Horizontal,
                ) {
                    *pt_changed = true;
                }
                ui.end_row();
            }
        });
    }

    /// Environment settings (env map, lighting)
    fn ui_3d_environment(&mut self, ui: &mut egui::Ui) {
        tinted_section(ui, "Environment", false, self.settings_tint_mix, |ui| {
            egui::Grid::new("env_grid")
                .num_columns(2)
                .spacing([8.0, 4.0])
                .min_col_width(SETTINGS_LABEL_WIDTH)
                .show(ui, |ui| {
                    control_label(ui, "Background:");
                    let mut color = egui::Color32::from_rgb(
                        (self.render_3d_opts.background_color[0] * 255.0) as u8,
                        (self.render_3d_opts.background_color[1] * 255.0) as u8,
                        (self.render_3d_opts.background_color[2] * 255.0) as u8,
                    );
                    if ui.color_edit_button_srgba(&mut color).changed() {
                        self.render_3d_opts.background_color = [
                            color.r() as f32 / 255.0,
                            color.g() as f32 / 255.0,
                            color.b() as f32 / 255.0,
                        ];
                        self.needs_layout = true;
                    }
                    ui.end_row();

                    control_label(ui, "Env Map:");
                    ui.horizontal(|ui| {
                        let old_enabled = self.render_3d_opts.env_map_enabled;
                        if ui
                            .checkbox(&mut self.render_3d_opts.env_map_enabled, "")
                            .changed()
                        {
                            if let Some(r) = &mut self.renderer_3d {
                                if self.render_3d_opts.env_map_enabled {
                                    if let Some(ref path) = self.render_3d_opts.env_map_path {
                                        if path.exists() {
                                            if let Err(e) = r.load_env_map(path) {
                                                log::error!("Env map: {e}");
                                            }
                                        }
                                    }
                                }
                                r.mark_pt_env_dirty();
                                r.reset_pt_accumulation();
                            }
                            if self.render_3d_opts.env_map_enabled != old_enabled {
                                self.needs_layout = true;
                            }
                        }
                        if self.render_3d_opts.env_map_enabled
                            && ui.small_button("Load...").clicked()
                        {
                            if let Some(path) = rfd::FileDialog::new()
                                .add_filter("Images", &["png", "jpg", "jpeg", "hdr", "exr"])
                                .pick_file()
                            {
                                if let Some(r) = &mut self.renderer_3d {
                                    if let Err(e) = r.load_env_map(&path) {
                                        log::error!("Env map: {e}");
                                    } else {
                                        self.render_3d_opts.env_map_path = Some(path);
                                    }
                                }
                            }
                        }
                    });
                    ui.end_row();

                    if self.render_3d_opts.env_map_enabled {
                        control_label(ui, "Intensity:");
                        if ui
                            .add(egui::Slider::new(
                                &mut self.render_3d_opts.env_map_intensity,
                                0.0..=5.0,
                            ))
                            .changed()
                        {
                            self.needs_layout = true;
                        }
                        ui.end_row();

                        control_label(ui, "Rotation:");
                        let mut env_deg = self.render_3d_opts.env_map_rotation.to_degrees();
                        if ui
                            .add(egui::Slider::new(&mut env_deg, -360.0..=360.0).suffix(" deg"))
                            .changed()
                        {
                            self.render_3d_opts.env_map_rotation = env_deg.to_radians();
                            self.needs_layout = true;
                        }
                        ui.end_row();
                    }
                });

            if self.render_3d_opts.env_map_enabled {
                egui::Grid::new("env_visibility_grid")
                    .num_columns(2)
                    .spacing([8.0, 4.0])
                    .min_col_width(SETTINGS_LABEL_WIDTH)
                    .show(ui, |ui| {
                        // Visibility only — env animation lives in the
                        // Animation section.
                        control_label(ui, "Visible");
                        ui.checkbox(&mut self.render_3d_opts.env_map_visible, "")
                            .on_hover_text(
                                "Show the env background while keeping its lighting contribution",
                            );
                        ui.end_row();
                    });
            }
        });
    }

    /// Interaction settings (hover highlight)
    fn ui_3d_interaction(&mut self, ui: &mut egui::Ui) {
        tinted_section(ui, "Interaction", false, self.settings_tint_mix, |ui| {
            egui::Grid::new("interaction_grid")
                .num_columns(2)
                .spacing([8.0, 4.0])
                .min_col_width(SETTINGS_LABEL_WIDTH)
                .show(ui, |ui| {
                    control_label(ui, "Hover:");
                    multibutton_exclusive(
                        ui,
                        &mut self.render_3d_opts.hover_mode,
                        &[
                            (HoverMode::None, "None"),
                            (HoverMode::Outline, "Outline"),
                            (HoverMode::Tint, "Tint"),
                            (HoverMode::Both, "Both"),
                        ],
                        MultiButtonAxis::Horizontal,
                    );
                    ui.end_row();

                    if matches!(
                        self.render_3d_opts.hover_mode,
                        HoverMode::Outline | HoverMode::Both
                    ) {
                        control_label(ui, "Width:");
                        if ui
                            .add(egui::Slider::new(
                                &mut self.render_3d_opts.hover_outline_width,
                                0.5..=5.0,
                            ))
                            .changed()
                        {
                            self.needs_layout = true;
                        }
                        ui.end_row();

                        control_label(ui, "Alpha:");
                        if ui
                            .add(egui::Slider::new(
                                &mut self.render_3d_opts.hover_outline_alpha,
                                0.1..=1.0,
                            ))
                            .changed()
                        {
                            self.needs_layout = true;
                        }
                        ui.end_row();
                    }
                });
        });
    }

    /// Camera controls
    fn ui_3d_camera(&mut self, ui: &mut egui::Ui) {
        tinted_section(ui, "Camera", false, self.settings_tint_mix, |ui| {
            egui::Grid::new("camera_grid")
                .num_columns(2)
                .spacing([8.0, 4.0])
                .min_col_width(SETTINGS_LABEL_WIDTH)
                .show(ui, |ui| {
                    control_label(ui, "Inertia:");
                    ui.checkbox(&mut self.render_3d_opts.inertia_enabled, "")
                        .on_hover_text("Enable smooth camera momentum after drag");
                    ui.end_row();

                    if self.render_3d_opts.inertia_enabled {
                        control_label(ui, "Friction:");
                        ui.add(egui::Slider::new(
                            &mut self.render_3d_opts.inertia_friction,
                            1.0..=15.0,
                        ))
                        .on_hover_text("Higher = faster stop (1=floaty, 15=responsive)");
                        ui.end_row();

                        control_label(ui, "Cutoff:");
                        ui.add(
                            egui::Slider::new(
                                &mut self.render_3d_opts.inertia_cutoff,
                                0.0001..=0.05,
                            )
                            .logarithmic(true),
                        )
                        .on_hover_text("Stop inertia when speed drops below this threshold");
                        ui.end_row();
                    }
                });

            ui.horizontal(|ui| {
                ui.small("LMB: Orbit  MMB: Pan  RMB: Zoom");
                if ui.small_button("Reset").clicked() {
                    self.orbit_camera.reset();
                    self.needs_layout = true;
                }
            });
        });
    }
}
