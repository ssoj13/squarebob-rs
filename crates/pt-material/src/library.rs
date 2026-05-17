//! Scene-owned ordered collection of [`Material`]s.

use serde::{Deserialize, Serialize};

use crate::material::Material;
use crate::presets::default_library;

/// Ordered library of materials a scene exposes. Cubes reference
/// entries by `material_index` (u32 array index). Reordering /
/// inserting / deleting entries invalidates indices — callers that
/// persist external references should round-trip through
/// [`Material::uuid`] instead.
///
/// `active` is the editor selection — the index whose attributes are
/// shown in the right-hand attribute editor. Defaults to `0`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MaterialLibrary {
    pub materials: Vec<Material>,
    #[serde(default)]
    pub active: usize,
}

impl Default for MaterialLibrary {
    fn default() -> Self {
        default_library()
    }
}

impl MaterialLibrary {
    /// Number of materials in the library.
    pub fn len(&self) -> usize {
        self.materials.len()
    }

    /// `true` when the library has zero entries.
    pub fn is_empty(&self) -> bool {
        self.materials.is_empty()
    }

    /// Reference to the active material, or `None` when the library
    /// is empty or `active` is out-of-range.
    pub fn active_material(&self) -> Option<&Material> {
        self.materials.get(self.active)
    }

    /// Mutable reference to the active material — used by the editor
    /// UI to push slider edits.
    pub fn active_material_mut(&mut self) -> Option<&mut Material> {
        self.materials.get_mut(self.active)
    }

    /// Append a new material. Returns its index.
    pub fn push(&mut self, m: Material) -> usize {
        self.materials.push(m);
        self.materials.len() - 1
    }

    /// Duplicate the entry at `index` (new UUID, suffixed name).
    /// Returns the new entry's index. No-op when `index` is OOB.
    pub fn duplicate(&mut self, index: usize) -> Option<usize> {
        let src = self.materials.get(index)?.clone();
        let mut copy = src;
        copy.uuid = uuid::Uuid::new_v4();
        copy.name = format!("{} copy", copy.name);
        Some(self.push(copy))
    }

    /// Remove the entry at `index`. Adjusts `active` so it remains in
    /// range. No-op on OOB; refuses to empty the library so the UI
    /// can always render an active selection.
    pub fn remove(&mut self, index: usize) {
        if index >= self.materials.len() || self.materials.len() <= 1 {
            return;
        }
        self.materials.remove(index);
        if self.active >= self.materials.len() {
            self.active = self.materials.len() - 1;
        }
    }

    /// Set the active slot, clamping to a valid range.
    pub fn set_active(&mut self, index: usize) {
        self.active = index.min(self.materials.len().saturating_sub(1));
    }

    /// Lookup by UUID (linear scan — libraries are short, this is fine).
    pub fn find_by_uuid(&self, uuid: uuid::Uuid) -> Option<(usize, &Material)> {
        self.materials
            .iter()
            .enumerate()
            .find(|(_, m)| m.uuid == uuid)
    }

    /// Mutable counterpart to [`Self::find_by_uuid`]. Used by editor
    /// flows (rename, programmatic param tweaks) that need to address a
    /// slot by stable identity rather than array index.
    pub fn find_by_uuid_mut(&mut self, uuid: uuid::Uuid) -> Option<(usize, &mut Material)> {
        self.materials
            .iter_mut()
            .enumerate()
            .find(|(_, m)| m.uuid == uuid)
    }
}
