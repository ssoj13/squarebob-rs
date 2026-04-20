//! Dock layout and tab rendering for the main panels.

use eframe::egui;
use egui_dock::{DockState, NodeIndex, TabViewer};

use super::App;

/// Dock tab identifiers for the main UI layout.
#[derive(Debug, Clone, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize)]
pub enum DockTab {
    FileView,
    QuadTreeView,
    Extensions,
    Settings,
}

/// Default dock state with all panels visible.
pub fn default_dock_state() -> DockState<DockTab> {
    build_dock_state(true)
}

/// Build dock layout with configurable right panels.
/// Layout: FileView | QuadTreeView | [Settings]
pub fn build_dock_state(show_settings: bool) -> DockState<DockTab> {
    let mut dock_state = DockState::new(vec![DockTab::FileView]);

    // FileView | QuadTreeView (always present)
    let [_file_view, quadtree] = dock_state
        .main_surface_mut()
        .split_right(NodeIndex::root(), 0.20, vec![DockTab::QuadTreeView]);

    if show_settings {
        let _ = dock_state.main_surface_mut().split_right(quadtree, 0.70, vec![DockTab::Settings]);
    }

    dock_state
}

/// Wrapper struct for egui_dock TabViewer implementation.
pub struct DockTabs<'a> {
    pub app: &'a mut App,
}

impl<'a> TabViewer for DockTabs<'a> {
    type Tab = DockTab;

    fn title(&mut self, tab: &mut DockTab) -> egui::WidgetText {
        match tab {
            DockTab::FileView => "Files".into(),
            DockTab::QuadTreeView => "Treemap".into(),
            DockTab::Extensions => "Extensions".into(),
            DockTab::Settings => "Settings".into(),
        }
    }

    fn ui(&mut self, ui: &mut egui::Ui, tab: &mut DockTab) {
        match tab {
            DockTab::FileView => self.app.ui_tree_panel(ui),
            DockTab::QuadTreeView => self.app.ui_treemap(ui),
            DockTab::Extensions => self.app.ui_settings(ui),
            DockTab::Settings => self.app.ui_settings(ui),
        }
    }
}
