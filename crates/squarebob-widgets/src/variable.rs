//! Typed "value + optional variance" egui widgets.
//!
//! Drop-in replacement for `egui::Slider` / colour picker that adds a
//! tiny collapsible triangle to the right of the value editor. When
//! the caller supplies a `variance` reference via [`Self::with_variance`]
//! the triangle appears; clicking it expands a second-row slider for
//! the variance magnitude. When `variance` is `None` the widget looks
//! identical to a plain `egui::Slider`.
//!
//! Visual cue: the triangle is rendered in orange when the variance
//! magnitude is non-zero, even while collapsed, so "there's a spread
//! set on this field" is visible at a glance.
//!
//! State (per-field expanded-or-not) is kept in [`VariableState`] and
//! addressed by string id supplied by the caller. Use a stable id per
//! widget so collapse state survives across frames.

use eframe::egui::{self, Color32, Response, Ui, Widget};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::ops::RangeInclusive;

/// Persistent collapse state for all variable-slider widgets in a
/// panel. Keep one instance per panel and pass `&mut state` to every
/// widget; the widget reads/writes the entry keyed by the per-widget
/// id string.
#[derive(Debug, Default, Clone, Serialize, Deserialize)]
pub struct VariableState {
    expanded: HashMap<String, bool>,
}

impl VariableState {
    pub fn is_expanded(&self, id: &str) -> bool {
        self.expanded.get(id).copied().unwrap_or(false)
    }
    pub fn set_expanded(&mut self, id: impl Into<String>, v: bool) {
        self.expanded.insert(id.into(), v);
    }
    pub fn toggle(&mut self, id: impl Into<String>) {
        let key = id.into();
        let cur = self.is_expanded(&key);
        self.expanded.insert(key, !cur);
    }
}

/// Threshold below which a variance value counts as "zero" for the
/// orange-triangle visual hint. Chosen well below the smallest UI
/// drag step so noise from slider snapping doesn't false-positive.
const VARIANCE_EPS: f32 = 1e-6;

/// Triangle hint colour when variance is non-zero — orange, contrasts
/// against egui's default light/dark palettes.
const ORANGE: Color32 = Color32::from_rgb(255, 165, 0);

/// Render the optional triangle expander and dispatch to
/// `state.toggle` on click. Returns whether the panel should render
/// its expanded body row this frame.
///
/// Centralised here so every concrete variable-widget renders the
/// triangle the same way (consistent colour, position, hint text).
fn render_triangle(
    ui: &mut Ui,
    id: &str,
    has_variance_now: bool,
    state: &mut VariableState,
) -> bool {
    let expanded = state.is_expanded(id);
    let arrow = if expanded { "▼" } else { "▶" };
    let color = if has_variance_now {
        ORANGE
    } else {
        ui.visuals().weak_text_color()
    };
    let resp = ui.add(
        egui::Label::new(egui::RichText::new(arrow).color(color))
            .sense(egui::Sense::click()),
    );
    if resp.clicked() {
        state.toggle(id.to_string());
    }
    state.is_expanded(id)
}

// ============================================================================
// VariableF32
// ============================================================================

/// `f32` slider with optional variance. When `variance` is `None`,
/// renders as a plain `egui::Slider`. Otherwise, a triangle expander
/// reveals a second slider for the variance magnitude (range
/// `0..=range.span()`).
pub struct VariableF32<'a> {
    label: &'a str,
    id: &'a str,
    value: &'a mut f32,
    variance: Option<&'a mut f32>,
    range: RangeInclusive<f32>,
    logarithmic: bool,
    suffix: Option<&'a str>,
    state: &'a mut VariableState,
}

impl<'a> VariableF32<'a> {
    pub fn new(
        id: &'a str,
        label: &'a str,
        value: &'a mut f32,
        range: RangeInclusive<f32>,
        state: &'a mut VariableState,
    ) -> Self {
        Self {
            label,
            id,
            value,
            variance: None,
            range,
            logarithmic: false,
            suffix: None,
            state,
        }
    }

    pub fn with_variance(mut self, v: &'a mut f32) -> Self {
        self.variance = Some(v);
        self
    }

    pub fn logarithmic(mut self, on: bool) -> Self {
        self.logarithmic = on;
        self
    }

    pub fn suffix(mut self, s: &'a str) -> Self {
        self.suffix = Some(s);
        self
    }
}

impl Widget for VariableF32<'_> {
    fn ui(self, ui: &mut Ui) -> Response {
        let Self {
            label,
            id,
            value,
            variance,
            range,
            logarithmic,
            suffix,
            state,
        } = self;
        let range_span = (*range.end() - *range.start()).abs().max(VARIANCE_EPS);

        ui.vertical(|ui| {
            // Value row.
            let value_resp = ui
                .horizontal(|ui| {
                    ui.label(label);
                    let mut slider = egui::Slider::new(value, range.clone());
                    if logarithmic {
                        slider = slider.logarithmic(true);
                    }
                    if let Some(s) = suffix {
                        slider = slider.suffix(s);
                    }
                    let resp = ui.add(slider);

                    // Triangle hint only when caller supplied variance.
                    let mut expanded_now = false;
                    if let Some(ref v) = variance {
                        let has = (**v).abs() > VARIANCE_EPS;
                        expanded_now = render_triangle(ui, id, has, state);
                    }
                    (resp, expanded_now)
                })
                .inner;

            let (resp, show_variance_row) = value_resp;

            // Expanded variance row (rendered below, indented).
            if show_variance_row
                && let Some(v) = variance
            {
                ui.horizontal(|ui| {
                    ui.add_space(20.0);
                    ui.label(format!("± {}", label));
                    let mut slider = egui::Slider::new(v, 0.0..=range_span);
                    if logarithmic {
                        slider = slider.logarithmic(true);
                    }
                    ui.add(slider);
                });
            }
            resp
        })
        .inner
    }
}

// ============================================================================
// VariableVec3
// ============================================================================

/// 3-component `f32` vector with optional per-component variance.
/// Used for params like coordinates or non-colour vec3 quantities.
/// For colour use [`VariableColor`] instead.
pub struct VariableVec3<'a> {
    label: &'a str,
    id: &'a str,
    value: &'a mut [f32; 3],
    variance: Option<&'a mut [f32; 3]>,
    range: RangeInclusive<f32>,
    state: &'a mut VariableState,
}

impl<'a> VariableVec3<'a> {
    pub fn new(
        id: &'a str,
        label: &'a str,
        value: &'a mut [f32; 3],
        range: RangeInclusive<f32>,
        state: &'a mut VariableState,
    ) -> Self {
        Self {
            label,
            id,
            value,
            variance: None,
            range,
            state,
        }
    }

    pub fn with_variance(mut self, v: &'a mut [f32; 3]) -> Self {
        self.variance = Some(v);
        self
    }
}

impl Widget for VariableVec3<'_> {
    fn ui(self, ui: &mut Ui) -> Response {
        let Self {
            label,
            id,
            value,
            variance,
            range,
            state,
        } = self;
        let range_span = (*range.end() - *range.start()).abs().max(VARIANCE_EPS);

        ui.vertical(|ui| {
            let value_resp = ui
                .horizontal(|ui| {
                    ui.label(label);
                    let r0 = ui.add(
                        egui::DragValue::new(&mut value[0])
                            .speed(0.01)
                            .range(range.clone()),
                    );
                    let r1 = ui.add(
                        egui::DragValue::new(&mut value[1])
                            .speed(0.01)
                            .range(range.clone()),
                    );
                    let r2 = ui.add(
                        egui::DragValue::new(&mut value[2])
                            .speed(0.01)
                            .range(range.clone()),
                    );

                    let mut expanded_now = false;
                    if let Some(ref v) = variance {
                        let has = v.iter().any(|&x| x.abs() > VARIANCE_EPS);
                        expanded_now = render_triangle(ui, id, has, state);
                    }
                    (r0.union(r1).union(r2), expanded_now)
                })
                .inner;
            let (resp, show_variance_row) = value_resp;

            if show_variance_row
                && let Some(v) = variance
            {
                ui.horizontal(|ui| {
                    ui.add_space(20.0);
                    ui.label(format!("± {}", label));
                    ui.add(
                        egui::DragValue::new(&mut v[0])
                            .speed(0.01)
                            .range(0.0..=range_span),
                    );
                    ui.add(
                        egui::DragValue::new(&mut v[1])
                            .speed(0.01)
                            .range(0.0..=range_span),
                    );
                    ui.add(
                        egui::DragValue::new(&mut v[2])
                            .speed(0.01)
                            .range(0.0..=range_span),
                    );
                });
            }
            resp
        })
        .inner
    }
}

// ============================================================================
// VariableColor
// ============================================================================

/// Colour swatch + sRGB picker + per-channel HDR drag values, with
/// optional per-channel variance row. Use this for albedo / emission
/// / spec tint / etc. The sRGB picker rounds to display bytes for the
/// preview, but the underlying `rgb` stays linear HDR (>1.0 allowed).
pub struct VariableColor<'a> {
    label: &'a str,
    id: &'a str,
    rgb: &'a mut [f32; 3],
    variance: Option<&'a mut [f32; 3]>,
    state: &'a mut VariableState,
}

impl<'a> VariableColor<'a> {
    pub fn new(
        id: &'a str,
        label: &'a str,
        rgb: &'a mut [f32; 3],
        state: &'a mut VariableState,
    ) -> Self {
        Self {
            label,
            id,
            rgb,
            variance: None,
            state,
        }
    }

    pub fn with_variance(mut self, v: &'a mut [f32; 3]) -> Self {
        self.variance = Some(v);
        self
    }
}

impl Widget for VariableColor<'_> {
    fn ui(self, ui: &mut Ui) -> Response {
        let Self {
            label,
            id,
            rgb,
            variance,
            state,
        } = self;

        ui.vertical(|ui| {
            let value_resp = ui
                .horizontal(|ui| {
                    ui.label(label);

                    // 8-bit preview swatch with picker — rounds the
                    // linear HDR value to sRGB display bytes. Edits
                    // propagate back to linear via /255.
                    let mut srgb = [
                        (rgb[0].clamp(0.0, 1.0) * 255.0) as u8,
                        (rgb[1].clamp(0.0, 1.0) * 255.0) as u8,
                        (rgb[2].clamp(0.0, 1.0) * 255.0) as u8,
                    ];
                    let swatch = ui.color_edit_button_srgb(&mut srgb);
                    if swatch.changed() {
                        rgb[0] = srgb[0] as f32 / 255.0;
                        rgb[1] = srgb[1] as f32 / 255.0;
                        rgb[2] = srgb[2] as f32 / 255.0;
                    }

                    let r0 = ui.add(egui::DragValue::new(&mut rgb[0]).speed(0.01));
                    let r1 = ui.add(egui::DragValue::new(&mut rgb[1]).speed(0.01));
                    let r2 = ui.add(egui::DragValue::new(&mut rgb[2]).speed(0.01));

                    let mut expanded_now = false;
                    if let Some(ref v) = variance {
                        let has = v.iter().any(|&x| x.abs() > VARIANCE_EPS);
                        expanded_now = render_triangle(ui, id, has, state);
                    }
                    (swatch.union(r0).union(r1).union(r2), expanded_now)
                })
                .inner;
            let (resp, show_variance_row) = value_resp;

            if show_variance_row
                && let Some(v) = variance
            {
                ui.horizontal(|ui| {
                    ui.add_space(20.0);
                    ui.label(format!("± {}", label));
                    ui.add(
                        egui::DragValue::new(&mut v[0])
                            .speed(0.01)
                            .range(0.0..=1.0),
                    );
                    ui.add(
                        egui::DragValue::new(&mut v[1])
                            .speed(0.01)
                            .range(0.0..=1.0),
                    );
                    ui.add(
                        egui::DragValue::new(&mut v[2])
                            .speed(0.01)
                            .range(0.0..=1.0),
                    );
                });
            }
            resp
        })
        .inner
    }
}
