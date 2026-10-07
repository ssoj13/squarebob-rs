use eframe::egui;
use egui_widgets_config::icons;

#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) enum Action {
    Load(String),
    Save(String),
    Create(String),
    Rename { from: String, to: String },
    Delete(String),
}

#[derive(Clone, Default)]
struct State {
    edit: Option<Edit>,
    error: Option<String>,
}

#[derive(Clone)]
struct Edit {
    from: Option<String>,
    name: String,
    focus: bool,
}

fn state_id() -> egui::Id {
    egui::Id::new("squarebob.settings.render_presets")
}

pub(super) fn finish(ctx: &egui::Context, result: Result<(), String>) {
    ctx.data_mut(|data| {
        let state = data.get_temp_mut_or_default::<State>(state_id());
        match result {
            Ok(()) => {
                state.edit = None;
                state.error = None;
            }
            Err(error) => state.error = Some(error),
        }
    });
}

pub(super) fn show(
    ui: &mut egui::Ui,
    names: &[String],
    active: &str,
    dirty: bool,
) -> Option<Action> {
    let mut state = ui
        .ctx()
        .data_mut(|data| data.get_temp_mut_or_default::<State>(state_id()).clone());
    let mut action = None;
    ui.horizontal_wrapped(|ui| {
        ui.label("Presets:");
        for name in names {
            let selected = name == active;
            let title = if selected && dirty {
                format!("{name} *")
            } else {
                name.clone()
            };
            ui.push_id(name, |ui| {
                let response = ui
                    .add(egui::Button::new(title).selected(selected).wrap())
                    .on_hover_text("Click: load settings\nRight-click: save, rename or delete");
                if response.clicked() {
                    action = Some(Action::Load(name.clone()));
                }
                response.context_menu(|ui| {
                    if ui
                        .button(format!("{} Save current settings", icons::SAVE))
                        .clicked()
                    {
                        action = Some(Action::Save(name.clone()));
                        ui.close();
                    }
                    if ui.button("Rename…").clicked() {
                        state.edit = Some(Edit {
                            from: Some(name.clone()),
                            name: name.clone(),
                            focus: true,
                        });
                        state.error = None;
                        ui.close();
                    }
                    ui.separator();
                    if ui
                        .button(format!("{} Delete preset", icons::TRASH))
                        .clicked()
                    {
                        action = Some(Action::Delete(name.clone()));
                        ui.close();
                    }
                });
            });
        }
        if ui
            .button(format!("{} New…", icons::ADD))
            .on_hover_text("Save current settings as a new named preset")
            .clicked()
        {
            state.edit = Some(Edit {
                from: None,
                name: String::new(),
                focus: true,
            });
            state.error = None;
        }
    });

    let mut close = false;
    if let Some(edit) = &mut state.edit {
        let mut open = true;
        egui::Window::new(if edit.from.is_some() {
            "Rename preset"
        } else {
            "Save new preset"
        })
        .id(state_id().with("name"))
        .collapsible(false)
        .resizable(false)
        .open(&mut open)
        .show(ui.ctx(), |ui| {
            ui.label("Preset name");
            let response = ui.add(egui::TextEdit::singleline(&mut edit.name).desired_width(240.0));
            if edit.focus {
                response.request_focus();
                edit.focus = false;
            }
            let name = edit.name.trim();
            let collision =
                names.iter().any(|existing| existing == name) && edit.from.as_deref() != Some(name);
            let valid = !name.is_empty() && !collision;
            if collision {
                ui.colored_label(
                    ui.visuals().error_fg_color,
                    "A preset with this name already exists.",
                );
            }
            let mut submit =
                valid && response.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter));
            ui.horizontal(|ui| {
                submit |= ui
                    .add_enabled(
                        valid,
                        egui::Button::new(if edit.from.is_some() {
                            "Rename"
                        } else {
                            "Save"
                        }),
                    )
                    .clicked();
                close |= ui.button("Cancel").clicked();
            });
            if submit {
                action = Some(match &edit.from {
                    Some(from) => Action::Rename {
                        from: from.clone(),
                        to: name.to_owned(),
                    },
                    None => Action::Create(name.to_owned()),
                });
            }
            close |= ui.input(|i| i.key_pressed(egui::Key::Escape));
            if let Some(error) = &state.error {
                ui.colored_label(ui.visuals().error_fg_color, error);
            }
        });
        close |= !open;
    } else if let Some(error) = &state.error {
        ui.colored_label(ui.visuals().error_fg_color, error);
    }
    if close {
        state.edit = None;
        state.error = None;
    }
    ui.ctx()
        .data_mut(|data| data.insert_temp(state_id(), state));
    action
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    #[ignore = "renders GPU reference screenshots into SQUAREBOB_PRESET_SHOTS"]
    fn preset_button_gpu_screenshots() {
        let directory = std::path::PathBuf::from(
            std::env::var_os("SQUAREBOB_PRESET_SHOTS").expect("set SQUAREBOB_PRESET_SHOTS"),
        );
        std::fs::create_dir_all(&directory).unwrap();
        let gpu = render_core::gpu::GpuContext::new().expect("GPU adapter");
        let ctx = egui::Context::default();
        egui_widgets_config::ensure_icon_font(&ctx);
        let mut renderer = egui_wgpu::Renderer::new(
            &gpu.device,
            egui_display::CANVAS_FORMAT,
            egui_wgpu::RendererOptions {
                dithering: false,
                ..Default::default()
            },
        );
        let mut next = |events| {
            let (output, _) = frame(&ctx, events);
            for (id, deltas) in &output.textures_delta.set {
                for delta in deltas {
                    renderer.update_texture(&gpu.device, &gpu.queue, *id, delta);
                }
            }
            output
        };
        next(vec![]);
        let buttons = next(vec![]);
        let pos = text_position(&buttons, "Cinema");
        next(vec![
            egui::Event::PointerMoved(pos),
            egui::Event::PointerButton {
                pos,
                button: egui::PointerButton::Secondary,
                pressed: true,
                modifiers: egui::Modifiers::NONE,
            },
        ]);
        next(vec![egui::Event::PointerButton {
            pos,
            button: egui::PointerButton::Secondary,
            pressed: false,
            modifiers: egui::Modifiers::NONE,
        }]);
        for _ in 0..20 {
            next(vec![]);
        }
        let menu = next(vec![]);
        let pos = text_position(&menu, "Rename…");
        next(vec![
            egui::Event::PointerMoved(pos),
            egui::Event::PointerButton {
                pos,
                button: egui::PointerButton::Primary,
                pressed: true,
                modifiers: egui::Modifiers::NONE,
            },
        ]);
        next(vec![egui::Event::PointerButton {
            pos,
            button: egui::PointerButton::Primary,
            pressed: false,
            modifiers: egui::Modifiers::NONE,
        }]);
        for _ in 0..20 {
            next(vec![]);
        }
        let edit = next(vec![]);
        drop(next);
        for (name, output) in [
            ("preset-buttons.png", buttons),
            ("preset-context-menu.png", menu),
            ("preset-rename.png", edit),
        ] {
            write_gpu_frame(
                &ctx,
                &mut renderer,
                &gpu,
                output,
                [600, 300],
                &directory,
                name,
            );
        }
        use crate::app::viewport_toolbar::tests::{toolbar_frame, visible_text};
        for (width, expanded, filename) in [
            (1000, true, "viewport-toolbar-wide.png"),
            (280, true, "viewport-toolbar-narrow-left.png"),
            (1000, false, "viewport-toolbar-collapsed.png"),
        ] {
            let ctx = egui::Context::default();
            egui_widgets_config::ensure_icon_font(&ctx);
            let mut app = crate::app::App::default();
            app.render_mode = render_shared::RenderMode::Mode3D;
            app.viewport_toolbar.expanded = expanded;
            app.camera_slots
                .store(0, &app.orbit_camera, &app.render_3d_opts);
            let mut next = |events| {
                let (output, response) = toolbar_frame(&ctx, &mut app, width as f32, events);
                for (id, deltas) in &output.textures_delta.set {
                    for delta in deltas {
                        renderer.update_texture(&gpu.device, &gpu.queue, *id, delta);
                    }
                }
                (output, response)
            };
            for _ in 0..20 {
                next(vec![]);
            }
            let (output, response) = next(vec![]);
            let pointer = egui::pos2(response.rect.left() + 90.0, response.rect.center().y);
            if expanded {
                assert!(visible_text(&output, "2D"));
                if width == 1000 {
                    assert!(visible_text(&output, "CamClip:"));
                }
            } else {
                assert!(!visible_text(&output, "2D"));
                assert!(response.rect.width() < 30.0);
            }
            drop(next);
            write_gpu_frame(
                &ctx,
                &mut renderer,
                &gpu,
                output,
                [width, 220],
                &directory,
                filename,
            );
            if width == 280 {
                let mut next = |events| {
                    let (output, response) = toolbar_frame(&ctx, &mut app, width as f32, events);
                    for (id, deltas) in &output.textures_delta.set {
                        for delta in deltas {
                            renderer.update_texture(&gpu.device, &gpu.queue, *id, delta);
                        }
                    }
                    (output, response)
                };
                next(vec![egui::Event::PointerMoved(pointer)]);
                for _ in 0..5 {
                    next(vec![egui::Event::MouseWheel {
                        unit: egui::MouseWheelUnit::Point,
                        phase: egui::TouchPhase::Move,
                        delta: egui::vec2(-400.0, 0.0),
                        modifiers: egui::Modifiers::NONE,
                    }]);
                }
                for _ in 0..20 {
                    next(vec![]);
                }
                let (output, _) = next(vec![]);
                assert!(visible_text(&output, "CamClip:"));
                assert!(visible_text(&output, "5"));
                drop(next);
                write_gpu_frame(
                    &ctx,
                    &mut renderer,
                    &gpu,
                    output,
                    [width, 220],
                    &directory,
                    "viewport-toolbar-narrow-right.png",
                );
            }
        }
    }

    fn write_gpu_frame(
        ctx: &egui::Context,
        renderer: &mut egui_wgpu::Renderer,
        gpu: &render_core::gpu::GpuContext,
        output: egui::FullOutput,
        size: [u32; 2],
        directory: &std::path::Path,
        name: &str,
    ) {
        let jobs = ctx.tessellate(output.shapes, 1.0);
        let screen = egui_wgpu::ScreenDescriptor {
            size_in_pixels: size,
            pixels_per_point: 1.0,
        };
        let mut present = egui_display::PresentPass::new(
            &gpu.device,
            wgpu::TextureFormat::Rgba8Unorm,
            egui_display::Output::Sdr8,
        );
        present.dither = false;
        let canvas = present.canvas(&gpu.device, size[0], size[1]).clone();
        let mut encoder = gpu.device.create_command_encoder(&Default::default());
        let commands =
            renderer.update_buffers(&gpu.device, &gpu.queue, &mut encoder, &jobs, &screen);
        {
            let mut pass = encoder
                .begin_render_pass(&wgpu::RenderPassDescriptor {
                    label: Some("squarebob.preset-reference"),
                    color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                        view: &canvas,
                        depth_slice: None,
                        resolve_target: None,
                        ops: wgpu::Operations {
                            load: wgpu::LoadOp::Clear(wgpu::Color::BLACK),
                            store: wgpu::StoreOp::Store,
                        },
                    })],
                    ..Default::default()
                })
                .forget_lifetime();
            renderer.render(&mut pass, &jobs, &screen);
        }
        gpu.queue
            .submit(commands.into_iter().chain([encoder.finish()]));
        let capture = present
            .capture(
                &gpu.device,
                &gpu.queue,
                egui_display::Output::Sdr8,
                egui_display::Target {
                    hdr: false,
                    white: 203.0,
                    peak: 1000.0,
                },
            )
            .unwrap()
            .wait(&gpu.device)
            .unwrap();
        capture
            .write_png(std::fs::File::create(directory.join(name)).unwrap())
            .unwrap();
    }

    fn frame(ctx: &egui::Context, events: Vec<egui::Event>) -> (egui::FullOutput, Option<Action>) {
        let mut action = None;
        let output = ctx.run_ui(
            egui::RawInput {
                time: Some(ctx.input(|input| input.time) + 1.0 / 60.0),
                screen_rect: Some(egui::Rect::from_min_size(
                    egui::Pos2::ZERO,
                    egui::vec2(600.0, 300.0),
                )),
                events,
                ..Default::default()
            },
            |ui| {
                egui::CentralPanel::default().show(ui, |ui| {
                    action = show(ui, &["Cinema".into(), "Preview".into()], "Cinema", false);
                });
            },
        );
        (output, action)
    }

    fn text_position(output: &egui::FullOutput, label: &str) -> egui::Pos2 {
        fn find(shape: &egui::Shape, label: &str) -> Option<egui::Pos2> {
            match shape {
                egui::Shape::Text(text) if text.galley.text().ends_with(label) => {
                    Some(text.pos + text.galley.size() / 2.0)
                }
                egui::Shape::Vec(shapes) => shapes.iter().find_map(|shape| find(shape, label)),
                _ => None,
            }
        }
        output
            .shapes
            .iter()
            .find_map(|shape| find(&shape.shape, label))
            .unwrap_or_else(|| panic!("Missing visible label: {label}"))
    }

    fn click(
        ctx: &egui::Context,
        pos: egui::Pos2,
        button: egui::PointerButton,
    ) -> (egui::FullOutput, Option<Action>) {
        frame(
            ctx,
            vec![
                egui::Event::PointerMoved(pos),
                egui::Event::PointerButton {
                    pos,
                    button,
                    pressed: true,
                    modifiers: egui::Modifiers::NONE,
                },
            ],
        );
        frame(
            ctx,
            vec![egui::Event::PointerButton {
                pos,
                button,
                pressed: false,
                modifiers: egui::Modifiers::NONE,
            }],
        )
    }

    #[test]
    fn named_button_primary_click_recalls_and_secondary_click_opens_save_menu() {
        let ctx = egui::Context::default();
        frame(&ctx, vec![]);
        let (output, _) = frame(&ctx, vec![]);
        let preview = text_position(&output, "Preview");
        assert_eq!(
            click(&ctx, preview, egui::PointerButton::Primary).1,
            Some(Action::Load("Preview".into()))
        );
        assert_eq!(click(&ctx, preview, egui::PointerButton::Secondary).1, None);
        let (output, _) = frame(&ctx, vec![]);
        let save = text_position(&output, "Save current settings");
        assert_eq!(
            click(&ctx, save, egui::PointerButton::Primary).1,
            Some(Action::Save("Preview".into()))
        );
    }

    #[test]
    fn context_rename_edits_name_and_failed_save_keeps_editor_open() {
        let ctx = egui::Context::default();
        frame(&ctx, vec![]);
        let (output, _) = frame(&ctx, vec![]);
        click(
            &ctx,
            text_position(&output, "Cinema"),
            egui::PointerButton::Secondary,
        );
        let (output, _) = frame(&ctx, vec![]);
        click(
            &ctx,
            text_position(&output, "Rename…"),
            egui::PointerButton::Primary,
        );
        let state = ctx.data_mut(|data| data.get_temp::<State>(state_id()).unwrap());
        assert_eq!(state.edit.as_ref().unwrap().from.as_deref(), Some("Cinema"));
        assert_eq!(state.edit.as_ref().unwrap().name, "Cinema");
        finish(&ctx, Err("read-only destination".into()));
        let (output, _) = frame(&ctx, vec![]);
        text_position(&output, "read-only destination");
        assert!(ctx.data_mut(|data| data.get_temp::<State>(state_id()).unwrap().edit.is_some()));
        finish(&ctx, Ok(()));
        assert!(ctx.data_mut(|data| data.get_temp::<State>(state_id()).unwrap().edit.is_none()));
    }
}
