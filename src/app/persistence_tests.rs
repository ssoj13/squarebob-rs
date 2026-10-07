use super::*;
use eframe::{App as _, Storage as _};

#[test]
fn typed_state_preserves_dock_sentinels_and_all_settings() {
    let mut app = App::default();
    app.render_3d_opts.color_pipeline.codepath = color_pipeline::ColorCodepath::Cpu;
    app.render_3d_opts.pt_physical_camera.iso = 240.0;
    app.render_3d_opts.pt_physical_camera.f_number = 1.0;
    app.orbit_camera.yaw = 0.75;
    app.orbit_camera.near = 0.025;
    app.camera_slots
        .store(4, &app.orbit_camera, &app.render_3d_opts);
    app.viewport_toolbar.expanded = false;
    app.viewport_toolbar.vertical_offset = 21.0;
    let window = app
        .dock_layout
        .add_window(vec![crate::app::dock::DockTab::Extensions]);
    let floating = app.dock_layout.get_window_state_mut(window).unwrap();
    floating.set_position(egui::pos2(32.0, 64.0));
    floating.set_size(egui::vec2(320.0, 180.0));
    let path = std::env::temp_dir().join(format!("squarebob-state-{}.ron", std::process::id()));
    let mut storage = crate::display_host::SessionStorage::open(path).unwrap();
    app.save(&mut storage);
    let text = storage.get_string("squarebob_state").unwrap();
    let restored = PersistState::decode(&text).unwrap();
    assert!(text.contains("inf"));
    assert_eq!(ron::to_string(&restored).unwrap(), text);
    assert!(crate::app::dock::dock_contains(
        &restored.dock_layout,
        &crate::app::dock::DockTab::AttributeEditor
    ));
    assert_eq!(
        restored.render_3d_opts.color_pipeline.codepath,
        color_pipeline::ColorCodepath::Cpu
    );
    assert_eq!(restored.render_3d_opts.pt_physical_camera.iso, 240.0);
    assert_eq!(restored.render_3d_opts.pt_physical_camera.f_number, 1.0);
    let clip = restored.camera_slots.slots[4].as_ref().unwrap();
    assert_eq!(clip.camera.yaw, 0.75);
    assert_eq!(clip.camera.near, 0.025);
    assert_eq!(clip.physical_camera.iso, 240.0);
    assert_eq!(clip.physical_camera.f_number, 1.0);
    assert!(!restored.viewport_toolbar.expanded);
    assert_eq!(restored.viewport_toolbar.vertical_offset, 21.0);
    assert!(PersistState::decode(&serde_json::to_string(&restored).unwrap()).is_err());
}
