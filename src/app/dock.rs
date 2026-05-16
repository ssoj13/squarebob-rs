//! Dock layout and tab rendering for the main panels.

use eframe::egui;
use egui_dock::tab_viewer::OnCloseResponse;
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
    let [_file_view, quadtree] = dock_state.main_surface_mut().split_right(
        NodeIndex::root(),
        0.20,
        vec![DockTab::QuadTreeView],
    );

    if show_settings {
        let _ = dock_state
            .main_surface_mut()
            .split_right(quadtree, 0.70, vec![DockTab::Settings]);
    }

    dock_state
}

/// `true` iff the given tab is currently present anywhere in the dock.
pub fn dock_contains(state: &DockState<DockTab>, tab: &DockTab) -> bool {
    state.iter_all_tabs().any(|(_, t)| t == tab)
}

/// Remove the first occurrence of `tab` from the dock. Returns `true` if
/// something was actually removed.
pub fn dock_remove(state: &mut DockState<DockTab>, tab: &DockTab) -> bool {
    if let Some(loc) = state.find_tab(tab) {
        state.remove_tab(loc);
        true
    } else {
        false
    }
}

/// Rebuild a visible dock state from a "memory" layout that holds every
/// known tab, by cloning the layout and stripping the tabs that should
/// be hidden.
///
/// This is the restore path for the toolbar toggles: the user closed a
/// panel, kept its column geometry stored in `layout`, and now wants it
/// back exactly where it was — not jammed onto the first leaf.
pub fn rebuild_from_layout(layout: &DockState<DockTab>, visible: &[DockTab]) -> DockState<DockTab> {
    let mut out = layout.clone();
    let hidden: Vec<DockTab> = out
        .iter_all_tabs()
        .map(|(_, t)| t.clone())
        .filter(|t| !visible.contains(t))
        .collect();
    for tab in &hidden {
        if let Some(loc) = out.find_tab(tab) {
            out.remove_tab(loc);
        }
    }
    out
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
            DockTab::Extensions => self.app.ui_ext_stats(ui),
            DockTab::Settings => self.app.ui_settings(ui),
        }
    }

    /// Every panel can be closed via the tab "x" button. The toolbar's
    /// matching toggle un-presses thanks to the flag update in
    /// `on_close`.
    fn closeable(&mut self, _tab: &mut DockTab) -> bool {
        true
    }

    /// Mirror the close action to the App visibility flags so the
    /// toolbar toggles stay in sync. Returning `Close` lets egui_dock
    /// actually remove the tab.
    fn on_close(&mut self, tab: &mut DockTab) -> OnCloseResponse {
        match tab {
            DockTab::FileView => self.app.show_outliner = false,
            DockTab::QuadTreeView => self.app.show_viewport = false,
            DockTab::Settings => self.app.show_settings = false,
            DockTab::Extensions => {}
        }
        OnCloseResponse::Close
    }
}
