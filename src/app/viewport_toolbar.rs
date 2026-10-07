use eframe::egui;
use egui_viewport_toolbar::{ToolbarEdge, ToolbarResponse, ViewportToolbar};
use egui_widgets_config::icons;
use render_shared::{CameraType, RenderMode};

use super::App;

impl App {
    pub(super) fn pointer_over_viewport_toolbar(&self, ctx: &egui::Context) -> bool {
        ctx.pointer_latest_pos()
            .is_some_and(|position| self.viewport_toolbar_rect.contains(position))
    }

    pub(super) fn viewport_toolbar(
        &mut self,
        ui: &mut egui::Ui,
        rect: egui::Rect,
    ) -> ToolbarResponse {
        let mut state = self.viewport_toolbar;
        let response = ViewportToolbar {
            edge: ToolbarEdge::Top,
            margin: 0.0,
            height: (ui.text_style_height(&egui::TextStyle::Body) + 12.0)
                .max(
                    ui.spacing().interact_size.y
                        + egui::style::ScrollStyle::solid().allocated_width(),
                )
                .max(34.0),
            id_salt: "squarebob.viewport.toolbar",
            ..Default::default()
        }
        .show(ui, rect, &mut state, |ui| {
            ui.add_enabled_ui(!self.encode_dialog.is_encoding(), |ui| {
                ui.horizontal(|ui| {
                    let previous_mode = (
                        self.render_mode,
                        self.render_3d_opts.path_tracing,
                        self.render_3d_opts.show_wireframe,
                    );
                    if ui
                        .selectable_label(self.render_mode == RenderMode::Mode2D, "2D")
                        .clicked()
                    {
                        self.render_mode = RenderMode::Mode2D;
                    }
                    if ui
                        .selectable_label(
                            self.render_mode == RenderMode::Mode3D
                                && !self.render_3d_opts.path_tracing
                                && !self.render_3d_opts.show_wireframe,
                            "PBR",
                        )
                        .clicked()
                    {
                        self.render_mode = RenderMode::Mode3D;
                        self.render_3d_opts.path_tracing = false;
                        self.render_3d_opts.show_wireframe = false;
                    }
                    if ui
                        .selectable_label(
                            self.render_mode == RenderMode::Mode3D
                                && self.render_3d_opts.path_tracing,
                            "PT",
                        )
                        .clicked()
                    {
                        self.render_mode = RenderMode::Mode3D;
                        self.render_3d_opts.path_tracing = true;
                        self.render_3d_opts.show_wireframe = false;
                    }
                    if previous_mode
                        != (
                            self.render_mode,
                            self.render_3d_opts.path_tracing,
                            self.render_3d_opts.show_wireframe,
                        )
                    {
                        self.on_render_mode_changed(previous_mode.0);
                        self.needs_layout = true;
                        self.needs_render_3d = true;
                        self.preset_dirty = true;
                        if let Some(renderer) = &mut self.renderer_3d {
                            renderer.mark_pt_scene_dirty();
                            renderer.reset_pt_accumulation();
                        }
                    }
                    ui.separator();
                    let physical = self.render_mode == RenderMode::Mode3D
                        && self.render_3d_opts.pt_camera_type == CameraType::Physical;
                    let hold_id = ui.make_persistent_id("physical_camera_exposure_hold");
                    let mut hold = ui
                        .ctx()
                        .data_mut(|data| data.get_temp::<f32>(hold_id).unwrap_or(0.0));
                    let mut exposure_changed = false;
                    ui.add_enabled_ui(physical, |ui| {
                        exposure_changed = egui_viewport_toolbar::exposure_control(
                            ui,
                            &mut self
                                .render_3d_opts
                                .pt_physical_camera
                                .exposure_compensation_ev,
                            &mut hold,
                            -8.0..=8.0,
                        );
                    })
                    .response
                    .on_disabled_hover_text("Exposure compensation requires a physical 3D camera.");
                    ui.ctx().data_mut(|data| data.insert_temp(hold_id, hold));
                    if exposure_changed {
                        self.needs_render_3d = true;
                        self.preset_dirty = true;
                        if let Some(renderer) = &mut self.renderer_3d {
                            renderer.mark_pt_accum_reset();
                        }
                    }
                    ui.separator();
                    if ui
                        .add_enabled(
                            self.render_mode == RenderMode::Mode3D
                                && self.render_3d_opts.path_tracing,
                            egui::DragValue::new(&mut self.render_3d_opts.pt_samples)
                                .range(16..=32768)
                                .suffix(" spp"),
                        )
                        .on_hover_text(
                            "Target samples to converge, shared with Settings → Samples.",
                        )
                        .changed()
                    {
                        self.needs_render_3d = true;
                        self.preset_dirty = true;
                        if let Some(renderer) = &mut self.renderer_3d {
                            renderer.mark_pt_accum_reset();
                        }
                    }
                    if ui
                        .button(icons::GEAR)
                        .on_hover_text("Render and display settings")
                        .clicked()
                    {
                        self.events.emit(crate::events::OpenSettingsEvent);
                        ui.ctx().request_repaint();
                    }
                    ui.separator();
                    let snapshot = ui.button(icons::CAMERA).on_hover_text(
                        "Save viewport as displayed (SDR PNG) · right click: export settings",
                    );
                    if snapshot.clicked() {
                        self.request_viewport_snapshot();
                    }
                    snapshot.context_menu(|ui| {
                        if ui.button("Export settings…").clicked() {
                            self.encode_dialog.export_mode = media_encoder::ExportMode::Sequence;
                            self.show_encode_panel = true;
                            ui.close();
                        }
                    });
                    ui.separator();
                    self.ui_camera_slots(ui);
                });
            });
        });
        self.viewport_toolbar = state;
        self.viewport_toolbar_rect = response.rect;
        response
    }

    fn request_viewport_snapshot(&mut self) {
        let Some(path) = rfd::FileDialog::new()
            .add_filter("PNG image", &["png"])
            .set_file_name("squarebob.png")
            .save_file()
        else {
            return;
        };
        let Some(path) = path.to_str() else {
            self.screenshot_taken = false;
            self.screenshot_error = Some("Screenshot path is not valid Unicode.".into());
            return;
        };
        self.screenshot_path = Some(path.to_owned());
        self.screenshot_delay = Some(0.0);
        self.screenshot_start_time = Some(std::time::Instant::now());
        self.screenshot_taken = false;
        self.screenshot_error = None;
        self.exit_after_screenshot = false;
    }
}

#[cfg(test)]
pub(super) mod tests {
    use super::*;

    pub(crate) fn toolbar_frame(
        ctx: &egui::Context,
        app: &mut App,
        width: f32,
        events: Vec<egui::Event>,
    ) -> (egui::FullOutput, ToolbarResponse) {
        let mut response = ToolbarResponse {
            rect: egui::Rect::NOTHING,
            contains_pointer: false,
            toggled: false,
            moved: false,
        };
        let output = ctx.run_ui(
            egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(
                    egui::Pos2::ZERO,
                    egui::vec2(width, 220.0),
                )),
                events,
                ..Default::default()
            },
            |ui| {
                egui::CentralPanel::default().show(ui, |ui| {
                    let rect = ui.available_rect_before_wrap();
                    ui.painter()
                        .rect_filled(rect, 0.0, egui::Color32::from_gray(24));
                    response = app.viewport_toolbar(ui, rect);
                });
            },
        );
        (output, response)
    }

    pub(crate) fn visible_text(output: &egui::FullOutput, label: &str) -> bool {
        output.shapes.iter().any(|shape| match &shape.shape {
            egui::Shape::Text(text) if text.galley.text() == label => shape
                .clip_rect
                .contains(text.pos + text.galley.rect.center().to_vec2()),
            _ => false,
        })
    }

    #[test]
    fn narrow_viewport_toolbar_horizontal_scroll_reaches_camera_slots() {
        let ctx = egui::Context::default();
        egui_widgets_config::ensure_icon_font(&ctx);
        let mut app = App::default();
        app.render_mode = RenderMode::Mode3D;
        let text_rect = |output: &egui::FullOutput, predicate: fn(&str) -> bool| {
            output
                .shapes
                .iter()
                .find_map(|shape| match &shape.shape {
                    egui::Shape::Text(text) if predicate(text.galley.text()) => {
                        Some(text.galley.rect.translate(text.pos.to_vec2()))
                    }
                    _ => None,
                })
                .unwrap_or_else(|| {
                    let painted_labels: Vec<_> = output
                        .shapes
                        .iter()
                        .filter_map(|shape| match &shape.shape {
                            egui::Shape::Text(text) => Some(text.galley.text().to_owned()),
                            _ => None,
                        })
                        .collect();
                    panic!("toolbar text geometry; painted labels: {painted_labels:?}");
                })
        };
        // egui omits fully clipped text from its paint output. Measure ordering
        // while both controls are visible, then exercise the narrow scroll area.
        for _ in 0..20 {
            toolbar_frame(&ctx, &mut app, 1200.0, vec![]);
        }
        let (wide_output, _) = toolbar_frame(&ctx, &mut app, 1200.0, vec![]);
        let samples = text_rect(&wide_output, |label| label.ends_with(" spp"));
        let camera = text_rect(&wide_output, |label| label == "CamClip:");
        assert!(
            camera.left() >= samples.right(),
            "camera clips must follow samples without overlap"
        );
        for _ in 0..20 {
            let (_, bar) = toolbar_frame(&ctx, &mut app, 280.0, vec![]);
            assert!(bar.rect.width() <= 280.0);
        }
        let (mut output, bar) = toolbar_frame(&ctx, &mut app, 280.0, vec![]);
        assert!(visible_text(&output, "2D"));
        assert!(!visible_text(&output, "CamClip:"));
        let pointer = egui::pos2(bar.rect.left() + 90.0, bar.rect.center().y);
        toolbar_frame(
            &ctx,
            &mut app,
            280.0,
            vec![egui::Event::PointerMoved(pointer)],
        );
        for _ in 0..5 {
            toolbar_frame(
                &ctx,
                &mut app,
                280.0,
                vec![egui::Event::MouseWheel {
                    unit: egui::MouseWheelUnit::Point,
                    phase: egui::TouchPhase::Move,
                    delta: egui::vec2(-400.0, 0.0),
                    modifiers: egui::Modifiers::NONE,
                }],
            );
        }
        for _ in 0..20 {
            toolbar_frame(&ctx, &mut app, 280.0, vec![]);
        }
        (output, _) = toolbar_frame(&ctx, &mut app, 280.0, vec![]);
        assert!(visible_text(&output, "CamClip:"));
        assert!(visible_text(&output, "5"));
        let camera = text_rect(&output, |label| label == "CamClip:");
        assert!(
            camera.bottom()
                <= bar.rect.bottom() - egui::style::ScrollStyle::solid().allocated_width(),
            "horizontal scrollbar must occupy its own space below the controls"
        );
    }

    #[test]
    fn viewport_toolbar_pointer_guard_only_claims_painted_strip() {
        let ctx = egui::Context::default();
        let mut app = App::default();
        app.viewport_toolbar_rect =
            egui::Rect::from_min_size(egui::pos2(10.0, 20.0), egui::vec2(300.0, 34.0));
        let _ = ctx.run_ui(
            egui::RawInput {
                events: vec![egui::Event::PointerMoved(egui::pos2(50.0, 35.0))],
                ..Default::default()
            },
            |_| {},
        );
        assert!(app.pointer_over_viewport_toolbar(&ctx));
        let _ = ctx.run_ui(
            egui::RawInput {
                events: vec![egui::Event::PointerMoved(egui::pos2(50.0, 60.0))],
                ..Default::default()
            },
            |_| {},
        );
        assert!(!app.pointer_over_viewport_toolbar(&ctx));
        app.viewport_toolbar_rect = egui::Rect::NOTHING;
        assert!(!app.pointer_over_viewport_toolbar(&ctx));
    }
}
