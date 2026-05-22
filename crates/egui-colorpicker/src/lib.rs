//! Maya-style HDR color picker popup for egui.
//!
//! The crate exports two things:
//!
//! - [`color_button`] / [`color_button_with`] — the swatch button you
//!   drop into a row; clicking opens the picker popup.
//! - [`PickerConfig`] — host-side knobs: HDR slider cap, alpha
//!   toggle, display-space transform (default sRGB OETF, but any
//!   linear→display function works — pass your tonemap here), and
//!   an optional eyedropper callback.
//!
//! Picker contents (Maya-style):
//!
//! * Two preview rectangles at the top: **working** (raw linear
//!   sRGB) vs **display** (after `display_transform`).
//! * A 2D Hue × Saturation gradient + vertical Value slider that
//!   honours the HDR cap, so emissive materials can pull V > 1.0.
//! * Side panel with RGB (linear + sRGB columns), HSV, hex, alpha
//!   rows. Editing any field updates the others in real time.
//! * Color history strip (12 slots, deduped, persisted via
//!   `egui::Memory.data` so it survives panel layouts).
//! * Eyedropper button shown only when the host installed a
//!   callback — the picker invokes the callback; the host wires it
//!   to a viewport-sample tool.
//!
//! All channels are stored linearly internally and exposed via the
//! `[f32; 4]` mutable handle. HDR values (any channel > 1.0) flow
//! through unchanged; the sRGB column clamps to `[0, 1]` because
//! sRGB encoding is undefined above 1.0.

use egui::{Color32, Mesh, Pos2, Rect, Response, Sense, Shape, Stroke, Ui, Vec2};
use serde::{Deserialize, Serialize};

mod color;
pub use color::{
    hex_to_linear, hsv_to_rgb, linear_to_hex, linear_to_srgb, rgb_to_hsv, srgb_to_linear,
};

/// Maximum number of past colors kept by the history strip.
pub const HISTORY_SIZE: usize = 12;

/// Picker configuration. Build with `PickerConfig::default()` then
/// tweak fields, or use the builder methods.
pub struct PickerConfig {
    /// Upper bound for the HDR slider lane (R/G/B/V linear). Values
    /// can still go higher via hex / direct edit; the slider just
    /// won't drag past this.
    pub hdr_max: f32,
    /// Show the alpha row in the popup.
    pub alpha_enabled: bool,
    /// Map a linear RGB triple to its on-screen "display" version.
    /// Default = sRGB OETF (per-channel gamma). Replace with your
    /// project's tonemap to keep the display preview honest.
    pub display_transform: Box<dyn Fn([f32; 3]) -> [f32; 3]>,
    /// Optional eyedropper hook. When `Some`, the popup shows an
    /// "Eyedropper" button; clicking it invokes the callback. The
    /// callback is expected to start a viewport-sample interaction;
    /// the actual color update happens later, when the host writes
    /// back into the `&mut [f32; 4]` handle on the next frame.
    pub eyedropper: Option<Box<dyn FnMut()>>,
}

impl Default for PickerConfig {
    fn default() -> Self {
        Self {
            hdr_max: 4.0,
            alpha_enabled: true,
            display_transform: Box::new(|c| {
                [linear_to_srgb(c[0]), linear_to_srgb(c[1]), linear_to_srgb(c[2])]
            }),
            eyedropper: None,
        }
    }
}

impl PickerConfig {
    pub fn with_hdr_max(mut self, max: f32) -> Self {
        self.hdr_max = max;
        self
    }
    pub fn with_alpha(mut self, enabled: bool) -> Self {
        self.alpha_enabled = enabled;
        self
    }
    pub fn with_display_transform(
        mut self,
        f: impl Fn([f32; 3]) -> [f32; 3] + 'static,
    ) -> Self {
        self.display_transform = Box::new(f);
        self
    }
    pub fn with_eyedropper(mut self, f: impl FnMut() + 'static) -> Self {
        self.eyedropper = Some(Box::new(f));
        self
    }
}

/// History entries — the bar of last-used colors at the bottom of
/// the popup. Persisted via `egui::Memory.data` so it survives
/// panel layouts.
#[derive(Clone, Default, Serialize, Deserialize)]
pub struct ColorHistory {
    pub entries: Vec<[f32; 4]>,
}

impl ColorHistory {
    fn push(&mut self, c: [f32; 4]) {
        let eq = |a: [f32; 4], b: [f32; 4]| {
            (a[0] - b[0]).abs() < 1e-4
                && (a[1] - b[1]).abs() < 1e-4
                && (a[2] - b[2]).abs() < 1e-4
                && (a[3] - b[3]).abs() < 1e-4
        };
        self.entries.retain(|e| !eq(*e, c));
        self.entries.insert(0, c);
        self.entries.truncate(HISTORY_SIZE);
    }
}

fn history_id() -> egui::Id {
    egui::Id::new("egui_colorpicker_history_v1")
}

fn load_history(ctx: &egui::Context) -> ColorHistory {
    ctx.memory_mut(|m| {
        m.data
            .get_persisted::<ColorHistory>(history_id())
            .unwrap_or_default()
    })
}

fn save_history(ctx: &egui::Context, h: ColorHistory) {
    ctx.memory_mut(|m| m.data.insert_persisted(history_id(), h));
}

/// Drop-in swatch button using the default [`PickerConfig`]. Use
/// when the host doesn't need to override any picker behaviour.
pub fn color_button(ui: &mut Ui, color: &mut [f32; 4]) -> Response {
    color_button_with(ui, color, &mut PickerConfig::default())
}

/// Swatch button with a customised config (HDR cap, display
/// transform, eyedropper hook, etc.). The `&mut PickerConfig` is
/// taken by mutable reference so the eyedropper callback can be
/// invoked from inside the popup without owning the entire config.
pub fn color_button_with(ui: &mut Ui, color: &mut [f32; 4], cfg: &mut PickerConfig) -> Response {
    // Nuke-style tiny chip: a round colour pill that sits next to
    // the R/G/B/A DragValues without stealing column width. Clicks
    // open the full picker popup.
    let size = ui.spacing().interact_size.y.max(14.0);
    let swatch_size = Vec2::new(size, size);
    let (rect, response) = ui.allocate_exact_size(swatch_size, Sense::click());

    paint_swatch_icon(ui, rect, color, cfg, response.hovered());

    // Use `Popup::menu(&response)` — the only `Popup` constructor
    // that wires both pieces of click semantics for us:
    //   * `gesture(Click)`     — clicking the anchor toggles open.
    //   * `CloseOnClickOutside` — clicking outside closes it.
    //
    // `Popup::from_response` on its own has no gesture and no
    // close behaviour by default, so the popup renders every
    // frame for every row (which is what we saw: pickers
    // permanently open on every Vec4 attribute).
    let popup_id = response.id.with("colorpicker_popup");
    egui::Popup::menu(&response)
        .id(popup_id)
        .gap(4.0)
        .show(|ui| picker_popup_contents(ui, color, cfg));
    response
}

/// Compact circular swatch used inside an attribute row. Hover
/// brightens the rim so the hit target reads clearly even when
/// the colour itself is close to the background.
fn paint_swatch_icon(
    ui: &Ui,
    rect: Rect,
    color: &[f32; 4],
    cfg: &PickerConfig,
    hovered: bool,
) {
    let d = (cfg.display_transform)([color[0], color[1], color[2]]);
    let c32 = display_to_color32(d, color[3]);
    let painter = ui.painter();
    let radius = (rect.width().min(rect.height()) * 0.5) - 0.5;
    let center = rect.center();
    // Tiny checkerboard for alpha visibility — drawn as a clipped
    // square that the circle stroke covers on the edge.
    let cell = (radius * 0.6).max(2.0);
    let chk_rect = Rect::from_center_size(center, Vec2::splat(radius * 2.0));
    paint_checker(painter, chk_rect, cell);
    painter.circle_filled(center, radius, c32);
    let rim = if hovered {
        Color32::from_gray(200)
    } else {
        Color32::from_gray(80)
    };
    painter.circle_stroke(center, radius, Stroke::new(1.0, rim));
}

fn display_to_color32(d: [f32; 3], a: f32) -> Color32 {
    let q = |x: f32| (x.clamp(0.0, 1.0) * 255.0).round() as u8;
    Color32::from_rgba_unmultiplied(q(d[0]), q(d[1]), q(d[2]), q(a))
}

fn paint_checker(painter: &egui::Painter, rect: Rect, cell: f32) {
    let dark = Color32::from_gray(60);
    let light = Color32::from_gray(180);
    painter.rect_filled(rect, 0.0, light);
    let cols = ((rect.width() / cell).ceil() as i32).max(1);
    let rows = ((rect.height() / cell).ceil() as i32).max(1);
    for r in 0..rows {
        for c in 0..cols {
            if (r + c) % 2 == 0 {
                continue;
            }
            let p = Pos2::new(rect.min.x + c as f32 * cell, rect.min.y + r as f32 * cell);
            let cr = Rect::from_min_size(p, Vec2::splat(cell)).intersect(rect);
            painter.rect_filled(cr, 0.0, dark);
        }
    }
}

fn picker_popup_contents(ui: &mut Ui, color: &mut [f32; 4], cfg: &mut PickerConfig) {
    // Fixed popup width. The previous `ui.available_width()` based
    // splitter created a feedback loop inside `Popup::show`: the
    // popup sizes itself to its content's bounding box, but the
    // content sized itself to whatever the parent gave (which was
    // unbounded). Each frame the popup grew → more available width
    // → content grew → ad infinitum. A hard-coded width breaks the
    // loop and keeps the popup stable across frames.
    const POPUP_W: f32 = 360.0;
    const PREVIEW_W: f32 = (POPUP_W - 12.0) * 0.5;
    ui.set_max_width(POPUP_W);
    ui.set_min_width(POPUP_W);

    // --- Section 1: previews (working sRGB-encoded | display via
    // user-supplied transform). Side by side, equal size.
    let preview_h = 36.0;
    ui.horizontal(|ui| {
        let half = PREVIEW_W;
        let working_disp = [
            linear_to_srgb(color[0]),
            linear_to_srgb(color[1]),
            linear_to_srgb(color[2]),
        ];
        let (r1, _) = ui.allocate_exact_size(Vec2::new(half, preview_h), Sense::hover());
        paint_checker(ui.painter(), r1, 4.0);
        ui.painter()
            .rect_filled(r1, 4.0, display_to_color32(working_disp, color[3]));
        ui.painter().text(
            r1.left_top() + Vec2::new(6.0, 4.0),
            egui::Align2::LEFT_TOP,
            "working",
            egui::TextStyle::Small.resolve(ui.style()),
            Color32::WHITE,
        );

        let display = (cfg.display_transform)([color[0], color[1], color[2]]);
        let (r2, _) = ui.allocate_exact_size(Vec2::new(half, preview_h), Sense::hover());
        paint_checker(ui.painter(), r2, 4.0);
        ui.painter()
            .rect_filled(r2, 4.0, display_to_color32(display, color[3]));
        ui.painter().text(
            r2.left_top() + Vec2::new(6.0, 4.0),
            egui::Align2::LEFT_TOP,
            "display",
            egui::TextStyle::Small.resolve(ui.style()),
            Color32::WHITE,
        );
    });

    ui.add_space(6.0);

    // Compute current HSV (with HDR-capable V). Drives the 2D HS
    // picker + V slider AND the HSV row below — all editors are
    // funneled through this `(h, s, v)` so any change updates both
    // RGB and HSV views consistently.
    let (mut h_deg, mut s, mut v) =
        rgb_to_hsv(color[0], color[1], color[2]);

    // --- Section 2: 2D Hue × Saturation gradient + vertical V slider.
    ui.horizontal(|ui| {
        let hs_size = 180.0;
        if hs_picker(ui, hs_size, &mut h_deg, &mut s) {
            let (r, g, b) = hsv_to_rgb(h_deg, s, v);
            color[0] = r;
            color[1] = g;
            color[2] = b;
        }
        ui.add_space(6.0);
        if v_slider(ui, Vec2::new(22.0, hs_size), h_deg, s, &mut v, cfg.hdr_max) {
            let (r, g, b) = hsv_to_rgb(h_deg, s, v);
            color[0] = r;
            color[1] = g;
            color[2] = b;
        }
        ui.add_space(8.0);

        ui.vertical(|ui| {
            // --- RGB rows: linear vs sRGB-encoded columns.
            ui.label(
                egui::RichText::new("RGB")
                    .small()
                    .color(ui.visuals().weak_text_color()),
            );
            egui::Grid::new("egui_colorpicker_rgb_grid")
                .num_columns(3)
                .spacing([4.0, 2.0])
                .show(ui, |ui| {
                    for (label, ch) in [("R", 0usize), ("G", 1), ("B", 2)] {
                        ui.label(label);
                        ui.add(
                            egui::DragValue::new(&mut color[ch])
                                .range(0.0..=cfg.hdr_max)
                                .speed(0.005)
                                .max_decimals(3),
                        );
                        // sRGB-encoded display column. Edits round-
                        // trip through srgb_to_linear so the working
                        // linear value stays the source of truth.
                        let mut srgb = linear_to_srgb(color[ch]).clamp(0.0, 1.0);
                        if ui
                            .add(
                                egui::DragValue::new(&mut srgb)
                                    .range(0.0..=1.0)
                                    .speed(0.005)
                                    .max_decimals(3),
                            )
                            .changed()
                        {
                            color[ch] = srgb_to_linear(srgb.clamp(0.0, 1.0));
                        }
                        ui.end_row();
                    }
                });

            ui.add_space(4.0);
            ui.label(
                egui::RichText::new("HSV")
                    .small()
                    .color(ui.visuals().weak_text_color()),
            );
            let (orig_h, orig_s, orig_v) = (h_deg, s, v);
            egui::Grid::new("egui_colorpicker_hsv_grid")
                .num_columns(2)
                .spacing([4.0, 2.0])
                .show(ui, |ui| {
                    ui.label("H");
                    ui.add(
                        egui::DragValue::new(&mut h_deg)
                            .range(0.0..=360.0)
                            .speed(0.5)
                            .suffix("°")
                            .max_decimals(1),
                    );
                    ui.end_row();
                    ui.label("S");
                    ui.add(
                        egui::DragValue::new(&mut s)
                            .range(0.0..=1.0)
                            .speed(0.005)
                            .max_decimals(3),
                    );
                    ui.end_row();
                    ui.label("V");
                    ui.add(
                        egui::DragValue::new(&mut v)
                            .range(0.0..=cfg.hdr_max)
                            .speed(0.005)
                            .max_decimals(3),
                    );
                    ui.end_row();
                });
            if (h_deg - orig_h).abs() > 1e-4
                || (s - orig_s).abs() > 1e-4
                || (v - orig_v).abs() > 1e-4
            {
                let (r, g, b) = hsv_to_rgb(h_deg, s, v);
                color[0] = r;
                color[1] = g;
                color[2] = b;
            }
        });
    });

    ui.add_space(4.0);

    // --- Section 3: Hex input. Lossy for HDR (clamped to 0..1 in
    // sRGB), but the round-trip is documented so it's not a
    // surprise.
    ui.horizontal(|ui| {
        ui.label("Hex");
        let mut hex = linear_to_hex(color[0], color[1], color[2]);
        let resp = ui.add(egui::TextEdit::singleline(&mut hex).desired_width(80.0));
        if resp.lost_focus()
            && ui.input(|i| i.key_pressed(egui::Key::Enter))
            && let Some((r, g, b)) = hex_to_linear(&hex)
        {
            color[0] = r;
            color[1] = g;
            color[2] = b;
        }

        // Alpha row sits on the same line so the popup stays
        // compact when alpha is on.
        if cfg.alpha_enabled {
            ui.add_space(12.0);
            ui.label("A");
            ui.add(
                egui::DragValue::new(&mut color[3])
                    .range(0.0..=1.0)
                    .speed(0.005)
                    .max_decimals(3),
            );
        }
    });

    ui.add_space(6.0);

    // --- Section 4: history strip. Click to apply, double-click
    // to remove. The "Save" button pushes the current color.
    let mut history = load_history(ui.ctx());
    let mut history_dirty = false;
    ui.horizontal(|ui| {
        if ui.small_button("save").on_hover_text("Add current color to history").clicked() {
            history.push(*color);
            history_dirty = true;
        }
        let cell = Vec2::new(20.0, 18.0);
        let mut to_remove: Option<usize> = None;
        for (i, entry) in history.entries.iter().enumerate() {
            let (r, resp) = ui.allocate_exact_size(cell, Sense::click());
            let d = (cfg.display_transform)([entry[0], entry[1], entry[2]]);
            paint_checker(ui.painter(), r, 3.0);
            ui.painter().rect_filled(r, 2.0, display_to_color32(d, entry[3]));
            ui.painter().rect_stroke(
                r,
                2.0,
                Stroke::new(1.0, Color32::from_gray(40)),
                egui::StrokeKind::Inside,
            );
            if resp.clicked() {
                *color = *entry;
            }
            if resp.double_clicked() {
                to_remove = Some(i);
            }
            resp.on_hover_text("click = apply · double-click = remove");
        }
        if let Some(i) = to_remove {
            history.entries.remove(i);
            history_dirty = true;
        }
    });
    if history_dirty {
        save_history(ui.ctx(), history);
    }

    // --- Section 5: eyedropper button (host-installed callback).
    if let Some(cb) = cfg.eyedropper.as_mut() {
        ui.add_space(4.0);
        if ui
            .button("Eyedropper")
            .on_hover_text(
                "Sample a color from the viewport. Host app drives the actual sampling.",
            )
            .clicked()
        {
            cb();
        }
    }
}

/// 2D Hue × Saturation gradient + cursor. Returns true on edit.
/// `h` is in degrees `[0, 360)`, `s` in `[0, 1]`.
fn hs_picker(ui: &mut Ui, size: f32, h: &mut f32, s: &mut f32) -> bool {
    let (rect, response) =
        ui.allocate_exact_size(Vec2::splat(size), Sense::click_and_drag());
    let painter = ui.painter();

    // 32×32 mesh: enough resolution that the gradient looks smooth.
    // Vertex-color interpolation isn't HSV-correct, but eye-wise the
    // difference is invisible on a 32-cell grid.
    let n = 32usize;
    let mut mesh = Mesh::default();
    for j in 0..=n {
        for i in 0..=n {
            let u = i as f32 / n as f32;
            let vt = j as f32 / n as f32;
            let h_d = u * 360.0;
            let s_v = 1.0 - vt;
            let (r, g, b) = hsv_to_rgb(h_d, s_v, 1.0);
            let c = display_to_color32(
                [linear_to_srgb(r), linear_to_srgb(g), linear_to_srgb(b)],
                1.0,
            );
            let p = Pos2::new(
                rect.min.x + u * rect.width(),
                rect.min.y + vt * rect.height(),
            );
            mesh.colored_vertex(p, c);
        }
    }
    let stride = (n + 1) as u32;
    for j in 0..n as u32 {
        for i in 0..n as u32 {
            let a = j * stride + i;
            let b = a + 1;
            let c = a + stride;
            let d = c + 1;
            mesh.add_triangle(a, b, c);
            mesh.add_triangle(b, d, c);
        }
    }
    painter.add(Shape::mesh(mesh));

    // Cursor: small crosshair circle.
    let cur = Pos2::new(
        rect.min.x + (*h / 360.0).clamp(0.0, 1.0) * rect.width(),
        rect.min.y + (1.0 - *s).clamp(0.0, 1.0) * rect.height(),
    );
    painter.circle_stroke(cur, 6.0, Stroke::new(2.0, Color32::WHITE));
    painter.circle_stroke(cur, 6.0, Stroke::new(1.0, Color32::BLACK));

    let mut changed = false;
    if response.dragged() || response.clicked() {
        if let Some(pos) = response.interact_pointer_pos() {
            let u = ((pos.x - rect.min.x) / rect.width()).clamp(0.0, 1.0);
            let v = ((pos.y - rect.min.y) / rect.height()).clamp(0.0, 1.0);
            *h = u * 360.0;
            *s = 1.0 - v;
            changed = true;
        }
    }
    changed
}

/// Vertical V slider painted as the current (H, S) at V=0..hdr_max.
/// Returns true on edit.
fn v_slider(
    ui: &mut Ui,
    size: Vec2,
    h_deg: f32,
    s: f32,
    v: &mut f32,
    hdr_max: f32,
) -> bool {
    let (rect, response) = ui.allocate_exact_size(size, Sense::click_and_drag());
    let painter = ui.painter();

    // 8 stops top-to-bottom. Top = hdr_max, bottom = 0.
    let stops = 8usize;
    let mut mesh = Mesh::default();
    for i in 0..=stops {
        let t = i as f32 / stops as f32;
        let v_here = (1.0 - t) * hdr_max;
        let (r, g, b) = hsv_to_rgb(h_deg, s, v_here);
        let c = display_to_color32(
            [linear_to_srgb(r), linear_to_srgb(g), linear_to_srgb(b)],
            1.0,
        );
        let y = rect.min.y + t * rect.height();
        mesh.colored_vertex(Pos2::new(rect.min.x, y), c);
        mesh.colored_vertex(Pos2::new(rect.max.x, y), c);
    }
    for i in 0..stops as u32 {
        let a = i * 2;
        let b = a + 1;
        let c = a + 2;
        let d = a + 3;
        mesh.add_triangle(a, b, c);
        mesh.add_triangle(b, d, c);
    }
    painter.add(Shape::mesh(mesh));

    // Marker for current V (clamped into the visible band).
    let t = 1.0 - (v.clamp(0.0, hdr_max) / hdr_max.max(1e-6));
    let y = rect.min.y + t * rect.height();
    painter.line_segment(
        [Pos2::new(rect.min.x - 2.0, y), Pos2::new(rect.max.x + 2.0, y)],
        Stroke::new(2.0, Color32::WHITE),
    );
    painter.line_segment(
        [Pos2::new(rect.min.x - 2.0, y), Pos2::new(rect.max.x + 2.0, y)],
        Stroke::new(1.0, Color32::BLACK),
    );

    let mut changed = false;
    if response.dragged() || response.clicked() {
        if let Some(pos) = response.interact_pointer_pos() {
            let t = ((pos.y - rect.min.y) / rect.height()).clamp(0.0, 1.0);
            *v = (1.0 - t) * hdr_max;
            changed = true;
        }
    }
    changed
}
