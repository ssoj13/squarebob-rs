use eframe::egui;
use render_shared::{CameraType, OrbitCamera, PhysicalCamera, Render3DOptions};
use serde::{Deserialize, Serialize};

pub(super) const COUNT: usize = 5;

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub(super) struct CameraSlots {
    pub slots: [Option<CameraBookmark>; COUNT],
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub(super) struct CameraBookmark {
    pub camera: OrbitCamera,
    pub camera_type: CameraType,
    pub physical_camera: PhysicalCamera,
    pub dof_enabled: bool,
    pub aperture: f32,
    pub focus_distance: f32,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum SlotAction {
    Stored(usize),
    Restored(usize),
    Empty(usize),
}

impl CameraSlots {
    pub fn store(&mut self, index: usize, camera: &OrbitCamera, options: &Render3DOptions) -> bool {
        let Some(slot) = self.slots.get_mut(index) else {
            return false;
        };
        let mut camera = camera.clone();
        camera.stop_inertia();
        camera.cancel_animation();
        *slot = Some(CameraBookmark {
            camera,
            camera_type: options.pt_camera_type,
            physical_camera: options.pt_physical_camera,
            dof_enabled: options.pt_dof_enabled,
            aperture: options.pt_aperture,
            focus_distance: options.pt_focus_distance,
        });
        true
    }

    pub fn restore(
        &self,
        index: usize,
        camera: &mut OrbitCamera,
        options: &mut Render3DOptions,
    ) -> bool {
        let Some(Some(bookmark)) = self.slots.get(index) else {
            return false;
        };
        *camera = bookmark.camera.clone();
        camera.stop_inertia();
        camera.cancel_animation();
        options.pt_camera_type = bookmark.camera_type;
        options.pt_physical_camera = bookmark.physical_camera;
        options.pt_dof_enabled = bookmark.dof_enabled;
        options.pt_aperture = bookmark.aperture;
        options.pt_focus_distance = bookmark.focus_distance;
        true
    }

    pub fn ui(
        &mut self,
        ui: &mut egui::Ui,
        camera: &mut OrbitCamera,
        options: &mut Render3DOptions,
    ) -> Option<SlotAction> {
        let right_to_left = ui.layout().prefer_right_to_left();
        if !right_to_left {
            ui.label("CamClip:");
        }
        let mut action = None;
        for position in 0..COUNT {
            let index = if right_to_left {
                COUNT - 1 - position
            } else {
                position
            };
            let hint = match &self.slots[index] {
                Some(bookmark) => format!(
                    "CamClip {}\nLeft click: paste camera · Right click: copy current camera\n\nTarget {:.3} {:.3} {:.3}\nYaw {:.1}° · Pitch {:.1}° · Distance {:.3}\nFOV {:.1}° · Near {:.3} · Far {:.3}\n{:?} camera · {:.1}mm · f/{:.1} · ISO {:.0}\nFocus {:.3} · DoF {}",
                    index + 1,
                    bookmark.camera.target.x,
                    bookmark.camera.target.y,
                    bookmark.camera.target.z,
                    bookmark.camera.yaw.to_degrees(),
                    bookmark.camera.pitch.to_degrees(),
                    bookmark.camera.distance,
                    bookmark.camera.fov.to_degrees(),
                    bookmark.camera.near,
                    bookmark.camera.far,
                    bookmark.camera_type,
                    bookmark.physical_camera.focal_length_mm,
                    bookmark.physical_camera.f_number,
                    bookmark.physical_camera.iso,
                    bookmark.focus_distance,
                    if bookmark.dof_enabled { "on" } else { "off" }
                ),
                None => format!(
                    "CamClip {} (empty)\nLeft click: nothing to paste · Right click: copy current camera",
                    index + 1
                ),
            };
            let response = ui
                .add(
                    egui::Button::new((index + 1).to_string())
                        .selected(self.slots[index].is_some()),
                )
                .on_hover_text(hint);
            if response.clicked() {
                action = Some(if self.restore(index, camera, options) {
                    SlotAction::Restored(index)
                } else {
                    SlotAction::Empty(index)
                });
            } else if response.secondary_clicked() {
                self.store(index, camera, options);
                action = Some(SlotAction::Stored(index));
            }
        }
        if right_to_left {
            ui.label("CamClip:");
        }
        action
    }
}

impl super::App {
    pub(super) fn ui_camera_slots(&mut self, ui: &mut egui::Ui) {
        let mut action = None;
        ui.add_enabled_ui(!self.encode_dialog.is_encoding(), |ui| {
            action = self
                .camera_slots
                .ui(ui, &mut self.orbit_camera, &mut self.render_3d_opts);
        });
        if matches!(action, Some(SlotAction::Restored(_))) {
            self.needs_layout = true;
            self.needs_render_3d = true;
            self.preset_dirty = true;
            if let Some(renderer) = &mut self.renderer_3d {
                renderer.mark_pt_scene_dirty();
                renderer.reset_pt_accumulation();
            }
            ui.ctx().request_repaint();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn pointer_frame(
        ctx: &egui::Context,
        slots: &mut CameraSlots,
        camera: &mut OrbitCamera,
        options: &mut Render3DOptions,
        right_to_left: bool,
        events: Vec<egui::Event>,
    ) -> (egui::FullOutput, Option<SlotAction>) {
        let mut action = None;
        let output = ctx.run_ui(
            egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(
                    egui::Pos2::ZERO,
                    egui::vec2(600.0, 100.0),
                )),
                events,
                ..Default::default()
            },
            |ui| {
                egui::CentralPanel::default().show(ui, |ui| {
                    ui.with_layout(
                        if right_to_left {
                            egui::Layout::right_to_left(egui::Align::Center)
                        } else {
                            egui::Layout::left_to_right(egui::Align::Center)
                        },
                        |ui| {
                            action = slots.ui(ui, camera, options);
                        },
                    );
                });
            },
        );
        (output, action)
    }

    fn text_position(output: &egui::FullOutput, label: &str) -> egui::Pos2 {
        output
            .shapes
            .iter()
            .find_map(|shape| match &shape.shape {
                egui::Shape::Text(text) if text.galley.text() == label => {
                    Some(text.pos + text.galley.rect.center().to_vec2())
                }
                _ => None,
            })
            .unwrap_or_else(|| panic!("Missing camera slot label {label}"))
    }

    #[test]
    fn camera_clip_primary_pastes_secondary_copies_and_both_layouts_keep_order() {
        for right_to_left in [false, true] {
            let ctx = egui::Context::default();
            let mut slots = CameraSlots::default();
            let mut camera = OrbitCamera::default();
            let mut options = Render3DOptions::default();
            pointer_frame(
                &ctx,
                &mut slots,
                &mut camera,
                &mut options,
                right_to_left,
                vec![],
            );
            let (output, _) = pointer_frame(
                &ctx,
                &mut slots,
                &mut camera,
                &mut options,
                right_to_left,
                vec![],
            );
            assert!(text_position(&output, "CamClip:").x < text_position(&output, "1").x);
            assert!(text_position(&output, "1").x < text_position(&output, "5").x);
            let pos = text_position(&output, "1");
            let click = |button,
                         slots: &mut CameraSlots,
                         camera: &mut OrbitCamera,
                         options: &mut Render3DOptions| {
                pointer_frame(
                    &ctx,
                    slots,
                    camera,
                    options,
                    right_to_left,
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
                pointer_frame(
                    &ctx,
                    slots,
                    camera,
                    options,
                    right_to_left,
                    vec![egui::Event::PointerButton {
                        pos,
                        button,
                        pressed: false,
                        modifiers: egui::Modifiers::NONE,
                    }],
                )
                .1
            };
            let before = ron::to_string(&camera).unwrap();
            assert_eq!(
                click(
                    egui::PointerButton::Primary,
                    &mut slots,
                    &mut camera,
                    &mut options
                ),
                Some(SlotAction::Empty(0))
            );
            assert_eq!(ron::to_string(&camera).unwrap(), before);
            camera.yaw = 0.7;
            options.pt_physical_camera.focal_length_mm = 120.0;
            assert_eq!(
                click(
                    egui::PointerButton::Secondary,
                    &mut slots,
                    &mut camera,
                    &mut options
                ),
                Some(SlotAction::Stored(0))
            );
            camera.yaw = -0.4;
            options.pt_physical_camera.focal_length_mm = 24.0;
            assert_eq!(
                click(
                    egui::PointerButton::Primary,
                    &mut slots,
                    &mut camera,
                    &mut options
                ),
                Some(SlotAction::Restored(0))
            );
            assert_eq!(camera.yaw, 0.7);
            assert_eq!(options.pt_physical_camera.focal_length_mm, 120.0);
        }
    }

    #[test]
    fn camera_clip_roundtrip_restores_canonical_camera_and_all_lens_fields() {
        let mut slots = CameraSlots::default();
        let mut camera = OrbitCamera::default();
        camera.yaw = 1.25;
        camera.pitch = 0.25;
        camera.target = glam::vec3(3.0, 4.0, 5.0);
        camera.near = 0.025;
        camera.far = 2000.0;
        let mut options = Render3DOptions::default();
        options.pt_camera_type = CameraType::Physical;
        options.pt_physical_camera = PhysicalCamera {
            focal_length_mm: 135.0,
            sensor_width_mm: 24.0,
            f_number: 2.8,
            iso: 800.0,
            shutter_seconds: 1.0 / 60.0,
            exposure_compensation_ev: 1.5,
            vignetting: 0.3,
            chromatic_aberration: 0.2,
            bokeh_blades: 6,
            bokeh_anamorphic: 1.4,
            lens_distortion: -0.1,
        };
        options.pt_dof_enabled = true;
        options.pt_aperture = 0.2;
        options.pt_focus_distance = 123.0;
        assert!(slots.store(4, &camera, &options));
        let encoded = ron::to_string(&slots).unwrap();
        let restored: CameraSlots = ron::from_str(&encoded).unwrap();
        let mut other_camera = OrbitCamera::default();
        let mut other_options = Render3DOptions::default();
        other_options.pt_samples = 17;
        assert!(restored.restore(4, &mut other_camera, &mut other_options));
        assert_eq!(other_camera.target, camera.target);
        assert_eq!(other_camera.yaw, camera.yaw);
        assert_eq!(other_camera.pitch, camera.pitch);
        assert_eq!(other_camera.near, camera.near);
        assert_eq!(other_camera.far, camera.far);
        assert_eq!(other_options.pt_camera_type, options.pt_camera_type);
        assert_eq!(other_options.pt_physical_camera, options.pt_physical_camera);
        assert_eq!(other_options.pt_dof_enabled, options.pt_dof_enabled);
        assert_eq!(other_options.pt_aperture, options.pt_aperture);
        assert_eq!(other_options.pt_focus_distance, options.pt_focus_distance);
        assert_eq!(other_options.pt_samples, 17);
        assert!(restored.slots[..4].iter().all(Option::is_none));
    }

    #[test]
    fn empty_or_invalid_camera_clip_is_a_noop_and_stored_camera_does_not_drift() {
        let mut slots = CameraSlots::default();
        let mut camera = OrbitCamera::default();
        let mut options = Render3DOptions::default();
        let before = ron::to_string(&camera).unwrap();
        assert!(!slots.restore(0, &mut camera, &mut options));
        assert!(!slots.restore(COUNT, &mut camera, &mut options));
        assert_eq!(ron::to_string(&camera).unwrap(), before);
        camera.orbit_inertia(100.0, 20.0);
        slots.store(0, &camera, &options);
        let mut pasted = OrbitCamera::default();
        assert!(slots.restore(0, &mut pasted, &mut options));
        let stable = ron::to_string(&pasted).unwrap();
        pasted.update_inertia(0.016, 5.0, 0.0001);
        assert_eq!(ron::to_string(&pasted).unwrap(), stable);
        camera.animate_to(1.0, 0.5, 200.0, glam::Vec3::ONE);
        assert!(camera.is_animating());
        slots.store(1, &camera, &options);
        assert!(slots.restore(1, &mut pasted, &mut options));
        assert!(!pasted.is_animating());
        assert!(!pasted.has_inertia());
    }
}
