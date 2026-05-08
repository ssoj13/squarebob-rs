//! Left file tree panel with expandable directory hierarchy.
//! Uses virtual scrolling for large trees.

use std::collections::HashSet;
use std::path::PathBuf;

use eframe::egui;

use crate::events::{NavigateIntoEvent, SelectPathEvent};
use dirstat_core::DirEntry;

use super::helpers::{collect_all_dir_paths, format_tree_label};
use super::App;

/// Row content height passed to [`egui::ScrollArea::show_rows`].
///
/// **`show_rows` adds `Spacing::item_spacing.y` internally** (`row_stride = h + spacing`).
/// Passing a height that already includes `item_spacing` double-counts it and breaks
/// virtual scrolling, programmatic scroll offsets, and `scroll_to_rect` alignment.
fn row_height_sans_spacing(ui: &egui::Ui) -> f32 {
    let text_h = ui.text_style_height(&egui::TextStyle::Body);
    let widget_h = ui.spacing().interact_size.y;
    widget_h.max(text_h)
}

#[inline]
fn row_stride_y(ui: &egui::Ui, row_height_sans: f32) -> f32 {
    row_height_sans + ui.spacing().item_spacing.y
}

/// Flattened tree node for virtual rendering
struct FlatNode<'a> {
    node: &'a DirEntry,
    depth: usize,
    parent_size: u64,
    is_expanded: bool,
    has_children: bool,
}

impl App {
    /// Render the left file tree panel
    pub(super) fn ui_tree_panel(&mut self, ui: &mut egui::Ui) {
        let mut tree_clicked: Option<PathBuf> = None;
        let mut toggle_expand: Option<PathBuf> = None;

        // Check if this panel area contains the pointer (for F key)
        let panel_hovered = ui.rect_contains_pointer(ui.max_rect());

        // F key: scroll to selected file (only when hovering this panel)
        if panel_hovered
            && ui.input(|i| i.key_pressed(egui::Key::F))
            && self.selected_path.is_some()
        {
            self.scroll_to_selected = true;
        }

        // Rebuild filter cache only when search/mask actually changes
        if self.needs_filter_cache_rebuild() {
            self.rebuild_filtered_paths_cache();
        }

        let mut expand_all = false;
        let mut collapse_all = false;

        ui.set_min_width(ui.available_width());

        // Header with expand/collapse buttons
        ui.horizontal(|ui| {
            ui.heading("Files");
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if ui
                    .small_button("\u{25bc}")
                    .on_hover_text("Expand all")
                    .clicked()
                {
                    expand_all = true;
                }
                if ui
                    .small_button("\u{25b2}")
                    .on_hover_text("Collapse all")
                    .clicked()
                {
                    collapse_all = true;
                }
            });
        });

        // Mask filter UI
        ui.horizontal(|ui| {
            let text_resp = ui.add(
                egui::TextEdit::singleline(&mut self.file_mask_text)
                    .hint_text("*.txt, *.rs")
                    .desired_width(100.0),
            );
            let cb_resp = ui
                .checkbox(&mut self.use_file_mask, "")
                .on_hover_text("Apply mask filter");

            if cb_resp.changed() || (self.use_file_mask && text_resp.changed()) {
                self.rebuild_display_tree();
                self.treemap_tex = None;
            }
        });

        // Build flat list from the same tree that the treemap uses.
        // We use a raw pointer to release the immutable borrow on self so that
        // we can mutate self.expanded / self.scroll_to_selected below while the
        // FlatNode references into the tree remain valid (the tree lives inside
        // self for the duration of this function).
        let mut flat_nodes: Vec<FlatNode> = Vec::new();
        let root_ptr = self.display_root().map(|r| r as *const DirEntry);
        if let Some(ptr) = root_ptr {
            // Safety: the DirEntry lives in self.tree / self.display_tree_cache
            // which are not modified until after flat_nodes is consumed.
            let root = unsafe { &*ptr };
            flatten_tree(
                root,
                0,
                root.size,
                &self.expanded,
                &self.filtered_paths_cache,
                &mut flat_nodes,
            );
        }

        let total_rows = flat_nodes.len();
        let row_h_sans = row_height_sans_spacing(ui);
        let row_stride = row_stride_y(ui, row_h_sans);

        let search_lc = self.search_text.to_lowercase();

        // Find selected index
        let selected_idx = flat_nodes
            .iter()
            .position(|n| Some(&n.node.path) == self.selected_path.as_ref());

        // Remember if we need to scroll (consume the flag)
        let need_scroll = self.scroll_to_selected;
        if need_scroll {
            self.scroll_to_selected = false;
        }

        let scroll_area = egui::ScrollArea::vertical()
            .auto_shrink([false, false])
            .animated(false)
            .id_salt("file_tree_panel");

        // Virtual scroll with row-based rendering
        let mut output = scroll_area.show_rows(ui, row_h_sans, total_rows, |ui, row_range| {
            for i in row_range {
                if let Some(flat) = flat_nodes.get(i) {
                    let is_selected = self.selected_path.as_ref() == Some(&flat.node.path);
                    let matches_search =
                        !search_lc.is_empty() && flat.node.name.to_lowercase().contains(&search_lc);

                    ui.horizontal(|ui| {
                        // Indentation
                        ui.add_space(flat.depth as f32 * 16.0);

                        // Expand/collapse toggle for directories
                        if flat.has_children {
                            let icon = if flat.is_expanded { "▼" } else { "▶" };
                            if ui.small_button(icon).clicked() {
                                toggle_expand = Some(flat.node.path.clone());
                            }
                        } else {
                            ui.add_space(20.0); // Align with buttons
                        }

                        // Label
                        let label =
                            format_tree_label(&flat.node.name, flat.node.size, flat.parent_size);
                        let resp = if matches_search {
                            ui.selectable_label(
                                is_selected,
                                egui::RichText::new(&label)
                                    .strong()
                                    .color(egui::Color32::YELLOW),
                            )
                        } else {
                            ui.selectable_label(is_selected, &label)
                        };

                        if resp.clicked() {
                            tree_clicked = Some(flat.node.path.clone());
                        }
                    });
                }
            }
        });

        // Scroll selected row into view — indices use the same stride as egui `show_rows`:
        // `row_stride = row_height_sans_spacing + item_spacing.y` (see egui scroll_area.rs).
        //
        // Use laid-out content height for clamp (matches egui:`max_offset = content_size - inner`).
        if need_scroll {
            if let Some(idx) = selected_idx {
                let content_h = output.content_size.y;
                let view_h = output.inner_rect.height().max(row_h_sans);
                let max_offset = (content_h - view_h).max(0.0);
                let center_y = idx as f32 * row_stride + row_h_sans * 0.5;
                let new_offset = (center_y - view_h * 0.5).clamp(0.0, max_offset);
                output.state.offset.y = new_offset;
                output.state.store(ui.ctx(), output.id);
                ui.ctx().request_repaint();
            }
        }
        if let Some(path) = toggle_expand {
            if self.expanded.contains(&path) {
                self.expanded.remove(&path);
            } else {
                self.expanded.insert(path);
            }
        }

        // Handle expand/collapse all
        if expand_all {
            if let Some(ptr) = root_ptr {
                let root = unsafe { &*ptr };
                collect_all_dir_paths(root, &mut self.expanded);
            }
        }
        if collapse_all {
            self.expanded.clear();
            if let Some(ptr) = root_ptr {
                let root = unsafe { &*ptr };
                self.expanded.insert(root.path.clone());
            }
        }
        // Single click = select, double click = navigate/zoom
        if let Some(path) = tree_clicked {
            if ui.input(|i| {
                i.pointer
                    .button_double_clicked(egui::PointerButton::Primary)
            }) {
                self.events.emit(NavigateIntoEvent(path));
            } else {
                self.events.emit(SelectPathEvent(path));
            }
        }
    }
}

/// Flatten visible tree nodes into a list
fn flatten_tree<'a>(
    node: &'a DirEntry,
    depth: usize,
    parent_size: u64,
    expanded: &HashSet<PathBuf>,
    filter_cache: &Option<HashSet<PathBuf>>,
    out: &mut Vec<FlatNode<'a>>,
) {
    // Skip filtered nodes
    if let Some(cache) = filter_cache {
        if !cache.contains(&node.path) {
            return;
        }
    }

    let is_expanded = expanded.contains(&node.path);
    let has_children = node.is_dir && !node.children.is_empty();
    let force_open = filter_cache.is_some();

    out.push(FlatNode {
        node,
        depth,
        parent_size,
        is_expanded: is_expanded || force_open,
        has_children,
    });

    // Recurse into expanded directories
    if has_children && (is_expanded || force_open) {
        // Sort children by size descending
        let mut indices: Vec<usize> = (0..node.children.len()).collect();
        indices.sort_by(|&a, &b| node.children[b].size.cmp(&node.children[a].size));

        for &i in &indices {
            flatten_tree(
                &node.children[i],
                depth + 1,
                node.size,
                expanded,
                filter_cache,
                out,
            );
        }
    }
}
