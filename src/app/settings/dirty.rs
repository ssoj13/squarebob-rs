//! Typed dirtiness signal for settings-panel callbacks.
//!
//! Replaces the old `changed: &mut bool` channel, which was a blanket
//! umbrella that conflated several orthogonal concerns and routed them
//! all through a single `SettingsChangedEvent` whose handler did
//! `needs_layout = true` — silently triggering a 2D/3D treemap rebuild
//! and a PT-accumulation reset for *any* settings-panel edit, even
//! denoise hyper-params that don't affect the path-traced image.
//!
//! Each settings callback now records *what* its widgets dirtied via
//! one of the explicit `mark_*` methods on [`SettingsDirty`]; the
//! dispatcher in `super::mod::ui_settings` consumes the accumulated
//! record and triggers the minimum set of follow-up actions.
//!
//! ## Categories
//!
//! Five orthogonal flags, in roughly increasing cost of follow-up:
//!
//! - [`preset`](SettingsDirty::preset) — the preset baseline drifted;
//!   autosave-on-interval should fire. No render-side work.
//! - [`layout`](SettingsDirty::layout) — geometry / 2D treemap visuals
//!   changed; the legacy `needs_layout` path re-rasterises and (in 3D)
//!   re-collects cubes. Implies `preset`.
//! - [`materials`](SettingsDirty::materials) — material library /
//!   classification changed; PT scene buffers (instances, materials,
//!   BVH inputs) must re-upload. Emits [`MaterialsChangedEvent`] which
//!   the render loop already wires to `mark_pt_scene_dirty` +
//!   `reset_pt_accumulation`. Implies `preset`.
//! - [`pt_accum`](SettingsDirty::pt_accum) — a PT sampling / camera /
//!   lighting knob changed; the existing samples are stale but the GPU
//!   scene buffers stay valid. Calls
//!   [`mark_pt_accum_reset`](render_3d::Renderer3D::mark_pt_accum_reset)
//!   so the next dispatch zeroes `frame_count` without re-uploading
//!   the BVH. Implies `preset`.
//! - [`pt_scene`](SettingsDirty::pt_scene) — PT scene structure
//!   (geometry handed to the path tracer, BVH topology, env-map
//!   binding) changed; full re-init via
//!   [`mark_pt_scene_dirty`](render_3d::Renderer3D::mark_pt_scene_dirty).
//!   Implies `pt_accum` (which implies `preset`).
//!
//! ## Implication chain
//!
//! Stronger categories imply weaker ones:
//!
//! ```text
//! pt_scene  →  pt_accum  →  preset
//! materials                →  preset
//! layout                   →  preset
//! ```
//!
//! Calling [`SettingsDirty::pt_scene`] therefore also marks
//! `pt_accum` and `preset`; calling [`SettingsDirty::layout`] marks
//! `preset`; and so on. Callers state the *strongest* category their
//! change affects and the chain takes care of the rest.
//!
//! ## Migration status
//!
//! As of this commit only `preset` and `layout` have settings-panel
//! call sites flowing through this struct — the previous bug was in
//! the `preset`-vs-`layout` conflation, so those two are the channels
//! that were rewired. The `materials`, `pt_accum`, and `pt_scene`
//! categories sit ready for new settings to opt in (e.g. a future
//! "exposure (EV)" knob would mark `pt_accum`, a "scene scale" rework
//! would mark `pt_scene`). The currently-existing PT knob channel
//! (`pt_changed: &mut bool` inside `renderer.rs`) still calls
//! `Renderer3D::reset_pt_accumulation` directly because it pre-dates
//! this struct; migrating it is a separate, mechanical pass.

/// Settings-panel dirtiness accumulator. See the module docs for the
/// full category list and the implication chain.
#[derive(Default, Debug, Clone, Copy)]
pub(in crate::app) struct SettingsDirty {
    /// At least one preset-tracked value moved off its saved baseline.
    /// Triggers autosave-on-interval bookkeeping; no render work.
    preset: bool,
    /// Geometry or 2D-treemap-visual parameter changed. The 2D CPU
    /// renderer rasterises directly from the layout, so visual knobs
    /// (brightness / cushion / ambient) live under this flag too.
    layout: bool,
    /// Material library / per-cube classification changed. Sent as
    /// [`MaterialsChangedEvent`] for the render loop to consume.
    materials: bool,
    /// PT sampling / camera / lighting param changed. The renderer
    /// keeps its scene buffers; only `frame_count` resets.
    pt_accum: bool,
    /// PT scene structure changed (geometry given to the path tracer,
    /// BVH topology). Triggers full re-init.
    pt_scene: bool,
}

impl SettingsDirty {
    /// Mark a preset-tracked but otherwise side-effect-free change.
    /// Use for visual chrome (panel fonts, tint), denoiser
    /// hyper-params (interval, clamp, mode) — anything where the only
    /// required follow-up is preset autosave + a UI repaint.
    pub(in crate::app) fn preset(&mut self) {
        self.preset = true;
    }

    /// Mark a layout-affecting change. Implies [`Self::preset`] —
    /// the affected field is itself part of the saved preset. Use for
    /// geometry knobs (height / color / LOD), 2D treemap visuals
    /// (brightness / cushion / scale / ambient), and view options
    /// (free-space toggle, layout style, grid).
    pub(in crate::app) fn layout(&mut self) {
        self.preset = true;
        self.layout = true;
    }

    /// Mark a material-library / classification change. Implies
    /// [`Self::preset`]. The dispatcher emits
    /// [`MaterialsChangedEvent`] which the render loop wires to a
    /// full PT scene re-upload + accumulation reset.
    pub(in crate::app) fn materials(&mut self) {
        self.preset = true;
        self.materials = true;
    }

    /// Mark a PT sampling / camera / lighting knob change. Implies
    /// [`Self::preset`]. The dispatcher calls
    /// `Renderer3D::mark_pt_accum_reset` so the next dispatch zeros
    /// `frame_count` without rebuilding the scene buffers.
    pub(in crate::app) fn pt_accum(&mut self) {
        self.preset = true;
        self.pt_accum = true;
    }

    /// Mark a PT scene-structure change (geometry given to the path
    /// tracer, BVH topology, env-map binding). Implies
    /// [`Self::pt_accum`] (and through it [`Self::preset`]). The
    /// dispatcher calls `Renderer3D::mark_pt_scene_dirty` so the next
    /// dispatch re-initialises the path tracer from scratch.
    pub(in crate::app) fn pt_scene(&mut self) {
        self.preset = true;
        self.pt_accum = true;
        self.pt_scene = true;
    }

    /// True if any flag is set. Used by the dispatcher to decide
    /// whether a repaint is needed at all.
    pub(in crate::app) fn any(&self) -> bool {
        self.preset || self.layout || self.materials || self.pt_accum || self.pt_scene
    }

    /// True if the preset baseline has drifted.
    pub(in crate::app) fn is_preset(&self) -> bool {
        self.preset
    }

    /// True if the cube / treemap layout must rebuild.
    pub(in crate::app) fn is_layout(&self) -> bool {
        self.layout
    }

    /// True if the material library / classification changed.
    pub(in crate::app) fn is_materials(&self) -> bool {
        self.materials
    }

    /// True if PT samples must reset but the scene stays valid.
    pub(in crate::app) fn is_pt_accum(&self) -> bool {
        self.pt_accum
    }

    /// True if the PT scene structure (geometry / BVH / env) changed.
    pub(in crate::app) fn is_pt_scene(&self) -> bool {
        self.pt_scene
    }
}
