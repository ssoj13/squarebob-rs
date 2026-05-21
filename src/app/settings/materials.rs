//! Materials editor — split across two surfaces:
//!
//! * [`materials_browser_section`] — inline subsection rendered inside
//!   the existing "Materials" tinted section in `settings/renderer.rs`.
//!   Slot list (colour swatch + name + active highlight), add /
//!   duplicate / remove buttons, JSON save / load.
//!
//! * [`App::ui_attribute_editor`] — dock tab rendered via the
//!   `playa-ae` generic Attribute Editor. Bridges the active material
//!   to a typed `playa_ae::Attrs` instance, runs the table renderer,
//!   then writes the modified values back into the material.
//!
//! Mutations flow straight into `App.render_3d_opts.material_library`.
//! PBR re-uploads `materials_buf` each frame and PT invalidates its
//! per-cube expansion cache via UUID + params hashing — so edits show
//! up live in both pipelines.

use eframe::egui;
use playa_ae::{AttrDef, AttrFlags, AttrSchema, AttrType, AttrValue, Attrs, FLAG_DISPLAY};
use pt_material::{Material, StandardSurfaceParams, io as mat_io};
use std::path::PathBuf;
use std::sync::OnceLock;

use super::super::App;
use crate::events::MaterialsChangedEvent;

// ============================================================================
// Material schema for the Attribute Editor — drives row order + UI hints
// ============================================================================

/// Static `AttrSchema` for `StandardSurfaceParams`. Built lazily on
/// first use and pinned via `OnceLock` so the `&'static AttrSchema`
/// reference required by `Attrs::with_schema` outlives the program.
fn material_schema() -> &'static AttrSchema {
    static SCHEMA: OnceLock<AttrSchema> = OnceLock::new();
    SCHEMA.get_or_init(|| {
        // Scalar-only schema: the seven colour-weight Vec4 fields
        // (Base Color, Specular, …) are now rendered above the AE
        // by `color_rows()` with the Maya-style egui-colorpicker
        // widget — they no longer go through playa-ae's generic
        // Vec4 row. The AE here owns just the params1/params2
        // scalar lanes (roughness / metalness / IOR / etc.).
        const FLOAT_FLAGS: AttrFlags = FLAG_DISPLAY;
        AttrSchema::new(
            "StandardSurfaceParams",
            &[
                AttrDef::with_ui_order(
                    "Diffuse Roughness",
                    AttrType::Float,
                    FLOAT_FLAGS,
                    &["0.0", "1.0", "0.01"],
                    10.0,
                ),
                AttrDef::with_ui_order(
                    "Metalness",
                    AttrType::Float,
                    FLOAT_FLAGS,
                    &["0.0", "1.0", "0.01"],
                    11.0,
                ),
                AttrDef::with_ui_order(
                    "Specular Roughness",
                    AttrType::Float,
                    FLOAT_FLAGS,
                    &["0.0", "1.0", "0.01"],
                    12.0,
                ),
                AttrDef::with_ui_order(
                    "Specular IOR",
                    AttrType::Float,
                    FLOAT_FLAGS,
                    &["1.0", "3.0", "0.01"],
                    13.0,
                ),
                AttrDef::with_ui_order(
                    "Spec Anisotropy",
                    AttrType::Float,
                    FLOAT_FLAGS,
                    &["0.0", "1.0", "0.01"],
                    14.0,
                ),
                AttrDef::with_ui_order(
                    "Coat Roughness",
                    AttrType::Float,
                    FLOAT_FLAGS,
                    &["0.0", "1.0", "0.01"],
                    15.0,
                ),
                AttrDef::with_ui_order(
                    "Coat IOR",
                    AttrType::Float,
                    FLOAT_FLAGS,
                    &["1.0", "3.0", "0.01"],
                    16.0,
                ),
            ],
        )
    })
}

/// Pull a `StandardSurfaceParams` into a fresh `Attrs` keyed by the
/// names used in [`material_schema`]. Vec4 colour-weights are uploaded
/// as `AttrValue::Vec4` directly; scalar packs are split into
/// individually-named `Float` entries so the AE row labels are
/// meaningful.
fn material_to_attrs(p: &StandardSurfaceParams) -> Attrs {
    let mut a = Attrs::with_schema(material_schema());
    // Vec4 colour-weights deliberately omitted — the colour rows
    // are rendered separately by `color_rows()` with the picker
    // widget, not via playa-ae.
    a.set("Diffuse Roughness", AttrValue::Float(p.params1.x));
    a.set("Metalness", AttrValue::Float(p.params1.y));
    a.set("Specular Roughness", AttrValue::Float(p.params1.z));
    a.set("Specular IOR", AttrValue::Float(p.params1.w));
    a.set("Spec Anisotropy", AttrValue::Float(p.params2.x));
    a.set("Coat Roughness", AttrValue::Float(p.params2.y));
    a.set("Coat IOR", AttrValue::Float(p.params2.z));
    a.clear_dirty();
    a
}

/// Apply edits from an `Attrs` (post-AE render) back to the source
/// `StandardSurfaceParams`. Missing keys leave the corresponding field
/// untouched, so partial schemas are tolerated.
fn attrs_to_material(a: &Attrs, p: &mut StandardSurfaceParams) {
    // Vec4 colour-weights are written directly by `color_rows()`
    // — playa-ae only carries the scalar params now, so this
    // function only reads back the scalar keys.
    if let Some(v) = a.get_float("Diffuse Roughness") {
        p.params1.x = v;
    }
    if let Some(v) = a.get_float("Metalness") {
        p.params1.y = v;
    }
    if let Some(v) = a.get_float("Specular Roughness") {
        p.params1.z = v;
    }
    if let Some(v) = a.get_float("Specular IOR") {
        p.params1.w = v;
    }
    if let Some(v) = a.get_float("Spec Anisotropy") {
        p.params2.x = v;
    }
    if let Some(v) = a.get_float("Coat Roughness") {
        p.params2.y = v;
    }
    if let Some(v) = a.get_float("Coat IOR") {
        p.params2.z = v;
    }
}

// ============================================================================
// Inline subsection: slot browser (lives in Settings → Materials section)
// ============================================================================

/// Slot browser embedded in the existing `tinted_section "Materials"`
/// in `settings/renderer.rs`. Toolbar (New / Duplicate / Remove / Load
/// / Save / Save As) above a scroll-capped list of slots; each row
/// shows a colour swatch + name with double-click rename.
pub(super) fn materials_browser_section(app: &mut App, ui: &mut egui::Ui) {
    materials_toolbar(ui, app);
    ui.add_space(4.0);
    egui::ScrollArea::vertical()
        .id_salt("materials_list_scroll")
        .max_height(160.0)
        .show(ui, |ui| {
            materials_list(ui, app);
        });
    materials_footer(ui, app);
}

/// Bulk-weight footer row placed *under* the slot list (the top
/// toolbar has no horizontal room left for additional buttons).
/// A `weight == 0.0` slot is effectively excluded from the per-cube
/// distribution, so "All off" disables every material at once and
/// "All on" restores the uniform 1.0 weight. Disabled when the
/// library is empty so the buttons cannot mutate a zero-length
/// vector.
fn materials_footer(ui: &mut egui::Ui, app: &mut App) {
    let mut library_dirty = false;
    {
        let lib = &mut app.render_3d_opts.material_library;
        ui.add_space(4.0);
        ui.horizontal(|ui| {
            ui.add_enabled_ui(!lib.is_empty(), |ui| {
                if ui
                    .button("All on")
                    .on_hover_text("Set every material weight to 1.0 (include all in distribution)")
                    .clicked()
                {
                    for mat in &mut lib.materials {
                        mat.weight = 1.0;
                    }
                    library_dirty = true;
                }
                if ui
                    .button("All off")
                    .on_hover_text("Set every material weight to 0.0 (exclude all from distribution)")
                    .clicked()
                {
                    for mat in &mut lib.materials {
                        mat.weight = 0.0;
                    }
                    library_dirty = true;
                }
            });
        });
    }
    if library_dirty {
        app.events.emit(MaterialsChangedEvent);
    }
}

// ============================================================================
// Dock tab: full Attribute Editor for the active material
// ============================================================================

impl App {
    /// Attribute Editor dock tab — rebuilds an `Attrs` from the active
    /// material each frame, runs `playa_ae::render`, applies the edits
    /// back to the material slot. The rebuild is cheap (14 keys); doing
    /// it per-frame keeps the AE in sync with external edits (Settings
    /// → Materials presets, file load, etc.) without a manual refresh.
    pub(crate) fn ui_attribute_editor(&mut self, ui: &mut egui::Ui) {
        let lib = &mut self.render_3d_opts.material_library;
        let Some(active_idx) = (lib.active < lib.materials.len()).then_some(lib.active) else {
            ui.label("No active material — open Settings → Materials and add or select a slot.");
            return;
        };
        let mat: &mut Material = &mut lib.materials[active_idx];

        ui.label(egui::RichText::new(&mat.name).heading());
        ui.label(egui::RichText::new(format!("uuid: {}", mat.uuid)).weak().small());
        ui.separator();

        // Colour rows go through the Maya-style picker widget; the
        // remaining scalar lanes (roughness / metalness / IOR / …)
        // still ride playa-ae for free table layout + sliders.
        let mut any_changed = color_rows(ui, &mut mat.params);

        ui.separator();
        let mut attrs = material_to_attrs(&mat.params);
        if playa_ae::render(ui, &mut attrs, &mut self.materials_ae_state, "Material") {
            attrs_to_material(&attrs, &mut mat.params);
            any_changed = true;
        }
        if any_changed {
            self.events.emit(MaterialsChangedEvent);
        }
    }
}

/// Maya-style colour pickers for the seven `StandardSurfaceParams`
/// Vec4 lanes. Each row shows the lane label, the swatch button
/// (click → popup), and the four numeric `DragValue`s so direct
/// keyboard input still works for power-users. Returns `true` when
/// any field changed.
fn color_rows(ui: &mut egui::Ui, p: &mut StandardSurfaceParams) -> bool {
    let mut changed = false;
    egui::Grid::new("material_color_grid")
        .num_columns(2)
        .spacing([6.0, 2.0])
        .show(ui, |ui| {
            changed |= color_row(ui, "Base Color", &mut p.base_color_weight);
            changed |= color_row(ui, "Specular", &mut p.specular_color_weight);
            changed |= color_row(ui, "Transmission", &mut p.transmission_color_weight);
            changed |= color_row(ui, "Subsurface", &mut p.subsurface_color_weight);
            changed |= color_row(ui, "Coat", &mut p.coat_color_weight);
            changed |= color_row(ui, "Emission", &mut p.emission_color_weight);
            changed |= color_row(ui, "Opacity", &mut p.opacity);
        });
    changed
}

/// One `StandardSurfaceParams` colour-weight row: label · swatch
/// button · `x` · `y` · `z` · `w` (weight) `DragValue`s. Edits in
/// either the picker or the numeric inputs propagate to `*v`.
fn color_row(ui: &mut egui::Ui, label: &str, v: &mut glam::Vec4) -> bool {
    let mut changed = false;
    ui.label(label);
    ui.horizontal(|ui| {
        let mut tmp = v.to_array();
        let before = tmp;
        let _ = egui_colorpicker::color_button(ui, &mut tmp);
        if tmp != before {
            *v = glam::Vec4::from(tmp);
            changed = true;
        }
        // Direct numeric edit kept alongside the picker — useful
        // when the user needs an exact HDR value the picker
        // sliders won't easily hit.
        let mut x = v.x;
        let mut y = v.y;
        let mut z = v.z;
        let mut w = v.w;
        let mut row_changed = false;
        row_changed |= ui
            .add(egui::DragValue::new(&mut x).speed(0.01).max_decimals(3))
            .changed();
        row_changed |= ui
            .add(egui::DragValue::new(&mut y).speed(0.01).max_decimals(3))
            .changed();
        row_changed |= ui
            .add(egui::DragValue::new(&mut z).speed(0.01).max_decimals(3))
            .changed();
        row_changed |= ui
            .add(
                egui::DragValue::new(&mut w)
                    .speed(0.01)
                    .range(0.0..=1.0)
                    .max_decimals(3),
            )
            .changed();
        if row_changed {
            *v = glam::Vec4::new(x, y, z, w);
            changed = true;
        }
    });
    ui.end_row();
    changed
}

// ============================================================================
// Internal helpers — slot toolbar + list + rename
// ============================================================================

/// Toolbar row: slot ops + file I/O. Scopes the library borrow before
/// the rfd dialog calls so `app` isn't borrowed twice. Any mutation
/// that changes the library contents emits a `MaterialsChangedEvent`
/// so the renderer resets PT accumulation and forces a fresh frame.
fn materials_toolbar(ui: &mut egui::Ui, app: &mut App) {
    let mut library_dirty = false;
    ui.horizontal(|ui| {
        {
            let lib = &mut app.render_3d_opts.material_library;
            let has_active = lib.active < lib.materials.len() && !lib.is_empty();

            if ui
                .button("+ New")
                .on_hover_text("Append a default material slot")
                .clicked()
            {
                let name = format!("material_{}", lib.materials.len());
                let idx = lib.push(Material::new(name, StandardSurfaceParams::default()));
                lib.set_active(idx);
                library_dirty = true;
            }

            ui.add_enabled_ui(has_active, |ui| {
                if ui
                    .button("Duplicate")
                    .on_hover_text("Copy active slot to the end of the library")
                    .clicked()
                    && let Some(idx) = lib.duplicate(lib.active)
                {
                    lib.set_active(idx);
                    library_dirty = true;
                }
                if ui
                    .button("Remove")
                    .on_hover_text("Remove active slot (refuses to empty the library)")
                    .clicked()
                {
                    lib.remove(lib.active);
                    library_dirty = true;
                }
            });
        }

        ui.separator();

        if ui
            .button("Load…")
            .on_hover_text("Load library from JSON file")
            .clicked()
            && let Some(path) = rfd_pick_open_file()
        {
            match mat_io::load_library(&path) {
                Ok(loaded) => {
                    app.render_3d_opts.material_library = loaded;
                    app.materials_last_save_path = Some(path);
                    library_dirty = true;
                }
                Err(e) => log::error!("Failed to load library: {e}"),
            }
        }

        if ui
            .button("Save As…")
            .on_hover_text("Save library to a new JSON file")
            .clicked()
            && let Some(path) = rfd_pick_save_file()
        {
            if let Err(e) = mat_io::save_library(&app.render_3d_opts.material_library, &path) {
                log::error!("Failed to save library: {e}");
            } else {
                app.materials_last_save_path = Some(path);
            }
        }

        if let Some(path) = app.materials_last_save_path.clone()
            && ui
                .button("Save")
                .on_hover_text(path.display().to_string())
                .clicked()
            && let Err(e) = mat_io::save_library(&app.render_3d_opts.material_library, &path)
        {
            log::error!("Failed to save library: {e}");
        }
    });
    if library_dirty {
        app.events.emit(MaterialsChangedEvent);
    }
}

/// Slot list — `egui::Grid` keeps 3 columns aligned across rows even
/// when slider widths fluctuate: [swatch | name | weight].
/// Single-click name selects, double-click starts in-place rename.
/// Trailing weight slider (0.0..=10.0, default 1.0) drives the
/// per-cube distribution — values normalise to sum=1.0 at classify
/// time, so the absolute scale doesn't matter, only ratios.
fn materials_list(ui: &mut egui::Ui, app: &mut App) {
    let mut to_select: Option<usize> = None;
    let mut to_start_rename: Option<(uuid::Uuid, String)> = None;
    let mut to_commit_rename: Option<(uuid::Uuid, String)> = None;
    let mut to_cancel_rename = false;
    let mut to_set_weight: Option<(uuid::Uuid, f32)> = None;

    // Three fixed columns; trailing space goes to the slider so it
    // stretches to the available width without pushing the label out
    // of view. The label column shrinks to its content (selectable
    // labels auto-size in egui), keeping rows compact.
    let total_w = ui.available_width();
    let swatch_w = 18.0;
    let name_min_w: f32 = 120.0;
    let gap = 6.0;
    let slider_w = (total_w - swatch_w - name_min_w - gap * 3.0).clamp(80.0, 220.0);

    let lib = &mut app.render_3d_opts.material_library;
    let active = lib.active;

    egui::Grid::new("materials_list_grid")
        .num_columns(3)
        .spacing([gap, 2.0])
        .show(ui, |ui| {
            for (i, mat) in lib.materials.iter().enumerate() {
                let selected = i == active;
                let base = mat.params.base_color_weight;
                let swatch = egui::Color32::from_rgb(
                    (base.x.clamp(0.0, 1.0) * 255.0) as u8,
                    (base.y.clamp(0.0, 1.0) * 255.0) as u8,
                    (base.z.clamp(0.0, 1.0) * 255.0) as u8,
                );

                // Col 1: swatch
                let (rect, _resp) = ui.allocate_exact_size(
                    egui::Vec2::new(swatch_w - 4.0, 14.0),
                    egui::Sense::hover(),
                );
                ui.painter()
                    .rect_filled(rect, egui::CornerRadius::same(2), swatch);

                // Col 2: name / rename — natural-width selectable
                // label (auto-shrinks to text). The min-width column
                // hint on the grid keeps short names from collapsing
                // the column completely.
                ui.scope(|ui| {
                    ui.set_min_width(name_min_w);
                    let in_rename = matches!(
                        &app.materials_rename_buffer,
                        Some((uuid, _)) if *uuid == mat.uuid
                    );
                    if in_rename {
                        if let Some((_, text)) = &mut app.materials_rename_buffer {
                            let resp = ui.add(
                                egui::TextEdit::singleline(text)
                                    .desired_width(name_min_w - 4.0)
                                    .id_salt(("materials_rename", mat.uuid)),
                            );
                            resp.request_focus();
                            if resp.lost_focus() {
                                if ui.input(|i| i.key_pressed(egui::Key::Escape)) {
                                    to_cancel_rename = true;
                                } else {
                                    to_commit_rename = Some((mat.uuid, text.clone()));
                                }
                            }
                        }
                    } else {
                        let resp = ui.selectable_label(selected, &mat.name);
                        if resp.clicked() {
                            to_select = Some(i);
                        }
                        if resp.double_clicked() {
                            to_start_rename = Some((mat.uuid, mat.name.clone()));
                        }
                    }
                });

                // Col 3: weight slider, fixed width via add_sized.
                let mut w = mat.weight;
                let resp = ui.add_sized(
                    [slider_w, 18.0],
                    egui::Slider::new(&mut w, 0.0..=10.0)
                        .text("")
                        .clamping(egui::SliderClamping::Always)
                        .show_value(true),
                );
                if resp.changed() {
                    to_set_weight = Some((mat.uuid, w));
                }
                resp.on_hover_text(
                    "Distribution weight — slots normalise to sum 1.0; 0 excludes this slot",
                );

                ui.end_row();
            }
        });

    if let Some(i) = to_select {
        lib.set_active(i);
    }
    // Capture this BEFORE the consume below — used by the change-event
    // gate at the bottom of the function.
    let rename_committed = to_commit_rename.is_some();
    if let Some((uuid, new_name)) = to_commit_rename {
        if let Some((_, m)) = lib.find_by_uuid_mut(uuid) {
            m.name = new_name;
        }
        app.materials_rename_buffer = None;
    }
    if to_cancel_rename {
        app.materials_rename_buffer = None;
    }
    if let Some(entry) = to_start_rename {
        app.materials_rename_buffer = Some(entry);
    }
    let mut weight_changed = false;
    if let Some((uuid, w)) = to_set_weight
        && let Some((_, m)) = lib.find_by_uuid_mut(uuid)
    {
        m.weight = w;
        weight_changed = true;
    }
    if weight_changed || rename_committed {
        // Weight directly drives the per-cube distribution → PT
        // accumulation must reset. Rename is cosmetic but cheap to
        // emit; keeps the cube cache fingerprint coherent in case
        // anything downstream hashes by name.
        app.events.emit(MaterialsChangedEvent);
    }
}

// --- File-dialog wrappers ---

fn rfd_pick_open_file() -> Option<PathBuf> {
    rfd::FileDialog::new()
        .add_filter("Materials JSON", &["json"])
        .pick_file()
}

fn rfd_pick_save_file() -> Option<PathBuf> {
    rfd::FileDialog::new()
        .add_filter("Materials JSON", &["json"])
        .set_file_name("library.json")
        .save_file()
}
