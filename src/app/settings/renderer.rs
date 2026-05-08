//! Renderer settings: 2D backend, 3D options.

use super::tinted_section;
use crate::app::helpers::{multibutton_exclusive, MultiButtonAxis};
use crate::app::App;
use crate::renderer::{
    AdaptivePreset, ColorMode, CubeHeightMode, FolderColorMode, GlassPreset, HashTransformEffect,
    HoverMode, PtSamplerMode, RenderMode, SpectralMode,
};
use eframe::egui;
use pt_mats::{MaterialDistribution, MaterialSource, MaterializeMode};

/// Maximum absolute PT light/glass cube count in the UI and when persisting settings.
///
/// When `total_cubes == 0` (scan not finished), the drag range must **not** fall back to `0..=1`:
/// [`egui::DragValue`] clamps existing values every frame (`clamp_existing_to_range(true)` default),
/// which was resetting saved counts to **1** on every launch before the tree existed.
const MAX_PT_MAT_CUBE_COUNT: u32 = 5000;
const SETTINGS_LABEL_WIDTH: f32 = 112.0;
const PT_VALUE_WIDTH: f32 = 58.0;

fn settings_grid(ui: &mut egui::Ui, id: &'static str, add_contents: impl FnOnce(&mut egui::Ui)) {
    egui::Grid::new(id)
        .num_columns(2)
        .spacing([8.0, 4.0])
        .min_col_width(SETTINGS_LABEL_WIDTH)
        .show(ui, add_contents);
}

fn compact_section(
    ui: &mut egui::Ui,
    title: &'static str,
    default_open: bool,
    add_contents: impl FnOnce(&mut egui::Ui),
) {
    egui::CollapsingHeader::new(title)
        .default_open(default_open)
        .show(ui, add_contents);
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
                ui.label("Mode:");
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

    /// Geometry settings (height mode and scale)
    fn ui_3d_geometry(&mut self, ui: &mut egui::Ui) {
        tinted_section(ui, "Geometry", true, self.settings_tint_mix, |ui| {
            egui::Grid::new("geometry_grid")
                .num_columns(2)
                .spacing([8.0, 4.0])
                .min_col_width(SETTINGS_LABEL_WIDTH)
                .show(ui, |ui| {
                    // Height Mode
                    ui.label("Height:");
                    let old_mode = self.render_3d_opts.height_mode;
                    let old_pow = (
                        self.render_3d_opts.height_power_enabled,
                        self.render_3d_opts.height_power,
                    );
                    ui.vertical(|ui| {
                        // Row 1: size-based modes
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
                        // Row 2: other modes
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
                        // Row 3: power slider
                        ui.horizontal(|ui| {
                            ui.checkbox(&mut self.render_3d_opts.height_power_enabled, "^")
                                .on_hover_text("Power (0.1..4.0)");
                            if self.render_3d_opts.height_power_enabled {
                                ui.add(
                                    egui::Slider::new(
                                        &mut self.render_3d_opts.height_power,
                                        0.1..=4.0,
                                    )
                                    .show_value(true),
                                );
                            }
                        });
                    });
                    let new_pow = (
                        self.render_3d_opts.height_power_enabled,
                        self.render_3d_opts.height_power,
                    );
                    if self.render_3d_opts.height_mode != old_mode || new_pow != old_pow {
                        self.needs_layout = true;
                    }
                    ui.end_row();

                    // Height Scale
                    ui.label("Scale:");
                    if ui
                        .add(egui::Slider::new(
                            &mut self.render_3d_opts.height_scale,
                            0.1..=5.0,
                        ))
                        .changed()
                    {
                        self.needs_layout = true;
                    }
                    ui.end_row();

                    // Color Mode
                    ui.label("Color:");
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

                    // Folder tint
                    ui.label("Folder tint:");
                    ui.horizontal(|ui| {
                        if ui
                            .add(
                                egui::Slider::new(&mut self.render_3d_opts.folder_tint, 0.0..=1.0)
                                    .show_value(true),
                            )
                            .changed()
                        {
                            self.needs_layout = true;
                        }
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
                    });
                    ui.end_row();
                });

            // LOD (Level of Detail)
            egui::Grid::new("geometry_lod_grid")
                .num_columns(2)
                .spacing([8.0, 4.0])
                .min_col_width(SETTINGS_LABEL_WIDTH)
                .show(ui, |ui| {
                    ui.label("LOD:");
                    ui.horizontal(|ui| {
                        if ui
                            .checkbox(&mut self.render_3d_opts.lod_enabled, "")
                            .on_hover_text(
                                "Level of Detail: skip rendering cubes smaller than threshold",
                            )
                            .changed()
                        {
                            self.needs_layout = true;
                        }
                        if self.render_3d_opts.lod_enabled {
                            ui.label("Min px:");
                            if ui
                                .add(egui::Slider::new(
                                    &mut self.render_3d_opts.lod_min_screen_size,
                                    0.5..=10.0,
                                ))
                                .changed()
                            {
                                self.needs_layout = true;
                            }
                        }
                    });
                    ui.end_row();
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
                    ui.label("Effect:");
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
                        ui.label("Strength:");
                        if ui
                            .add(egui::Slider::new(
                                &mut self.render_3d_opts.hash_effect_strength,
                                0.0..=10.0,
                            ))
                            .changed()
                        {
                            self.needs_layout = true;
                        }
                        ui.end_row();
                    }
                });

            if self.render_3d_opts.hash_effect != HashTransformEffect::None {
                egui::Grid::new("effects_anim_grid")
                    .num_columns(2)
                    .spacing([8.0, 4.0])
                    .min_col_width(SETTINGS_LABEL_WIDTH)
                    .show(ui, |ui| {
                        ui.label("Animate:");
                        ui.horizontal(|ui| {
                            if ui.checkbox(&mut self.render_3d_opts.animate, "").changed() {
                                self.needs_layout = true;
                            }
                            if self.render_3d_opts.animate {
                                ui.add(egui::Slider::new(
                                    &mut self.render_3d_opts.animation_speed,
                                    0.1..=3.0,
                                ));
                            }
                        });
                        ui.end_row();
                    });
            }

            // Slice plane controls
            ui.separator();
            egui::Grid::new("slice_enable_grid")
                .num_columns(2)
                .spacing([8.0, 4.0])
                .min_col_width(SETTINGS_LABEL_WIDTH)
                .show(ui, |ui| {
                    ui.label("Slice Plane:");
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
                        ui.label("Mode:");
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
                            ui.label("Normal:");
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
                            ui.label("Axis:");
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

                        ui.label("Distance:");
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
                        ui.label("Invert:");
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

    /// Material settings (PBR properties)
    fn ui_3d_material(&mut self, ui: &mut egui::Ui) {
        tinted_section(ui, "Material", false, self.settings_tint_mix, |ui| {
            egui::Grid::new("material_grid")
                .num_columns(2)
                .spacing([8.0, 4.0])
                .min_col_width(SETTINGS_LABEL_WIDTH)
                .show(ui, |ui| {
                    // Source: what data determines the material
                    ui.label("Source:");
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
                        // Distribution: how values map to materials
                        ui.label("Distribute:");
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
                                ui.label("Levels:");
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
                                ui.label("Bands:");
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
                                ui.label("Scale:");
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
                        ui.label("Seed:");
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
                        ui.label("Mix:");
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

                    ui.label("Roughness:");
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

                    ui.label("Metalness:");
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

                    ui.label("Specular IOR:");
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
                    ui.label("Shading:");
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
            ui.label("Source:");
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

            ui.label("Distribute:");
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
                    ui.label("Levels:");
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
                    ui.label("Bands:");
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
                    ui.label("Scale:");
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

            ui.label("Seed:");
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

            ui.label("Mix:");
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
        ui.label("Light Cubes:");
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
            ui.label("Warm Bias:");
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

            ui.label("Cool Bias:");
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

            ui.label("Light Power:");
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

            ui.label("Light Rand:");
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

        ui.label("Glass Cubes:");
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
            ui.label("Env MIS:");
            if ui
                .checkbox(&mut self.render_3d_opts.pt_env_importance_sampling, "")
                .on_hover_text("Use HDR CDF importance sampling + MIS")
                .changed()
            {
                *pt_changed = true;
                self.mark_pt_scene_dirty();
            }
            ui.end_row();

            ui.label("Emissive NEE:");
            if ui
                .checkbox(&mut self.render_3d_opts.pt_emissive_sampling, "")
                .on_hover_text("Directly sample emissive cubes")
                .changed()
            {
                *pt_changed = true;
            }
            ui.end_row();

            if self.render_3d_opts.pt_emissive_sampling {
                ui.label("Light SPP:");
                *pt_changed |= ui
                    .add(
                        egui::DragValue::new(&mut self.render_3d_opts.pt_emissive_samples)
                            .range(1..=8)
                            .speed(1),
                    )
                    .changed();
                ui.end_row();

                ui.label("Light Min:");
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
            ui.label("Max Samples:");
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

            ui.label("SPP/frame:");
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

            ui.label("Auto SPP:");
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
                ui.label("Target FPS:");
                *pt_changed |= ui
                    .add(
                        egui::Slider::new(&mut self.render_3d_opts.pt_target_fps, 1.0..=120.0)
                            .integer(),
                    )
                    .changed();
                ui.end_row();
            }

            ui.label("Sampler:");
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
            ui.label("Adaptive:");
            *pt_changed |= ui
                .checkbox(&mut self.render_3d_opts.pt_adaptive_sampling, "")
                .on_hover_text("More samples on high-variance areas")
                .changed();
            ui.end_row();

            if !self.render_3d_opts.pt_adaptive_sampling {
                return;
            }

            ui.label("Preset:");
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

            ui.label("SPP Range:");
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

            ui.label("Variance:");
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

            ui.label("Interval:");
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
            ui.label("Bounces:");
            *pt_changed |= ui
                .add(egui::Slider::new(
                    &mut self.render_3d_opts.pt_max_bounces,
                    1..=64,
                ))
                .changed();
            ui.end_row();

            ui.label("Transmission:");
            *pt_changed |= ui
                .add(egui::Slider::new(
                    &mut self.render_3d_opts.pt_max_transmission_depth,
                    1..=64,
                ))
                .changed();
            ui.end_row();

            ui.label("Russian Roulette:");
            *pt_changed |= ui
                .checkbox(&mut self.render_3d_opts.pt_russian_roulette, "")
                .on_hover_text("Probabilistic path termination")
                .changed();
            ui.end_row();
        });
    }

    fn ui_pt_glass(&mut self, ui: &mut egui::Ui, pt_changed: &mut bool) {
        settings_grid(ui, "pt_glass_grid", |ui| {
            ui.label("Transparency:");
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

            ui.label("Preset:");
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

            ui.label("Thin:");
            if ui
                .checkbox(&mut self.render_3d_opts.pt_glass_thin, "")
                .changed()
            {
                *pt_changed = true;
                self.mark_pt_scene_dirty();
            }
            ui.end_row();

            ui.label("Specular:");
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

            ui.label("Base:");
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

            ui.label("Roughness:");
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

            ui.label("IoR:");
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

            ui.label("Dispersion:");
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

            ui.label("Temperature:");
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
            ui.label("DOF:");
            *pt_changed |= ui
                .checkbox(&mut self.render_3d_opts.pt_dof_enabled, "")
                .changed();
            ui.end_row();

            if self.render_3d_opts.pt_dof_enabled {
                ui.label("Aperture:");
                *pt_changed |= ui
                    .add(egui::Slider::new(
                        &mut self.render_3d_opts.pt_aperture,
                        0.01..=2.0,
                    ))
                    .changed();
                ui.end_row();

                ui.label("Focus:");
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
            ui.label("Backend:");
            *pt_changed |= ui
                .checkbox(&mut self.render_3d_opts.pt_wavefront, "Wavefront")
                .on_hover_text("Split path tracing into separate passes")
                .changed();
            ui.end_row();

            if self.render_3d_opts.pt_wavefront {
                ui.label("WF Tile:");
                ui.horizontal(|ui| {
                    *pt_changed |= ui
                        .add(
                            egui::DragValue::new(&mut self.render_3d_opts.pt_wavefront_tile_size)
                                .range(0..=8192)
                                .speed(16),
                        )
                        .changed();
                    ui.small("0 = full frame");
                });
                ui.end_row();

                ui.label("WF Scope:");
                ui.small("R2/NEE direct use megakernel; ReSTIR uses wavefront.");
                ui.end_row();
            }

            ui.label("GPU BVH:");
            *pt_changed |= ui
                .checkbox(&mut self.render_3d_opts.pt_gpu_bvh, "")
                .on_hover_text("Build BVH on GPU")
                .changed();
            ui.end_row();

            if self.render_3d_opts.pt_gpu_bvh {
                ui.label("BVH Refit:");
                *pt_changed |= ui
                    .checkbox(&mut self.render_3d_opts.pt_bvh_refit, "")
                    .on_hover_text("Fast AABB update for animation")
                    .changed();
                ui.end_row();
            }
        });

        settings_grid(ui, "pt_spectral_grid", |ui| {
            ui.label("Spectral:");
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
                ui.label("Spectral SPP:");
                *pt_changed |= ui
                    .add(egui::Slider::new(
                        &mut self.render_3d_opts.pt_spectral_samples,
                        1..=8,
                    ))
                    .changed();
                ui.end_row();

                ui.label("Dispersion:");
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
            ui.label("ReSTIR:");
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
                ui.label("Reuse:");
                ui.horizontal(|ui| {
                    *pt_changed |= ui
                        .checkbox(&mut self.render_3d_opts.pt_restir_temporal, "Temporal")
                        .changed();
                    *pt_changed |= ui
                        .checkbox(&mut self.render_3d_opts.pt_restir_spatial, "Spatial")
                        .changed();
                });
                ui.end_row();

                ui.label("M max:");
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
            ui.label("Path Guide:");
            *pt_changed |= ui
                .checkbox(&mut self.render_3d_opts.pt_path_guiding, "")
                .on_hover_text("Learn where light comes from")
                .changed();
            ui.end_row();

            if self.render_3d_opts.pt_path_guiding {
                ui.label("SVO:");
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
                    ui.label("Background:");
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

                    ui.label("Env Map:");
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
                        ui.label("Intensity:");
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

                        ui.label("Rotation:");
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
                egui::Grid::new("env_anim_grid")
                    .num_columns(2)
                    .spacing([8.0, 4.0])
                    .min_col_width(SETTINGS_LABEL_WIDTH)
                    .show(ui, |ui| {
                        ui.label("Env Anim:");
                        ui.horizontal(|ui| {
                            ui.checkbox(&mut self.render_3d_opts.env_map_visible, "Visible")
                                .on_hover_text("Show the environment background while keeping lighting enabled");
                            ui.checkbox(&mut self.render_3d_opts.env_animate, "Animate");
                            if self.render_3d_opts.env_animate {
                                ui.add(egui::Slider::new(&mut self.render_3d_opts.env_speed, 0.1..=5.0));
                            }
                        });
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
                    ui.label("Hover:");
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
                        ui.label("Width:");
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

                        ui.label("Alpha:");
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
                    ui.label("Inertia:");
                    ui.checkbox(&mut self.render_3d_opts.inertia_enabled, "")
                        .on_hover_text("Enable smooth camera momentum after drag");
                    ui.end_row();

                    if self.render_3d_opts.inertia_enabled {
                        ui.label("Friction:");
                        ui.add(egui::Slider::new(
                            &mut self.render_3d_opts.inertia_friction,
                            1.0..=15.0,
                        ))
                        .on_hover_text("Higher = faster stop (1=floaty, 15=responsive)");
                        ui.end_row();

                        ui.label("Cutoff:");
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
