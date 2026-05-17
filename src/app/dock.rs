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
    AttributeEditor,
}

/// Default dock state — Settings visible, AE hidden so existing
/// users don't get a surprise re-layout on first launch.
pub fn default_dock_state() -> DockState<DockTab> {
    build_dock_state(true, false)
}

/// Default *memory* layout — must include every known tab so
/// `rebuild_from_layout` can restore a previously-hidden panel.
/// Otherwise opening AE for the first time would be a no-op (the
/// layout would have no slot to put it in).
pub fn default_dock_layout() -> DockState<DockTab> {
    build_dock_state(true, true)
}

/// Build dock layout with configurable right panels.
/// Layout: FileView | QuadTreeView | [Settings, AttributeEditor]
pub fn build_dock_state(show_settings: bool, show_ae: bool) -> DockState<DockTab> {
    let mut dock_state = DockState::new(vec![DockTab::FileView]);

    // FileView | QuadTreeView (always present)
    let [_file_view, quadtree] = dock_state.main_surface_mut().split_right(
        NodeIndex::root(),
        0.20,
        vec![DockTab::QuadTreeView],
    );

    let right_tabs: Vec<DockTab> = [
        (show_settings, DockTab::Settings),
        (show_ae, DockTab::AttributeEditor),
    ]
    .into_iter()
    .filter_map(|(want, tab)| want.then_some(tab))
    .collect();

    if !right_tabs.is_empty() {
        let _ = dock_state
            .main_surface_mut()
            .split_right(quadtree, 0.70, right_tabs);
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
            DockTab::AttributeEditor => "Attribute Editor".into(),
        }
    }

    fn ui(&mut self, ui: &mut egui::Ui, tab: &mut DockTab) {
        match tab {
            DockTab::FileView => self.app.ui_tree_panel(ui),
            DockTab::QuadTreeView => self.app.ui_treemap(ui),
            DockTab::Extensions => self.app.ui_ext_stats(ui),
            DockTab::Settings => self.app.ui_settings(ui),
            DockTab::AttributeEditor => self.app.ui_attribute_editor(ui),
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
            DockTab::AttributeEditor => self.app.show_ae = false,
            DockTab::Extensions => {}
        }
        OnCloseResponse::Close
    }
}
