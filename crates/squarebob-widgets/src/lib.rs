//! `squarebob-widgets` — reusable egui widgets specific to squarebob-rs.
//!
//! Modules:
//! * [`variable`] — low-level typed widgets that pair a base slider
//!   with an optional inline collapsible variance slider. Drop-in
//!   replacement for `egui::Slider` and friends. Used by the material
//!   editor and physical camera to give every numeric parameter an
//!   optional per-instance / per-cube random spread without changing
//!   the call site visually when variance is disabled.
//!
//! The generic Attribute Editor (table-based property grid + typed
//! `Attrs` store) has moved to the standalone `playa-ae` crate so
//! other consumers can pull it without dragging in the rest of
//! squarebob-widgets.

pub mod variable;

pub use variable::{VariableColor, VariableF32, VariableState, VariableVec3};
