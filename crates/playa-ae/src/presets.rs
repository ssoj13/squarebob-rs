//! Maya / Houdini–style attribute preset bank.
//!
//! A [`PresetBank`] stores named snapshots of [`Attrs`] indexed by
//! their owning *schema name*, so a single JSON file can hold
//! presets for as many distinct attribute structs as the host app
//! wires up.
//!
//! Apply semantics deliberately match Houdini's "load preset" flow:
//! when a snapshot is applied to a target `Attrs`, each preset
//! entry is written **only if** the target carries an attribute of
//! the same `name` *and* the same [`AttrType`] (the discriminant of
//! the existing value). Mismatches and orphans are silently
//! skipped — this is what lets a preset survive schema renames
//! (extra keys ignored) and additions (missing keys keep their
//! current value).

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

use crate::attrs::{AttrValue, Attrs};

/// One named snapshot. The map keys mirror the source `Attrs` keys
/// 1:1; we only store entries the user could see in the AE (i.e.
/// `FLAG_DISPLAY`) so silent internal state doesn't leak into
/// preset files. Caller filters with `Attrs.iter()` when capturing.
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct PresetSnapshot {
    pub entries: BTreeMap<String, AttrValue>,
}

/// All presets across every schema the host has presented in this
/// session, ready to round-trip through `serde_json`.
///
/// `schemas[schema_name][preset_name] = PresetSnapshot`. A
/// `BTreeMap` is used so the in-memory order matches alphabetical
/// — that's also the order shown in the menu.
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct PresetBank {
    pub schemas: BTreeMap<String, BTreeMap<String, PresetSnapshot>>,
}

/// Apply result. Host uses this for status-bar feedback ("Applied
/// 6 / 7 attrs · 1 skipped (type mismatch)"). Counts add up so a
/// caller can sanity-check round-trips.
#[derive(Clone, Copy, Debug, Default)]
pub struct ApplyReport {
    pub applied: u32,
    pub skipped_missing: u32,
    pub skipped_type_mismatch: u32,
}

impl PresetBank {
    /// List preset names for a schema in stable (alphabetical)
    /// order. Empty iterator when the schema has no entries.
    pub fn list(&self, schema: &str) -> impl Iterator<Item = &str> {
        self.schemas
            .get(schema)
            .into_iter()
            .flat_map(|m| m.keys().map(String::as_str))
    }

    /// `true` if `name` exists under `schema`.
    pub fn contains(&self, schema: &str, name: &str) -> bool {
        self.schemas
            .get(schema)
            .map(|m| m.contains_key(name))
            .unwrap_or(false)
    }

    /// Capture every attribute on `attrs` into a snapshot stored
    /// under `(schema, name)`. Overwrites any existing entry with
    /// the same name. Returns the number of attributes captured.
    pub fn save(&mut self, schema: &str, name: impl Into<String>, attrs: &Attrs) -> usize {
        let snap = PresetSnapshot {
            entries: attrs
                .iter()
                .map(|(k, v)| (k.clone(), v.clone()))
                .collect(),
        };
        let count = snap.entries.len();
        self.schemas
            .entry(schema.to_string())
            .or_default()
            .insert(name.into(), snap);
        count
    }

    /// Apply preset `(schema, name)` onto `attrs`. Returns a count
    /// breakdown; `applied + skipped_*` equals the snapshot size.
    /// No-op (with `ApplyReport::default`) when the preset doesn't
    /// exist — caller decides whether to surface that case.
    pub fn apply(&self, schema: &str, name: &str, attrs: &mut Attrs) -> ApplyReport {
        let Some(snap) = self.schemas.get(schema).and_then(|m| m.get(name)) else {
            return ApplyReport::default();
        };
        let mut report = ApplyReport::default();
        for (key, value) in &snap.entries {
            match attrs.get(key) {
                None => report.skipped_missing += 1,
                Some(existing) => {
                    // `AttrType` doesn't enumerate every AttrValue
                    // variant (Int8 / UInt / Mat* are value-only),
                    // so we compare discriminants directly — that
                    // catches every kind that exists today and stays
                    // correct if AttrValue grows new variants.
                    if std::mem::discriminant(existing)
                        == std::mem::discriminant(value)
                    {
                        attrs.set(key.clone(), value.clone());
                        report.applied += 1;
                    } else {
                        report.skipped_type_mismatch += 1;
                    }
                }
            }
        }
        report
    }

    /// Remove preset `(schema, name)`. Returns `true` when the
    /// preset was present and got dropped.
    pub fn remove(&mut self, schema: &str, name: &str) -> bool {
        let Some(map) = self.schemas.get_mut(schema) else {
            return false;
        };
        let removed = map.remove(name).is_some();
        if map.is_empty() {
            self.schemas.remove(schema);
        }
        removed
    }

    /// Rename `(schema, old)` to `(schema, new)`. Returns `true` on
    /// success; `false` when `old` is missing or `new` is already
    /// taken (the latter avoids silent overwrites).
    pub fn rename(&mut self, schema: &str, old: &str, new: impl Into<String>) -> bool {
        let new = new.into();
        let Some(map) = self.schemas.get_mut(schema) else {
            return false;
        };
        if map.contains_key(&new) {
            return false;
        }
        let Some(snap) = map.remove(old) else {
            return false;
        };
        map.insert(new, snap);
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_attrs() -> Attrs {
        let mut a = Attrs::new();
        a.set("RGB", AttrValue::Vec3([0.5, 0.25, 0.0]));
        a.set("Roughness", AttrValue::Float(0.4));
        a.set("Name", AttrValue::Str("base".into()));
        a
    }

    #[test]
    fn save_and_apply_round_trip() {
        let mut bank = PresetBank::default();
        let src = make_attrs();
        bank.save("Material", "favourite", &src);

        let mut dst = make_attrs();
        dst.set("Roughness", AttrValue::Float(0.0));
        let report = bank.apply("Material", "favourite", &mut dst);

        assert_eq!(report.applied, 3);
        assert_eq!(report.skipped_missing, 0);
        assert_eq!(report.skipped_type_mismatch, 0);
        assert_eq!(dst.get_float("Roughness"), Some(0.4));
    }

    #[test]
    fn apply_skips_missing_keys() {
        let mut bank = PresetBank::default();
        let mut src = make_attrs();
        src.set("Extra", AttrValue::Float(1.0));
        bank.save("Material", "p", &src);

        let mut dst = make_attrs();
        let report = bank.apply("Material", "p", &mut dst);
        assert_eq!(report.applied, 3);
        assert_eq!(report.skipped_missing, 1);
    }

    #[test]
    fn apply_skips_type_mismatch() {
        let mut bank = PresetBank::default();
        let mut src = make_attrs();
        src.set("Roughness", AttrValue::Str("forty".into()));
        bank.save("Material", "wrong-type", &src);

        let mut dst = make_attrs();
        let report = bank.apply("Material", "wrong-type", &mut dst);
        assert_eq!(report.applied, 2);
        assert_eq!(report.skipped_type_mismatch, 1);
    }

    #[test]
    fn rename_refuses_overwrite() {
        let mut bank = PresetBank::default();
        let a = make_attrs();
        bank.save("Material", "p1", &a);
        bank.save("Material", "p2", &a);
        assert!(!bank.rename("Material", "p1", "p2"));
        assert!(bank.contains("Material", "p1"));
        assert!(bank.contains("Material", "p2"));
    }

    #[test]
    fn remove_clears_empty_schema() {
        let mut bank = PresetBank::default();
        let a = make_attrs();
        bank.save("Material", "p", &a);
        assert!(bank.remove("Material", "p"));
        assert!(!bank.schemas.contains_key("Material"));
    }

    #[test]
    fn apply_unknown_preset_is_noop() {
        let bank = PresetBank::default();
        let mut dst = make_attrs();
        let report = bank.apply("Material", "nope", &mut dst);
        assert_eq!(report.applied, 0);
    }

    #[test]
    fn json_round_trip() {
        let mut bank = PresetBank::default();
        let a = make_attrs();
        bank.save("Material", "p", &a);
        let json = serde_json::to_string(&bank).unwrap();
        let parsed: PresetBank = serde_json::from_str(&json).unwrap();
        assert!(parsed.contains("Material", "p"));
    }
}
