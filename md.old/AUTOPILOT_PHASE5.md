# Autopilot Plan — Phase 5: Materials Editor Panel + Toolbar Button

**Goal**: Ship a user-facing Materials editor for the
`pt_material::MaterialLibrary` that already lives inside
`Render3DOptions.material_library`. After this lands the user can:

1. Open a "Materials" dock panel via a toolbar toggle.
2. See a left-side list of every material slot in the active library
   (name + colour swatch + selected highlight).
3. Add / duplicate / remove / rename slots.
4. Edit the active material's `StandardSurfaceParams` field-by-field
   via `squarebob_widgets::{VariableF32, VariableVec3, VariableColor}`,
   with per-attribute variance on the triangle-expander.
5. Save / Load the whole library to / from JSON via
   `pt_material::io::{save_library, load_library}`.

Edits are immediately visible in PBR and PT because:
- `App.render_3d_opts.material_library` is the single source of truth
  (Phase 4 wiring).
- PBR re-uploads `materials_buf` from `opts.material_library.materials[].params`
  every frame in `Renderer3D::update_uniforms`.
- PT `pt_expand_opts_hash` hashes UUID + params + variance, so the
  per-cube expansion cache invalidates as soon as the editor mutates a
  field.
- `mat_settings_hash` hashes every UUID, so classification cache
  invalidates on add / remove / reorder.

**Scope**: ~500–650 LoC across 4 files. New file ~400 LoC; toolbar
patch ~25 LoC; dock patch ~30 LoC; state patch ~30 LoC. No
crate-boundary changes (`squarebob-widgets` + `pt-material` already
deps of squarebob-rs via `render-shared`).

---

## Pre-flight (3 min)

- [ ] Phase 4 merged (already on `main` per commit history).
- [ ] `cargo build -p squarebob-rs --release` passes.
- [ ] `Render3DOptions.material_library` resolves to
      `pt_material::MaterialLibrary`.
- [ ] `squarebob-widgets` exposes `VariableF32`, `VariableVec3`,
      `VariableColor`, `VariableState`.

```
cd C:/projects/projects.rust.cg/squarebob-rs && python bootstrap.py b
```
Expect exit 0.

---

## Phase A — App state plumbing (15 min)

### A1. `src/app/state.rs`

Add fields next to the other `show_*` booleans:

```rust
// Materials editor — toolbar toggle wires this; dock tab follows.
// Default: closed (avoids stealing screen real-estate on first launch).
pub show_materials: bool,                  // public mirror (preset / restore)
pub(super) materials_variable_state:       // collapse memory for the
    squarebob_widgets::VariableState,      //   variance triangles per field
pub(super) materials_last_save_path:       // remembered for the
    Option<std::path::PathBuf>,            //   re-save shortcut
pub(super) materials_rename_buffer:        // None when not renaming; Some(uuid, text)
    Option<(uuid::Uuid, String)>,          //   when an in-place rename is active
```

`Default for App` initialises: `show_materials: false`,
`materials_variable_state: Default::default()`,
`materials_last_save_path: None`, `materials_rename_buffer: None`.

`pub(super)` mirror in the `(crate-internal) struct App { ... }` block
must match the public field list at the same time (state.rs has two
parallel field lists — read both before editing). Note: only
`show_materials` is part of the persisted preset surface; the other
three are session-only ephemeral state.

### A2. `Cargo.toml` (squarebob-rs root)

Verify `squarebob-widgets` and `pt-material` are already dependencies
(they are — both are reached transitively via `render-shared`, but the
new module imports them by name so they must be direct deps too):

```toml
squarebob-widgets = { path = "crates/squarebob-widgets" }
pt-material = { path = "crates/pt-material" }
uuid = { version = "1", features = ["v4"] }  # if not already direct
```

If absent, add. Run a quick `cargo build --release` after to confirm
no resolution surprises.

---

## Phase B — Materials panel module (60–80 min)

### B1. Create `src/app/settings/materials.rs`

Single new file. Skeleton:

```rust
//! Materials editor panel.
//!
//! Two-column layout: a left-hand list of every material slot in
//! `opts.material_library`, and a right-hand attribute editor for the
//! active material. The active slot is shared state on
//! `MaterialLibrary.active`; clicking a row updates it.
//!
//! Edits flow directly into `App.render_3d_opts.material_library`.
//! PBR and PT pick the changes up automatically because the library
//! is the single source of truth (see Phase 4 wiring); this module
//! never touches GPU state directly.

use eframe::egui;
use pt_material::{io as mat_io, Material, MaterialLibrary, StandardSurfaceParams};
use squarebob_widgets::{
    VariableColor, VariableF32, VariableState, Widget as _, // re-export not needed; use full path
};

use super::super::helpers::rfd_pick_folder; // re-uses existing folder picker
use super::super::App;

impl App {
    /// Top-level panel entry. Mirrors `ui_settings`, `ui_tree_panel`,
    /// etc. — called from the dock tab dispatch.
    pub(crate) fn ui_materials(&mut self, ui: &mut egui::Ui) {
        // Toolbar row: New / Duplicate / Remove / Load / Save / Save As
        materials_toolbar(ui, self);

        ui.separator();

        // Split view: list (left) + editor (right). Use a side-panel
        // inside the tab so the list keeps its width on resize.
        egui::SidePanel::left("materials_list")
            .resizable(true)
            .default_width(180.0)
            .show_inside(ui, |ui| {
                materials_list(ui, self);
            });

        egui::CentralPanel::default().show_inside(ui, |ui| {
            materials_editor(ui, self);
        });
    }
}

/// Top toolbar row: slot ops + file I/O.
fn materials_toolbar(ui: &mut egui::Ui, app: &mut App) {
    ui.horizontal(|ui| {
        let lib = &mut app.render_3d_opts.material_library;

        if ui.button("+ New").on_hover_text("Add a default material slot").clicked() {
            let idx = lib.push(Material::new(
                format!("material_{}", lib.materials.len()),
                StandardSurfaceParams::default(),
            ));
            lib.set_active(idx);
        }

        let has_active = lib.active < lib.materials.len();
        ui.add_enabled_ui(has_active, |ui| {
            if ui.button("Duplicate").clicked() {
                if let Some(idx) = lib.duplicate(lib.active) {
                    lib.set_active(idx);
                }
            }
            if ui.button("Remove").clicked() {
                lib.remove(lib.active);
            }
        });

        ui.separator();

        if ui.button("Load…").on_hover_text("Load library from JSON").clicked() {
            if let Some(path) = rfd_pick_open_file("Materials JSON", &["json"]) {
                match mat_io::load_library(&path) {
                    Ok(loaded) => {
                        app.render_3d_opts.material_library = loaded;
                        app.materials_last_save_path = Some(path);
                    }
                    Err(e) => log::error!("Failed to load library: {}", e),
                }
            }
        }
        if ui.button("Save As…").clicked() {
            if let Some(path) = rfd_pick_save_file("Materials JSON", "library.json", &["json"]) {
                if let Err(e) = mat_io::save_library(&app.render_3d_opts.material_library, &path) {
                    log::error!("Failed to save library: {}", e);
                } else {
                    app.materials_last_save_path = Some(path);
                }
            }
        }
        if let Some(path) = app.materials_last_save_path.clone() {
            if ui.button("Save").on_hover_text(path.display().to_string()).clicked() {
                if let Err(e) = mat_io::save_library(&app.render_3d_opts.material_library, &path) {
                    log::error!("Failed to save library: {}", e);
                }
            }
        }
    });
}

/// Left-pane slot list. Each row: colour swatch + name (or rename
/// text-edit) + active-highlight.
fn materials_list(ui: &mut egui::Ui, app: &mut App) {
    egui::ScrollArea::vertical().show(ui, |ui| {
        let lib = &mut app.render_3d_opts.material_library;
        let active = lib.active;

        // Indexed iteration so we can both compare against `active`
        // and borrow `&mut` for rename. We avoid `iter_mut` because
        // the rename buffer (on `app`) needs disjoint borrow.
        let mut to_select: Option<usize> = None;
        let mut to_commit_rename: Option<(uuid::Uuid, String)> = None;
        for (i, mat) in lib.materials.iter().enumerate() {
            let selected = i == active;
            let base = mat.params.base_color_weight;
            let swatch = egui::Color32::from_rgb(
                (base.x.clamp(0.0, 1.0) * 255.0) as u8,
                (base.y.clamp(0.0, 1.0) * 255.0) as u8,
                (base.z.clamp(0.0, 1.0) * 255.0) as u8,
            );

            ui.horizontal(|ui| {
                let (rect, _resp) = ui.allocate_exact_size(
                    egui::Vec2::splat(14.0),
                    egui::Sense::hover(),
                );
                ui.painter().rect_filled(rect, 2.0, swatch);

                // Rename mode: text-edit replaces label when this row
                // matches the rename buffer's uuid.
                let in_rename = matches!(
                    &app.materials_rename_buffer,
                    Some((uuid, _)) if *uuid == mat.uuid
                );
                if in_rename {
                    if let Some((_, text)) = &mut app.materials_rename_buffer {
                        let resp = ui.add(
                            egui::TextEdit::singleline(text)
                                .desired_width(140.0)
                                .id_source(("materials_rename", mat.uuid)),
                        );
                        if resp.lost_focus() {
                            to_commit_rename = Some((mat.uuid, text.clone()));
                        }
                    }
                } else {
                    let resp = ui.selectable_label(selected, &mat.name);
                    if resp.clicked() {
                        to_select = Some(i);
                    }
                    if resp.double_clicked() {
                        app.materials_rename_buffer = Some((mat.uuid, mat.name.clone()));
                    }
                }
            });
        }
        if let Some(i) = to_select {
            lib.set_active(i);
        }
        if let Some((uuid, new_name)) = to_commit_rename {
            if let Some((_, m)) = lib.find_by_uuid_mut(uuid) {
                m.name = new_name;
            }
            app.materials_rename_buffer = None;
        }
    });
}

/// Right-pane attribute editor for the active material. Uses
/// `VariableColor` for the 6 colour-weight fields and `VariableF32`
/// for the scalar params; per-field variance feeds straight into
/// `Material::variance`.
fn materials_editor(ui: &mut egui::Ui, app: &mut App) {
    let state = &mut app.materials_variable_state;
    let lib = &mut app.render_3d_opts.material_library;
    let Some(active_idx) = (lib.active < lib.materials.len()).then_some(lib.active) else {
        ui.label("No active material — add one via the + New button.");
        return;
    };
    let mat = &mut lib.materials[active_idx];

    egui::ScrollArea::vertical().show(ui, |ui| {
        ui.heading(&mat.name);
        ui.label(format!("uuid: {}", mat.uuid));
        ui.separator();

        // Colours — 6 of them, all share the same shape (rgb + weight).
        color_weight_row(ui, "Base Color",   "base",   &mut mat.params.base_color_weight,   &mut mat.variance.base_color_weight,   state);
        color_weight_row(ui, "Specular",     "spec",   &mut mat.params.specular_color_weight, &mut mat.variance.specular_color_weight, state);
        color_weight_row(ui, "Transmission", "trans",  &mut mat.params.transmission_color_weight, &mut mat.variance.transmission_color_weight, state);
        color_weight_row(ui, "Subsurface",   "subsur", &mut mat.params.subsurface_color_weight,   &mut mat.variance.subsurface_color_weight,   state);
        color_weight_row(ui, "Coat",         "coat",   &mut mat.params.coat_color_weight,   &mut mat.variance.coat_color_weight,   state);
        color_weight_row(ui, "Emission",     "emit",   &mut mat.params.emission_color_weight, &mut mat.variance.emission_color_weight, state);

        ui.separator();

        // Opacity is rgb-only (no weight).
        ui.add(VariableColor::new(
            "opacity", "Opacity",
            unsafe { &mut *(mat.params.opacity.as_mut() as *mut [f32] as *mut [f32; 3]) },
            state,
        ).with_variance(unsafe { &mut *(mat.variance.opacity.as_mut() as *mut [f32] as *mut [f32; 3]) }));

        ui.separator();

        // Scalars packed into params1 / params2 — label them
        // explicitly per the docstring on StandardSurfaceParams.
        scalar_row(ui, "Diffuse Roughness", "diff_rough", &mut mat.params.params1[0], &mut mat.variance.params1[0], 0.0..=1.0, state);
        scalar_row(ui, "Metalness",         "metal",      &mut mat.params.params1[1], &mut mat.variance.params1[1], 0.0..=1.0, state);
        scalar_row(ui, "Specular Rough",    "spec_rough", &mut mat.params.params1[2], &mut mat.variance.params1[2], 0.0..=1.0, state);
        scalar_row(ui, "Specular IOR",      "spec_ior",   &mut mat.params.params1[3], &mut mat.variance.params1[3], 1.0..=3.0, state);
        scalar_row(ui, "Spec Anisotropy",   "spec_aniso", &mut mat.params.params2[0], &mut mat.variance.params2[0], 0.0..=1.0, state);
        scalar_row(ui, "Coat Roughness",    "coat_rough", &mut mat.params.params2[1], &mut mat.variance.params2[1], 0.0..=1.0, state);
        scalar_row(ui, "Coat IOR",          "coat_ior",   &mut mat.params.params2[2], &mut mat.variance.params2[2], 1.0..=3.0, state);
    });
}

/// One row: colour picker + variance triangle for an
/// `[r, g, b, weight]` vec4. Implemented as two adjacent widgets so
/// the weight gets its own slider (variance + value).
fn color_weight_row(
    ui: &mut egui::Ui,
    label: &str,
    id_prefix: &str,
    value: &mut glam::Vec4,
    variance: &mut glam::Vec4,
    state: &mut VariableState,
) {
    ui.horizontal(|ui| {
        let mut rgb = [value.x, value.y, value.z];
        let mut var_rgb = [variance.x, variance.y, variance.z];
        let resp = ui.add(
            VariableColor::new(&format!("{}_rgb", id_prefix), label, &mut rgb, state)
                .with_variance(&mut var_rgb),
        );
        if resp.changed() {
            value.x = rgb[0]; value.y = rgb[1]; value.z = rgb[2];
            variance.x = var_rgb[0]; variance.y = var_rgb[1]; variance.z = var_rgb[2];
        }
        let mut w = value.w;
        let mut var_w = variance.w;
        let resp_w = ui.add(
            VariableF32::new(&format!("{}_w", id_prefix), "weight", &mut w, 0.0..=1.0, state)
                .with_variance(&mut var_w),
        );
        if resp_w.changed() {
            value.w = w;
            variance.w = var_w;
        }
    });
}

/// One scalar row: value + variance via VariableF32.
fn scalar_row(
    ui: &mut egui::Ui,
    label: &str,
    id: &str,
    value: &mut f32,
    variance: &mut f32,
    range: std::ops::RangeInclusive<f32>,
    state: &mut VariableState,
) {
    ui.add(
        VariableF32::new(id, label, value, range, state)
            .with_variance(variance),
    );
}

// --- File-dialog helpers ---

/// Open dialog returning a single file path. Mirrors
/// `super::super::helpers::rfd_pick_folder`. Defined here (small,
/// single call site) instead of helpers.rs to avoid touching shared
/// surface.
fn rfd_pick_open_file(label: &str, exts: &[&str]) -> Option<std::path::PathBuf> {
    rfd::FileDialog::new()
        .add_filter(label, exts)
        .pick_file()
}
fn rfd_pick_save_file(label: &str, suggested: &str, exts: &[&str]) -> Option<std::path::PathBuf> {
    rfd::FileDialog::new()
        .add_filter(label, exts)
        .set_file_name(suggested)
        .save_file()
}
```

**Note**: The `unsafe` cast on opacity is to reinterpret `[f32; 4]`'s
first three lanes as `[f32; 3]` — `StandardSurfaceParams.opacity` is
`Vec4` because the GPU layout pads it, but UI-side the alpha channel
is unused. A safer alternative is a small local helper:
```rust
fn first3_mut(v: &mut glam::Vec4) -> &mut [f32; 3] {
    bytemuck::from_bytes_mut::<[f32; 3]>(&v.x as *const f32 as *const u8)
}
```
…but that's still a transmute. Cleanest: copy `[v.x, v.y, v.z]` into
a local `[f32; 3]`, pass to `VariableColor`, then write back on
`changed()` — same pattern as the colour-weight row helper. Use this
approach instead of `unsafe` in the final code.

### B2. Register the module in `src/app/settings/mod.rs`

Add `mod materials;` next to the other `mod` declarations. The
`pub(crate) fn ui_materials` defined on `App` becomes reachable from
the dock dispatch automatically (it's an `impl App` block).

### B3. `find_by_uuid_mut` helper in `pt-material::MaterialLibrary`

The current library only has `find_by_uuid` (immutable). The rename
flow needs mutable access. Add:

```rust
pub fn find_by_uuid_mut(&mut self, uuid: uuid::Uuid) -> Option<(usize, &mut Material)> {
    self.materials
        .iter_mut()
        .enumerate()
        .find(|(_, m)| m.uuid == uuid)
}
```

In `crates/pt-material/src/library.rs`, mirror the existing
`find_by_uuid`. 4 lines.

---

## Phase C — Dock + toolbar integration (15 min)

### C1. `src/app/dock.rs`

Add `Materials` variant to `DockTab`:

```rust
pub enum DockTab {
    FileView,
    QuadTreeView,
    Extensions,
    Settings,
    Materials,   // <-- new
}
```

In `default_dock_state` / `build_dock_state`, decide initial
placement: open as a *floating* tab next to `Settings`, OFF by
default so existing users don't get a surprise re-layout. Easiest:
do NOT add it to the default state. Show only when
`app.show_materials == true` — the rebuild path (`rebuild_from_layout`)
adds it dynamically when the toggle flips.

`build_dock_state` signature changes to take `show_materials: bool`
alongside `show_settings: bool`; the Settings split adds a Materials
tab when requested:

```rust
let right_tabs = match (show_settings, show_materials) {
    (true, true)  => vec![DockTab::Settings, DockTab::Materials],
    (true, false) => vec![DockTab::Settings],
    (false, true) => vec![DockTab::Materials],
    (false, false) => vec![],
};
// only add the split when right_tabs is non-empty
```

Update the `match tab { ... }` blocks:
- `title`: `DockTab::Materials => "Materials".into()`
- `ui`: `DockTab::Materials => self.app.ui_materials(ui)`
- `on_close`: `DockTab::Materials => self.app.show_materials = false`

Also update `rebuild_from_layout` callers in `App::ui` (likely
`src/app/dock.rs` or `mod.rs`) to pass the new `show_materials` flag.

### C2. `src/app/toolbar.rs`

Add a `Materials` toggle button between `Viewport` and `Settings`:

```rust
if ui
    .add(egui::Button::new("Materials").selected(self.show_materials))
    .on_hover_text("Toggle Materials editor panel")
    .clicked()
{
    self.show_materials = !self.show_materials;
}
```

3 lines of state + dock rebuild logic in the place that already
handles `show_settings` flip → dock rebuild. Grep for that pattern
(`show_settings` flip + rebuild) and mirror.

### C3. Preset persistence (optional, defer if scope-budget tight)

`render-shared::Render3DOptions` already wraps the library, so
loading a preset loads the materials too — no extra work. The
`show_materials` panel toggle is NOT preset state; leave it on `App`
only.

---

## Phase D — Verify (15 min)

```
cd C:/projects/projects.rust.cg/squarebob-rs && python bootstrap.py b
```
Expect clean release build, exit 0.

Runtime checks (GUI — `target/release/squarebob.exe`):

- [ ] "Materials" button appears in toolbar.
- [ ] Clicking it opens a panel; clicking again closes it.
- [ ] Default library shows ~5 preset materials (from
      `pt_material::presets::default_library`).
- [ ] Click a slot → highlights as active.
- [ ] Drag a colour slider on Base Color → cube colours update live in
      the 3D viewport (PBR via `materials_buf` re-upload; PT via cache
      invalidation).
- [ ] Click triangle next to Base Color → variance row appears; setting
      it to e.g. `0.5` makes cubes assigned material[active] show
      per-cube colour variance in PT mode.
- [ ] "+ New" creates a default material; "Duplicate" copies the
      active; "Remove" removes it (refuses to empty the library).
- [ ] Double-click a slot name → in-place rename text edit.
- [ ] "Save As…" writes JSON; "Load…" reads it back; "Save" overwrites
      the last-known path.

If a check fails, stop and triage before declaring done.

---

## Phase E — Cleanup notes for follow-up sessions

Not in this autopilot run:

- **Drag-reorder slots** — egui has no built-in reorderable list; would
  need either `egui-dnd` dep or a manual up/down button column.
- **Material thumbnails** — render each slot as a tiny sphere preview
  using a minimal PT pipeline. ~1–2 days of work; outside scope.
- **Undo/redo for material edits** — would touch
  `Render3DOptions` history. Defer until the rest of the renderer has
  undo too.
- **VariableSlider unification in `app/settings/renderer.rs`** —
  replace `egui::Slider` with `VariableF32` for visual consistency,
  per the original Phase 4 Phase E notes. Independent of materials UI.

---

## Build commands cheatsheet

```bash
cd C:/projects/projects.rust.cg/squarebob-rs && python bootstrap.py b
cargo build -p squarebob-rs --release
cargo test  -p pt-material --release
cargo test  -p squarebob-widgets --release
target/release/squarebob.exe
```

## Rollback if surgery goes sideways

Phase 5 is purely additive on top of Phase 4. To revert:
- Delete `src/app/settings/materials.rs`
- Revert `src/app/settings/mod.rs` (remove `mod materials;`)
- Revert `src/app/state.rs` (remove the four new fields)
- Revert `src/app/toolbar.rs` (remove the Materials button)
- Revert `src/app/dock.rs` (remove the `Materials` variant + dispatch)
- Revert `crates/pt-material/src/library.rs` (remove
  `find_by_uuid_mut`)

Phase 4 infrastructure is untouched, so the renderer still works
without the editor.
