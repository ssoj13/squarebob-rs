//! `playa-ae` — generic Attribute Editor widget for egui.
//!
//! Two pieces:
//!
//! * [`attrs`] — typed key-value attribute store ([`Attrs`] /
//!   [`AttrValue`] / [`AttrSchema`] / [`AttrDef`]). Originally part of
//!   Playa's entity model; kept here in hermetic form so any consumer
//!   can build an `Attrs` from its own domain types.
//! * [`attr_editor`] — table-based property-grid renderer. Picks the
//!   per-row widget from the value's [`AttrValue`] variant, optionally
//!   honouring `ui_options` schema hints (combobox / slider range).
//!
//! Typical usage:
//!
//! ```ignore
//! let mut attrs = build_attrs_from_my_struct(&my_struct);
//! if playa_ae::render(ui, &mut attrs, &mut state, "MyStruct") {
//!     apply_attrs_to_my_struct(&attrs, &mut my_struct);
//! }
//! ```

pub mod attr_editor;
pub mod attrs;

pub use attr_editor::{AttributesState, render, render_with_mixed};
pub use attrs::{
    AttrDef, AttrFlags, AttrSchema, AttrType, AttrValue, Attrs, FLAG_DAG, FLAG_DISPLAY,
    FLAG_INTERNAL, FLAG_KEYABLE, FLAG_READONLY,
};
