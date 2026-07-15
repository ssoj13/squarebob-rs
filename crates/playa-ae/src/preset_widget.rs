//! `Presets ▾` menu button — the Maya / Houdini–style way to apply
//! a saved [`PresetSnapshot`](crate::presets::PresetSnapshot) to
//! the current `Attrs`, save the current `Attrs` as a new preset,
//! or rename / delete existing presets.
//!
//! Names containing `/` are split into sub-menus by the leading
//! segment, so a host can drop in a flat preset bank with names
//! like `"Plastic / Red Glossy"`, `"Metal / Brushed Gold"` and the
//! UI groups them automatically.

use egui::Ui;

use crate::attrs::Attrs;
use crate::presets::{ApplyReport, PresetBank};

/// Per-widget local state — survives across frames via the host's
/// owned `App` state. Only `new_name` and `manage_open` carry
/// session content; the rest are derived per-frame.
#[derive(Clone, Debug, Default)]
pub struct PresetButtonState {
    /// In-progress text for the "Save current as…" inline field.
    pub new_name: String,
    /// In-progress rename target: `(old_name, new_name_buffer)`.
    /// `None` when no rename is open.
    pub rename: Option<(String, String)>,
    /// Toggle for the inline manage panel (rename / delete list).
    pub manage_open: bool,
}

/// One event surfaced after the user interacts with the menu. The
/// host inspects this to drive side-effects: persist the
/// `PresetBank` to disk, raise a `MaterialsChangedEvent`, etc.
#[derive(Clone, Debug)]
pub enum PresetButtonEvent {
    /// No menu action this frame.
    None,
    /// A preset was applied onto `attrs`. The report tells the
    /// host how many attrs landed vs were skipped.
    Applied { name: String, report: ApplyReport },
    /// Current `attrs` was captured into the bank as `name`.
    Saved { name: String },
    /// `name` was removed from the bank.
    Removed { name: String },
    /// `old` was renamed to `new`.
    Renamed { old: String, new: String },
}

/// Render the menu button. `attrs` is the live attribute set —
/// "Save current as…" snapshots it, "Apply" writes back into it.
/// `schema` is the bank index key (use the same value as the
/// schema name you passed to `playa_ae::render`).
pub fn presets_button(
    ui: &mut Ui,
    bank: &mut PresetBank,
    state: &mut PresetButtonState,
    attrs: &mut Attrs,
    schema: &str,
) -> PresetButtonEvent {
    let mut event = PresetButtonEvent::None;

    let button = ui.menu_button("Presets ▾", |ui| {
        ui.set_min_width(220.0);

        // Group preset names by the segment before "/". Flat names
        // (no "/") go straight in. Sub-menus are sorted; entries
        // inside each menu are also sorted.
        let names: Vec<String> = bank.list(schema).map(String::from).collect();
        if names.is_empty() {
            ui.label(egui::RichText::new("(no presets yet)").color(ui.visuals().weak_text_color()));
        } else {
            let mut grouped: std::collections::BTreeMap<Option<String>, Vec<String>> =
                Default::default();
            for full in &names {
                let (group, _) = split_group(full);
                grouped.entry(group).or_default().push(full.clone());
            }
            // Flat names first, then grouped sub-menus.
            if let Some(flat) = grouped.remove(&None) {
                for full in flat {
                    if apply_item(ui, bank, attrs, schema, &full).is_some() {
                        event = PresetButtonEvent::Applied {
                            name: full.clone(),
                            report: bank.apply(schema, &full, attrs),
                        };
                    }
                }
                if !grouped.is_empty() {
                    ui.separator();
                }
            }
            for (group, entries) in grouped {
                let group_label = group.unwrap_or_else(|| "(misc)".into());
                ui.menu_button(group_label, |ui| {
                    for full in entries {
                        let leaf = split_group(&full).1;
                        if apply_sub_item(ui, leaf, &full).is_some() {
                            event = PresetButtonEvent::Applied {
                                name: full.clone(),
                                report: bank.apply(schema, &full, attrs),
                            };
                        }
                    }
                });
            }
        }

        ui.separator();

        // Save-current-as: inline text field + Enter / button.
        ui.horizontal(|ui| {
            ui.label("Save as:");
            let resp = ui.text_edit_singleline(&mut state.new_name);
            let commit_now = (resp.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter)))
                || ui.button("Save").clicked();
            if commit_now && !state.new_name.trim().is_empty() {
                let name = state.new_name.trim().to_string();
                bank.save(schema, name.clone(), attrs);
                state.new_name.clear();
                event = PresetButtonEvent::Saved { name };
            }
        });

        ui.separator();

        // Manage panel — rename / delete entries.
        let manage_label = if state.manage_open {
            "Hide manage"
        } else {
            "Manage…"
        };
        if ui.button(manage_label).clicked() {
            state.manage_open = !state.manage_open;
        }
        if state.manage_open {
            for name in names {
                ui.horizontal(|ui| {
                    let is_renaming = state
                        .rename
                        .as_ref()
                        .map(|(old, _)| old == &name)
                        .unwrap_or(false);
                    if is_renaming {
                        if let Some((_, buf)) = state.rename.as_mut() {
                            ui.text_edit_singleline(buf);
                            if ui.button("OK").clicked() {
                                let old = name.clone();
                                let new = buf.trim().to_string();
                                if !new.is_empty() && bank.rename(schema, &old, new.clone()) {
                                    event = PresetButtonEvent::Renamed { old, new };
                                }
                                state.rename = None;
                            }
                            if ui.button("Cancel").clicked() {
                                state.rename = None;
                            }
                        }
                    } else {
                        ui.label(&name);
                        if ui.button("rename").clicked() {
                            state.rename = Some((name.clone(), name.clone()));
                        }
                        if ui.button("delete").clicked() && bank.remove(schema, &name) {
                            event = PresetButtonEvent::Removed { name: name.clone() };
                        }
                    }
                });
            }
        }
    });
    button.response.on_hover_text(
        "Saved attribute presets. Apply one to the current values, save the \
         current state as a new preset, or rename / delete existing ones.",
    );

    event
}

/// Top-level "apply" entry in the flat list. Returns `Some(())`
/// when clicked so the caller can run the apply step. We don't
/// apply inline because that would require a second `&mut` borrow
/// on `bank` inside the closure.
fn apply_item(
    ui: &mut Ui,
    _bank: &PresetBank,
    _attrs: &Attrs,
    _schema: &str,
    name: &str,
) -> Option<()> {
    if ui.button(name).clicked() {
        Some(())
    } else {
        None
    }
}

/// Same as `apply_item` but for the sub-menu version where we
/// already know the leaf label.
fn apply_sub_item(ui: &mut Ui, leaf: &str, _full: &str) -> Option<()> {
    if ui.button(leaf).clicked() {
        Some(())
    } else {
        None
    }
}

/// Split a preset name on the first `/` so the menu can nest by
/// the leading segment. `"Metal / Gold"` → `(Some("Metal"), "Gold")`,
/// `"Plain"` → `(None, "Plain")`.
fn split_group(name: &str) -> (Option<String>, &str) {
    if let Some(slash) = name.find('/') {
        let group = name[..slash].trim().to_string();
        let leaf = name[slash + 1..].trim_start();
        if group.is_empty() {
            (None, name)
        } else {
            (Some(group), leaf)
        }
    } else {
        (None, name)
    }
}
