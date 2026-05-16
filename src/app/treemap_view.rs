//! Central treemap panel: rendering, hover, selection, context menu, camera controls.

use eframe::egui;
#[cfg(debug_assertions)]
use std::sync::atomic::Ordering;

use crate::events::{NavigateUpEvent, SelectPathEvent};
use crate::renderer::{RenderBackend, RenderMode};
use treemap::GpuRenderer2D;

use super::helpers::{find_node_by_path, fmt_size, path_to_dir};
use super::icons;
#[cfg(any(target_os = "windows", target_os = "macos"))]
use super::shell::{properties_label, shell_properties};
use super::shell::{reveal_label, shell_open, shell_open_terminal, shell_reveal, trash_label};
use super::state::HoverInfo;
use super::App;

impl App {
    /// Render the central treemap/3D panel
    pub(super) fn ui_treemap(&mut self, ui: &mut egui::Ui) {
        let ctx = ui.ctx().clone();
        if self.display_root().is_some() {
            let available = ui.available_size();
            let w = available.x.max(1.0) as u32;
            let h = available.y.max(1.0) as u32;

            // Zero-copy rendering paths (use eframe's wgpu device so egui
            // can sample the texture without a CPU readback round-trip).
            // Both 3D and 2D-GPU benefit; 2D-CPU remains the legacy path.
            let use_callback = self.wgpu_render_state.is_some()
                && self.gpu_context.is_some()
                && (self.render_mode == RenderMode::Mode3D
                    || (self.render_mode == RenderMode::Mode2D
                        && self.render_backend == RenderBackend::Gpu));

            if use_callback {
                if self.render_mode == RenderMode::Mode3D {
                    self.render_3d_callback(ui, w, h);
                } else {
                    self.render_2d_callback(ui, w, h);
                }
            } else {
                // Legacy path: render to texture, then display
                if self.needs_layout
                    || self.last_render_size != (w, h)
                    || (self.render_mode == RenderMode::Mode2D && self.treemap_tex.is_none())
                {
                    self.render_treemap(&ctx, (w, h));
                }

                // Use zero-copy texture if available, fallback to CPU texture
                let tex_id = if self.render_mode == RenderMode::Mode3D {
                    self.render_texture_id
                        .or_else(|| self.treemap_tex.as_ref().map(|t| t.id()))
                } else {
                    self.treemap_tex.as_ref().map(|t| t.id())
                };
                if let Some(id) = tex_id {
                    let img_resp = ui.image(egui::load::SizedTexture::new(
                        id,
                        egui::vec2(w as f32, h as f32),
                    ));
                    let resp = img_resp.interact(
                        egui::Sense::click()
                            .union(egui::Sense::hover())
                            .union(egui::Sense::drag()),
                    );

                    // 3D Camera Controls
                    if self.render_mode == RenderMode::Mode3D {
                        self.handle_3d_camera(&resp, &ctx);
                        self.draw_marquee_overlay(ui, &resp, &ctx);
                    }

                    // 2D Mode interactions
                    if self.render_mode == RenderMode::Mode2D {
                        self.handle_2d_interactions(ui, &resp, &ctx);
                    }

                    // Context menu (both 2D and 3D)
                    self.handle_context_menu(&ctx);
                }
            } // end legacy path
        } else if self.progress.scanning {
            ui.centered_and_justified(|ui| {
                ui.horizontal(|ui| {
                    ui.spinner();
                    if let Some(ref eng) = self.progress.scan_engine_label {
                        ui.label(eng.as_str());
                    }
                });
            });
        } else {
            ui.centered_and_justified(|ui| {
                ui.label("Select a folder and click Scan to begin.");
            });
        }
    }

    /// 3D camera controls (Houdini-style) + hover picking
    fn handle_3d_camera(&mut self, resp: &egui::Response, ctx: &egui::Context) {
        let is_pt = self.render_3d_opts.path_tracing;
        let ctrl_held = ctx.input(|i| i.modifiers.ctrl);
        let shift_held = ctx.input(|i| i.modifiers.shift);
        let w = resp.rect.width().max(1.0) as u32;
        let h = resp.rect.height().max(1.0) as u32;

        // Hover picking (throttled during animation to avoid starving the render loop)
        if let Some(pos) = resp.hover_pos() {
            let lx_f = pos.x - resp.rect.left();
            let ly_f = pos.y - resp.rect.top();
            let lx_u = lx_f.max(0.0) as u32;
            let ly_u = ly_f.max(0.0) as u32;
            // Raster mode: always feed cursor to the GPU picker so readback runs in render_3d_callback.
            // (Throttling only applied below to expensive PT ray picks and animation pick rate.)
            if !is_pt {
                if let Some(r) = &mut self.renderer_3d {
                    r.set_mouse_pos(lx_u, ly_u);
                }
            }
            // Sub-pixel threshold so slow mouse motion still issues picks when the camera is idle
            // (otherwise only camera motion forced `need_render` and hid the stale-hover effect).
            const PICK_MOVE_EPS_SQ: f32 = 0.25; // 0.5 px
            let moved_enough = match self.last_hover_pos_3d {
                Some((px, py)) => {
                    let dx = lx_f - px;
                    let dy = ly_f - py;
                    dx * dx + dy * dy > PICK_MOVE_EPS_SQ
                }
                None => true,
            };
            let pick_interval = if self.render_3d_opts.animate {
                std::time::Duration::from_millis(100)
            } else {
                std::time::Duration::ZERO
            };
            // Skip the per-frame hover pick during shift+drag (marquee selection).
            // Otherwise the cursor sweeping across cubes updates `hovered_3d_id` →
            // sets `needs_render_3d = true` → full PT/PBR re-render every frame,
            // which is dramatically more expensive than the egui-side marquee
            // overlay we actually want to update. The marquee rectangle is
            // drawn by `draw_marquee_overlay` independently of any scene state.
            let should_pick = !shift_held
                && moved_enough
                && self.last_pick_time_3d.elapsed() >= pick_interval;
            if should_pick {
                self.last_pick_time_3d = std::time::Instant::now();
                self.last_hover_pos_3d = Some((lx_f, ly_f));
                if is_pt {
                    // PT mode: CPU ray pick (no needs_layout - just update outline)
                    let mut hit_id: Option<u32> = None;
                    if let Some((origin, dir)) =
                        render_3d::Renderer3D::screen_ray(w, h, &self.orbit_camera, lx_f, ly_f)
                    {
                        if let Some(r) = &mut self.renderer_3d {
                            hit_id = r.pt_pick(origin, dir).map(|(id, _t)| id);
                        }
                    }
                    let id = hit_id.unwrap_or(0);
                    if id != self.hovered_3d_id {
                        self.hovered_3d_id = id;
                        // PT hover is a post-process overlay: redraw, but don't rebuild layout/scene.
                        self.needs_render_3d = true;
                        ctx.request_repaint();
                        if let Some(r) = &mut self.renderer_3d {
                            r.set_hovered_id(id);
                        }
                    }
                    // Update sticky_hover when file found
                    if id != 0 {
                        if let Some(r) = &self.renderer_3d {
                            if let Some(path) = r.path_for_id(id) {
                                let size = r.size_for_id(id).unwrap_or(0);
                                self.sticky_hover = Some((path.clone(), size));
                            }
                        }
                    } else {
                        self.sticky_hover = None;
                    }
                }
                // Raster: hovered_id / sticky_hover are synced after GPU readback in render_3d_callback
                // (same-frame hovered_id() here would always be stale).
            }
            // Show tooltip from sticky_hover (stable even during animation)
            if let Some((ref path, size)) = self.sticky_hover {
                let path_str = path.to_string_lossy().to_string();
                let file_name = path
                    .file_name()
                    .map(|n| n.to_string_lossy().to_string())
                    .unwrap_or_else(|| path_str.clone());
                let ext = path
                    .extension()
                    .map(|e| e.to_string_lossy().to_string())
                    .unwrap_or_default();
                #[allow(deprecated)]
                egui::show_tooltip_at_pointer(
                    ctx,
                    egui::LayerId::new(egui::Order::Tooltip, egui::Id::new("3d_tooltip_layer")),
                    egui::Id::new("3d_tooltip"),
                    |ui: &mut egui::Ui| {
                        ui.set_min_width(250.0);
                        ui.strong(&file_name);
                        ui.label(fmt_size(size));
                        if !ext.is_empty() {
                            ui.label(format!(".{ext}"));
                        }
                        ui.label(&path_str);
                    },
                );
            }
        } else if self.hovered_3d_id != 0 || self.sticky_hover.is_some() {
            // Mouse left the 3D view - clear hover state (no layout rebuild needed)
            self.hovered_3d_id = 0;
            self.last_hover_pos_3d = None;
            self.sticky_hover = None;
        }

        // Ctrl+LMB or MMB: DoF focus pick (PT mode)
        if ((ctrl_held && resp.clicked_by(egui::PointerButton::Primary))
            || resp.clicked_by(egui::PointerButton::Middle))
            && self.render_3d_opts.pt_dof_enabled
            && is_pt
        {
            if let Some(pos) = resp.interact_pointer_pos() {
                let mut focus_dist = 1000.0;
                if let Some((origin, dir)) = render_3d::Renderer3D::screen_ray(
                    w,
                    h,
                    &self.orbit_camera,
                    pos.x - resp.rect.left(),
                    pos.y - resp.rect.top(),
                ) {
                    if let Some(r) = &mut self.renderer_3d {
                        if let Some((_id, t)) = r.pt_pick(origin, dir) {
                            focus_dist = t;
                        }
                        r.reset_pt_accumulation();
                    }
                }
                self.render_3d_opts.pt_focus_distance = focus_dist;
                self.needs_layout = true;
            }
        }

        // LMB click: toggle selection (shift adds to selection without clearing)
        if resp.clicked_by(egui::PointerButton::Primary) && !ctrl_held {
            if let Some(pos) = resp.interact_pointer_pos() {
                let picked_id: Option<u32> = if is_pt {
                    if let Some((origin, dir)) = render_3d::Renderer3D::screen_ray(
                        w,
                        h,
                        &self.orbit_camera,
                        pos.x - resp.rect.left(),
                        pos.y - resp.rect.top(),
                    ) {
                        self.renderer_3d
                            .as_mut()
                            .and_then(|r| r.pt_pick(origin, dir).map(|(id, _)| id))
                    } else {
                        None
                    }
                } else {
                    // Try GPU pick first, then CPU pick
                    let gpu_id = self
                        .renderer_3d
                        .as_ref()
                        .map(|r| r.hovered_id())
                        .filter(|&id| id != 0);
                    if gpu_id.is_some() {
                        gpu_id
                    } else if let Some(root) = self.display_root() {
                        self.renderer_3d
                            .as_ref()
                            .and_then(|r| {
                                r.cpu_pick(
                                    root,
                                    w,
                                    h,
                                    &self.orbit_camera,
                                    &self.render_3d_opts,
                                    &self.opts,
                                    pos.x - resp.rect.left(),
                                    pos.y - resp.rect.top(),
                                )
                            })
                            .and_then(|hit| {
                                self.renderer_3d
                                    .as_ref()
                                    .and_then(|r| r.id_for_path(&hit.path))
                            })
                    } else {
                        None
                    }
                };

                if let Some(id) = picked_id {
                    if shift_held {
                        // Shift+click: toggle in selection (add/remove)
                        if self.selected_3d_ids.contains(&id) {
                            self.selected_3d_ids.remove(&id);
                        } else {
                            self.selected_3d_ids.insert(id);
                        }
                    } else {
                        // Normal click: single select (clears previous)
                        if self.selected_3d_ids.contains(&id) {
                            self.selected_3d_ids.remove(&id);
                        } else {
                            self.selected_3d_ids.clear();
                            self.selected_3d_ids.insert(id);
                        }
                    }
                    // Emit event for sidebar
                    if let Some(r) = &self.renderer_3d {
                        if let Some(path) = r.path_for_id(id).cloned() {
                            log::info!("Picked id={} -> path={:?}", id, path);
                            self.events.emit(SelectPathEvent(path));
                        } else {
                            log::warn!("Picked id={} but path_for_id returned None!", id);
                        }
                    }
                    // Selection is a pure overlay — refresh just the
                    // outline composite (no new PT sample, no PBR
                    // re-shading) so the highlight updates the moment
                    // the user clicks.
                    self.refresh_selection_overlay(ctx);
                } else if !shift_held {
                    // Click on empty space clears selection (but not with shift)
                    self.selected_3d_ids.clear();
                    self.needs_render_3d = true;
                }
            }
        }

        // RMB click: context menu (same as 2D)
        if resp.clicked_by(egui::PointerButton::Secondary) {
            if let Some(pos) = resp.interact_pointer_pos() {
                if is_pt {
                    if let Some((origin, dir)) = render_3d::Renderer3D::screen_ray(
                        w,
                        h,
                        &self.orbit_camera,
                        pos.x - resp.rect.left(),
                        pos.y - resp.rect.top(),
                    ) {
                        if let Some(r) = &mut self.renderer_3d {
                            if let Some((id, _t)) = r.pt_pick(origin, dir) {
                                if let Some(path) = r.path_for_id(id).cloned() {
                                    self.ctx_menu_path = Some(path);
                                    self.ctx_menu_pos = Some(pos);
                                }
                            }
                        }
                    }
                } else {
                    let mut picked: Option<std::path::PathBuf> = None;
                    if self.render_3d_opts.hover_mode != crate::renderer::HoverMode::None {
                        if let Some(r) = &self.renderer_3d {
                            // Use async hovered_id (already updated from hover)
                            let id = r.hovered_id();
                            picked = r.path_for_id(id).cloned();
                        }
                    }
                    if picked.is_none() {
                        if let Some(root) = self.display_root() {
                            if let Some(r) = &self.renderer_3d {
                                let hit = r.cpu_pick(
                                    root,
                                    w,
                                    h,
                                    &self.orbit_camera,
                                    &self.render_3d_opts,
                                    &self.opts,
                                    pos.x - resp.rect.left(),
                                    pos.y - resp.rect.top(),
                                );
                                picked = hit.map(|h| h.path);
                            }
                        }
                    }
                    if let Some(path) = picked {
                        self.ctx_menu_path = Some(path);
                        self.ctx_menu_pos = Some(pos);
                    }
                }
            }
        }

        // LMB - orbit (inertia optional, not with shift - that's marquee select).
        //
        // Camera rotation does NOT change scene geometry — cube positions are
        // world-space and the BVH is camera-independent. Set `needs_render_3d`
        // (repaint only). `render_to_view`'s `opts_hash` still detects LOD-
        // quantized camera changes and will rebuild instances on its own when
        // LOD is enabled. Previously this set `needs_layout = true` which
        // unconditionally invalidated the instance cache AND marked the PT
        // scene dirty, triggering a full PT `upload_scene` (buffer churn and
        // bind-group chain rebuilds) every drag frame — the dominant cost
        // when rotating with PT on.
        if resp.dragged_by(egui::PointerButton::Primary) && !ctrl_held && !shift_held {
            self.orbit_camera.cancel_animation(); // User took control
            let delta = resp.drag_delta();
            if self.render_3d_opts.inertia_enabled {
                self.orbit_camera.orbit_inertia(-delta.x, delta.y);
            } else {
                self.orbit_camera.orbit(-delta.x, delta.y);
            }
            self.needs_render_3d = true;
        }

        // Shift+LMB drag — live marquee. On the first drag frame we
        // snapshot the existing selection; every subsequent frame we
        // reset back to that snapshot and re-add the cubes inside the
        // current rect, then refresh the outline overlay so the user
        // sees in real time which cubes WILL be selected if they
        // release the button now.
        if shift_held && resp.dragged_by(egui::PointerButton::Primary) {
            if let Some(pos) = resp.interact_pointer_pos() {
                if self.marquee_start.is_none() {
                    self.marquee_start = Some(pos);
                    self.marquee_baseline = Some(self.selected_3d_ids.clone());
                }
                if let Some(start) = self.marquee_start {
                    let rect = egui::Rect::from_two_pos(start, pos);
                    if rect.width() > 5.0 || rect.height() > 5.0 {
                        self.commit_marquee_at(rect, &resp.rect, w, h, is_pt, true, ctx);
                    }
                }
                // Egui doesn't auto-repaint inside a drag if nothing
                // else flips render state, so the marquee rectangle
                // would freeze visually. Force a per-frame repaint
                // while the drag is in progress.
                ctx.request_repaint();
            }
        }
        // Shift+LMB released — commit the live preview and drop the
        // baseline. Selection is already in `selected_3d_ids` from the
        // last drag frame; the explicit release-time recompute below
        // covers the edge case where the user did a tiny drag (<5 px)
        // that the preview block ignored.
        if self.marquee_start.is_some()
            && !ctx.input(|i| i.pointer.button_down(egui::PointerButton::Primary))
        {
            if let Some(start) = self.marquee_start.take() {
                if let Some(end) = resp.interact_pointer_pos().or(resp.hover_pos()) {
                    let rect = egui::Rect::from_two_pos(start, end);
                    if rect.width() > 5.0 || rect.height() > 5.0 {
                        self.commit_marquee_at(rect, &resp.rect, w, h, is_pt, shift_held, ctx);
                    }
                }
            }
            self.marquee_baseline = None;
        }
        // MMB - pan (inertia optional)
        if resp.dragged_by(egui::PointerButton::Middle) {
            self.orbit_camera.cancel_animation();
            let delta = resp.drag_delta();
            if self.render_3d_opts.inertia_enabled {
                self.orbit_camera.pan_inertia(delta.x, delta.y);
            } else {
                self.orbit_camera.pan(delta.x, delta.y);
            }
            self.needs_layout = true;
        }
        // RMB - zoom (inertia optional)
        if resp.dragged_by(egui::PointerButton::Secondary) {
            self.orbit_camera.cancel_animation();
            let delta = resp.drag_delta();
            if self.render_3d_opts.inertia_enabled {
                self.orbit_camera.zoom_inertia(-delta.y * 3.0);
            } else {
                self.orbit_camera.zoom(-delta.y * 3.0);
            }
            self.needs_layout = true;
        }
        // Scroll wheel - zoom with inertia (3x speed)
        let scroll = ctx.input(|i| i.smooth_scroll_delta.y);
        if scroll.abs() > 0.1 && resp.hovered() {
            self.orbit_camera.cancel_animation();
            if self.render_3d_opts.inertia_enabled {
                self.orbit_camera.zoom_inertia(-scroll * 1.5);
            } else {
                self.orbit_camera.zoom(-scroll * 1.5);
            }
            self.needs_layout = true;
        }

        // 'F' key - frame view on selection or hovered, else scene
        if ctx.input(|i| i.key_pressed(egui::Key::F)) && resp.hovered() {
            self.frame_selection_or_scene(w as f32, h as f32);
            self.needs_layout = true;
        }

        // 'A' key - fit all (zoom only, keep rotation)
        if ctx.input(|i| i.key_pressed(egui::Key::A)) && resp.hovered() {
            let (scene_w, scene_h) = self.scene_layout_size_or_viewport(w, h);
            self.orbit_camera.zoom_to_fit_scene_for_viewport(
                scene_w,
                scene_h,
                w as f32 / h.max(1) as f32,
            );
            self.needs_layout = true;
        }

        // 'H' key - home (full reset with rotation)
        if ctx.input(|i| i.key_pressed(egui::Key::H)) && resp.hovered() {
            let (scene_w, scene_h) = self.scene_layout_size_or_viewport(w, h);
            self.orbit_camera.animate_to_front_view_for_viewport(
                scene_w,
                scene_h,
                w as f32 / h.max(1) as f32,
            );
            self.needs_layout = true;
        }
    }

    fn scene_layout_size_or_viewport(&self, w: u32, h: u32) -> (f32, f32) {
        self.renderer_3d
            .as_ref()
            .map(|r| r.current_scene_layout_size())
            .map(|(sw, sh)| (sw as f32, sh as f32))
            .unwrap_or((w as f32, h as f32))
    }

    /// Frame view on selection or hovered object (zoom only, keep rotation)
    fn frame_selection_or_scene(&mut self, w: f32, h: f32) {
        // Get bounding box of selected objects
        let bounds = if !self.selected_3d_ids.is_empty() {
            self.compute_selection_bounds()
        } else if self.hovered_3d_id != 0 {
            self.compute_id_bounds(self.hovered_3d_id)
        } else {
            None
        };

        if let Some((center, size)) = bounds {
            // Zoom to frame the bounds (keep rotation)
            let distance = size.length() / self.orbit_camera.fov.tan();
            self.orbit_camera
                .animate_zoom_to(distance.max(50.0), center);
        } else {
            // Fall back to fit scene (keep rotation)
            let (scene_w, scene_h) = self.scene_layout_size_or_viewport(w as u32, h as u32);
            self.orbit_camera
                .zoom_to_fit_scene_for_viewport(scene_w, scene_h, w / h.max(1.0));
        }
    }

    /// Compute bounding box center and size for all selected objects
    fn compute_selection_bounds(&self) -> Option<(glam::Vec3, glam::Vec3)> {
        let r = self.renderer_3d.as_ref()?;
        let mut min = glam::Vec3::splat(f32::MAX);
        let mut max = glam::Vec3::splat(f32::MIN);
        let mut found = false;

        for &id in &self.selected_3d_ids {
            if let Some((c, s)) = self.compute_id_bounds_inner(r, id) {
                min = min.min(c - s * 0.5);
                max = max.max(c + s * 0.5);
                found = true;
            }
        }

        if found {
            Some(((min + max) * 0.5, max - min))
        } else {
            None
        }
    }

    /// Compute bounding box for a single object ID
    fn compute_id_bounds(&self, id: u32) -> Option<(glam::Vec3, glam::Vec3)> {
        let r = self.renderer_3d.as_ref()?;
        self.compute_id_bounds_inner(r, id)
    }

    /// Inner bounds computation
    fn compute_id_bounds_inner(
        &self,
        r: &render_3d::Renderer3D,
        id: u32,
    ) -> Option<(glam::Vec3, glam::Vec3)> {
        // Get instance data for this ID
        r.instance_center_and_size(id)
    }

    /// Draw marquee selection rectangle overlay
    fn draw_marquee_overlay(&self, ui: &egui::Ui, resp: &egui::Response, _ctx: &egui::Context) {
        if let Some(start) = self.marquee_start {
            if let Some(current) = resp.interact_pointer_pos().or(resp.hover_pos()) {
                let rect = egui::Rect::from_two_pos(start, current);
                // Semi-transparent blue fill
                ui.painter().rect_filled(
                    rect,
                    0.0,
                    egui::Color32::from_rgba_unmultiplied(100, 150, 255, 40),
                );
                // Blue border
                ui.painter().rect_stroke(
                    rect,
                    0.0,
                    egui::Stroke::new(1.5, egui::Color32::from_rgb(100, 150, 255)),
                    egui::StrokeKind::Outside,
                );
            }
        }
    }

    /// Select all objects within a screen-space rectangle (marquee selection)
    /// Select objects whose centers fall within marquee rect.
    /// If `add_to_selection` is true, adds to existing selection instead of replacing.
    fn select_objects_in_rect(
        &mut self,
        marquee: egui::Rect,
        view_rect: &egui::Rect,
        w: u32,
        h: u32,
        _is_pt: bool,
        add_to_selection: bool,
    ) {
        let r = match &self.renderer_3d {
            Some(r) => r,
            None => return,
        };

        let instances = match r.cached_instances() {
            Some(i) => i,
            None => return,
        };

        // Convert marquee from screen coords to view-local coords
        let local_marquee = egui::Rect::from_min_max(
            egui::pos2(
                marquee.min.x - view_rect.left(),
                marquee.min.y - view_rect.top(),
            ),
            egui::pos2(
                marquee.max.x - view_rect.left(),
                marquee.max.y - view_rect.top(),
            ),
        );

        // Project each instance center to screen and check if in marquee
        let view = self.orbit_camera.view_matrix();
        let proj = self.orbit_camera.projection_matrix(w as f32 / h as f32);
        let vp = proj * view;

        if !add_to_selection {
            self.selected_3d_ids.clear();
        }

        for inst in instances {
            if inst.object_id == 0 {
                continue;
            }

            let m = glam::Mat4::from_cols_array_2d(&inst.model);
            let center = m.col(3).truncate();
            let clip = vp * center.extend(1.0);

            if clip.w <= 0.0 {
                continue;
            } // Behind camera

            let ndc = clip.truncate() / clip.w;
            let screen_x = (ndc.x + 1.0) * 0.5 * w as f32;
            let screen_y = (1.0 - ndc.y) * 0.5 * h as f32; // Y flipped

            if local_marquee.contains(egui::pos2(screen_x, screen_y)) {
                self.selected_3d_ids.insert(inst.object_id);
            }
        }

        log::info!("Marquee selected {} objects", self.selected_3d_ids.len());
    }

    /// Push the current `selected_3d_ids` into the GPU outline buffer
    /// and refresh the overlay on top of the existing rendered frame.
    /// Skips PT compute / PBR rebuild — the cube colours don't depend
    /// on which IDs are selected, only the outline shader does. Use
    /// after click / shift-click / marquee-complete so the new
    /// highlight shows up immediately without spending a PT sample or
    /// invalidating the accumulator.
    fn refresh_selection_overlay(&mut self, ctx: &egui::Context) {
        if let Some(r) = &mut self.renderer_3d {
            r.set_selected_ids(&self.selected_3d_ids);
        }
        if self.render_3d_opts.path_tracing {
            // PT path: re-composite render_view from the latest colour
            // source (OIDN result if denoised, raw PT accumulator
            // otherwise) plus the outline overlay. No fresh PT sample.
            let source = if self.oidn_display_is_denoised {
                self.oidn_denoiser.as_ref().and_then(|d| d.result_view())
            } else {
                None
            };
            if let Some(r) = self.renderer_3d.as_ref() {
                r.composite_overlay(source);
            }
            ctx.request_repaint();
        } else {
            // PBR/wireframe: cube colours come from a full rasterisation
            // pass, and we don't keep a "no-outline" cache to re-blit
            // from, so a redraw is unavoidable here. `needs_render_3d`
            // (not `needs_layout`) keeps the instance cache intact and
            // the PT accumulator untouched if PT is wired up.
            self.needs_render_3d = true;
        }
    }

    /// Commit a marquee selection at the given rectangle. Resets to
    /// the pre-drag baseline (so cubes that left the rect during the
    /// drag are correctly deselected), runs the cube-in-rect test, then
    /// refreshes the outline overlay. Used by both the live preview
    /// (each drag frame) and the final commit (on button release).
    fn commit_marquee_at(
        &mut self,
        rect: egui::Rect,
        view_rect: &egui::Rect,
        w: u32,
        h: u32,
        is_pt: bool,
        add_to_selection: bool,
        ctx: &egui::Context,
    ) {
        if let Some(baseline) = self.marquee_baseline.as_ref() {
            self.selected_3d_ids = baseline.clone();
        }
        self.select_objects_in_rect(rect, view_rect, w, h, is_pt, add_to_selection);
        self.refresh_selection_overlay(ctx);
    }

    /// 2D mode interactions: selection highlight, hover, clicks, scroll zoom
    fn handle_2d_interactions(
        &mut self,
        ui: &mut egui::Ui,
        resp: &egui::Response,
        ctx: &egui::Context,
    ) {
        // Draw selection highlight
        if let Some(sel_path) = &self.selected_path {
            let sel_rect = self
                .display_root()
                .and_then(|root| find_node_by_path(root, sel_path))
                .map(|n| n.rect.get());
            if let Some([sx, sy, sw, sh]) = sel_rect {
                let origin = resp.rect.left_top();
                let rect = egui::Rect::from_min_size(
                    egui::pos2(origin.x + sx, origin.y + sy),
                    egui::vec2(sw, sh),
                );
                ui.painter().rect_stroke(
                    rect,
                    0.0,
                    egui::Stroke::new(2.0, egui::Color32::WHITE),
                    egui::StrokeKind::Outside,
                );
                ui.painter().rect_stroke(
                    rect.shrink(2.0),
                    0.0,
                    egui::Stroke::new(1.0, egui::Color32::BLACK),
                    egui::StrokeKind::Outside,
                );
            }
        }

        // Hover + highlight
        if let Some(pos) = resp.hover_pos() {
            let lx = pos.x - resp.rect.left();
            let ly = pos.y - resp.rect.top();

            let hit_data = self.hit_test_at(lx, ly).map(|hit| {
                (
                    hit.path.to_string_lossy().to_string(),
                    hit.size,
                    hit.name.clone(),
                    hit.ext.clone(),
                    hit.rect.get(),
                    hit.lod_expand.is_some(),
                )
            });
            if let Some((path_str, size, name, ext, [hx, hy, hw, hh], is_lod_bucket)) = hit_data {
                self.hovered = Some(HoverInfo {
                    path: path_str.clone(),
                    size,
                });

                let origin = resp.rect.left_top();
                let hover_rect = egui::Rect::from_min_size(
                    egui::pos2(origin.x + hx, origin.y + hy),
                    egui::vec2(hw, hh),
                );
                ui.painter().rect_stroke(
                    hover_rect,
                    0.0,
                    egui::Stroke::new(
                        1.0,
                        egui::Color32::from_rgba_premultiplied(255, 255, 255, 180),
                    ),
                    egui::StrokeKind::Outside,
                );

                #[allow(deprecated)]
                egui::show_tooltip_at_pointer(
                    ui.ctx(),
                    egui::LayerId::new(
                        egui::Order::Tooltip,
                        egui::Id::new("treemap_tooltip_layer"),
                    ),
                    egui::Id::new("treemap_tooltip"),
                    |ui: &mut egui::Ui| {
                        ui.set_min_width(250.0);
                        ui.strong(&name);
                        ui.label(fmt_size(size));
                        if !ext.is_empty() {
                            ui.label(format!(".{}", ext));
                        }
                        ui.label(&path_str);
                        if is_lod_bucket {
                            ui.small("Double-click or scroll to expand into files");
                        }
                    },
                );
            }
        } else {
            self.hovered = None;
        }

        // Double-click: zoom deeper
        if resp.double_clicked() {
            if let Some(pos) = resp.interact_pointer_pos() {
                let lx = pos.x - resp.rect.left();
                let ly = pos.y - resp.rect.top();
                let hit_path = self.hit_test_at(lx, ly).map(|h| h.path.clone());
                if let Some(path) = hit_path {
                    self.zoom_step_toward(&path);
                    self.events.emit(SelectPathEvent(path));
                }
            }
        }

        // Left click: select
        if resp.clicked() && !resp.double_clicked() {
            if let Some(pos) = resp.interact_pointer_pos() {
                let lx = pos.x - resp.rect.left();
                let ly = pos.y - resp.rect.top();
                let hit_path = self.hit_test_at(lx, ly).map(|h| h.path.clone());
                if let Some(path) = hit_path {
                    self.events.emit(SelectPathEvent(path));
                }
            }
        }

        // Right click: context menu
        if resp.secondary_clicked() {
            if let Some(pos) = resp.interact_pointer_pos() {
                let lx = pos.x - resp.rect.left();
                let ly = pos.y - resp.rect.top();
                let hit_path = self.hit_test_at(lx, ly).map(|h| h.path.clone());
                if let Some(path) = hit_path {
                    self.ctx_menu_path = Some(path);
                    self.ctx_menu_pos = Some(pos);
                }
            }
        }

        // Mouse wheel: scroll zoom
        let scroll_y = ctx.input(|i| i.smooth_scroll_delta.y);
        let wheel_cooldown = std::time::Duration::from_millis(100);
        if self.last_wheel_zoom.elapsed() >= wheel_cooldown {
            if scroll_y > 5.0 {
                if let Some(pos) = resp.hover_pos() {
                    let lx = pos.x - resp.rect.left();
                    let ly = pos.y - resp.rect.top();
                    let hit_path = self.hit_test_at(lx, ly).map(|h| h.path.clone());
                    if let Some(path) = hit_path {
                        self.zoom_step_toward(&path);
                        self.last_wheel_zoom = std::time::Instant::now();
                    }
                }
            } else if scroll_y < -5.0 {
                self.events.emit(NavigateUpEvent);
                self.last_wheel_zoom = std::time::Instant::now();
            }
        }
    }

    /// Context menu popup (both 2D and 3D)
    fn handle_context_menu(&mut self, ctx: &egui::Context) {
        if self.ctx_menu_path.is_none() {
            return;
        }

        let menu_path = self.ctx_menu_path.clone().unwrap();
        let is_excluded = self.exclusions.contains(&menu_path);
        let mut close = false;
        let mut action_exclude = false;
        let mut action_include = false;

        egui::Area::new(egui::Id::new("ctx_menu"))
            .fixed_pos(self.ctx_menu_pos.unwrap_or(egui::Pos2::ZERO))
            .order(egui::Order::Foreground)
            .show(ctx, |ui| {
                egui::Frame::popup(ui.style()).show(ui, |ui| {
                    ui.set_min_width(180.0);
                    if ui.button("Open").clicked() {
                        shell_open(&menu_path);
                        close = true;
                    }
                    if ui.button("Open folder").clicked() {
                        let dir = path_to_dir(&menu_path);
                        shell_open(dir);
                        close = true;
                    }
                    if ui.button(reveal_label()).clicked() {
                        shell_reveal(&menu_path);
                        close = true;
                    }
                    if ui.button("Open in Terminal").clicked() {
                        shell_open_terminal(&menu_path);
                        close = true;
                    }
                    ui.separator();
                    if ui.button("Copy path").clicked() {
                        ctx.copy_text(menu_path.to_string_lossy().to_string());
                        close = true;
                    }
                    if ui.button("Copy folder path").clicked() {
                        let folder = path_to_dir(&menu_path);
                        ctx.copy_text(folder.to_string_lossy().to_string());
                        close = true;
                    }
                    ui.separator();
                    if is_excluded {
                        if ui.button(format!("{} Include", icons::CHECK)).clicked() {
                            action_include = true;
                            close = true;
                        }
                    } else if ui.button(format!("{} Exclude", icons::X)).clicked() {
                        action_exclude = true;
                        close = true;
                    }
                    ui.separator();
                    #[cfg(any(target_os = "windows", target_os = "macos"))]
                    {
                        if ui.button(properties_label()).clicked() {
                            shell_properties(&menu_path);
                            close = true;
                        }
                    }
                    if ui.button(trash_label()).clicked() {
                        self.request_trash_confirmation(menu_path.clone());
                        close = true;
                    }
                });
            });

        if action_exclude {
            self.exclude_path(&menu_path);
        }
        if action_include {
            self.include_path(&menu_path);
        }
        if close || ctx.input(|i| i.pointer.primary_clicked()) {
            self.ctx_menu_path = None;
        }
    }

    /// After GPU object-id readback, mirror hover into egui state (tooltips). Not valid in the same
    /// frame as `set_mouse_pos` — that only queues a pick; results appear after `pick_from_existing` / `render_to_view`.
    fn sync_treemap_hover_from_3d_gpu(&mut self) {
        let Some(r) = self.renderer_3d.as_ref() else {
            return;
        };
        let id = r.hovered_id();
        if id != self.hovered_3d_id {
            self.hovered_3d_id = id;
        }
        if id != 0 {
            if let Some(path) = r.path_for_id(id) {
                let size = r.size_for_id(id).unwrap_or(0);
                self.sticky_hover = Some((path.clone(), size));
            } else {
                self.sticky_hover = None;
                log::debug!("sync_treemap_hover_from_3d_gpu: id={id} path_for_id None (id_map)");
            }
        } else {
            self.sticky_hover = None;
        }
    }

    /// Zero-copy 3D rendering via register_native_texture
    fn render_3d_callback(&mut self, ui: &mut egui::Ui, w: u32, h: u32) {
        let ctx = ui.ctx().clone();

        // Ensure renderer exists
        if self.renderer_3d.is_none() {
            if let Some(gpu_ctx) = &self.gpu_context {
                let mut r3d = render_3d::Renderer3D::new(gpu_ctx.clone());
                if self.render_3d_opts.env_map_enabled {
                    if let Some(ref path) = self.render_3d_opts.env_map_path {
                        if path.exists() {
                            if let Err(e) = r3d.load_env_map(path) {
                                log::error!("Auto-load env map failed: {e}");
                            }
                        }
                    }
                }
                self.renderer_3d = Some(r3d);
            }
        }

        // Initialize camera to view center if not set
        if self.orbit_camera.target == glam::Vec3::ZERO && w > 0 && h > 0 {
            let (scene_w, scene_h) = self.scene_layout_size_or_viewport(w, h);
            self.orbit_camera.set_front_view_for_viewport(
                scene_w,
                scene_h,
                w as f32 / h.max(1) as f32,
            );
        }

        // Check if we need to render
        let size_changed = self.last_render_size != (w, h);
        // Hover pick: pending pick but scene unchanged — fast readback from existing texture
        let hover_needs_pick = !self.render_3d_opts.path_tracing
            && self.render_3d_opts.hover_mode != crate::renderer::HoverMode::None
            && self
                .renderer_3d
                .as_ref()
                .is_some_and(|r| r.has_pending_pick());
        let pt_throttled = self.render_3d_opts.path_tracing
            && (self.render_3d_opts.pt_auto_spp || self.render_3d_opts.pt_camera_snap);
        let pt_tick_ready = !pt_throttled || self.render_tick_3d;
        let need_render = self.needs_layout
            || self.needs_render_3d
            || size_changed
            || (self.render_3d_opts.path_tracing && pt_tick_ready);

        if !need_render && hover_needs_pick {
            // Fast path: readback updates hovered_id (tooltip), but outline/hover uniforms only refresh
            // in render_to_view — schedule a full pass when the hovered object changes.
            if let Some(r) = &mut self.renderer_3d {
                let id_before = r.hovered_id();
                r.pick_from_existing();
                let id_after = r.hovered_id();
                if self.render_3d_opts.hover_mode != crate::renderer::HoverMode::None
                    && id_after != id_before
                {
                    self.needs_render_3d = true;
                    ctx.request_repaint();
                }
            }
            if !self.render_3d_opts.path_tracing {
                self.sync_treemap_hover_from_3d_gpu();
            }
        }

        if need_render {
            let t0 = std::time::Instant::now();

            // Clone the RenderState (cheap — all internals are Arc'd) so
            // we don't hold an immutable borrow of `self.wgpu_render_state`
            // through the later `maybe_run_oidn_denoise` call which needs
            // `&mut self`.
            let render_state = self.wgpu_render_state.clone().unwrap();
            #[cfg(debug_assertions)]
            let error_scope = render_state
                .device
                .push_error_scope(wgpu::ErrorFilter::Validation);

            // When layout changes, invalidate instances and mark PT scene dirty
            if self.needs_layout {
                if let Some(r) = &mut self.renderer_3d {
                    r.invalidate_instances();
                    r.mark_pt_scene_dirty();
                }
            }

            // Get root - use raw pointer to avoid clone (safe: root lives for duration of render)
            let root_ptr = match self.display_root() {
                Some(r) => r as *const _,
                None => return,
            };

            // Render to texture (safe: root_ptr valid for this scope)
            if let Some(r) = &mut self.renderer_3d {
                // Sync selected IDs for outline rendering
                r.set_selected_ids(&self.selected_3d_ids);
                let root = unsafe { &*root_ptr };
                r.render_to_view(
                    root,
                    w,
                    h,
                    &self.orbit_camera,
                    &self.render_3d_opts,
                    &self.opts,
                );
            }
            self.last_render_frame_3d = self.frame_count;
            self.needs_render_3d = false;
            let t_render = t0.elapsed();

            // OIDN denoise pass. Fires only when PT is active, mode != Off,
            // and (a) the user pressed "Denoise now", or (b) auto-mode is on
            // and we've reached the sample target for the current
            // accumulation (and haven't denoised it yet).
            self.maybe_run_oidn_denoise(w, h);

            // When OIDN landed this frame, blit its result back into the
            // PT render target through the megakernel's ACES+gamma pipeline.
            // This is intentionally *not* a native-egui texture swap —
            // going through `blit_with_source` keeps hover/selection
            // overlays and tone-mapping consistent between raw and
            // denoised display.
            if self.oidn_display_is_denoised {
                if let (Some(r), Some(denoised_view)) = (
                    self.renderer_3d.as_ref(),
                    self.oidn_denoiser.as_ref().and_then(|d| d.result_view()),
                ) {
                    r.composite_overlay(Some(denoised_view));
                }
            }
            self.oidn_last_display_was_denoised = self.oidn_display_is_denoised;

            // Register/update the PT render-target texture with egui. Same
            // texture every frame regardless of denoise state — display
            // source has been mutated in place by the (raw) blit + optional
            // denoised re-blit above.
            if let Some(r) = &self.renderer_3d {
                if let Some(texture) = r.get_render_texture() {
                    if let Some(tex_id) = self.render_texture_id {
                        if size_changed {
                            let mut renderer = render_state.renderer.write();
                            renderer.update_egui_texture_from_wgpu_texture(
                                &render_state.device,
                                &texture.create_view(&wgpu::TextureViewDescriptor::default()),
                                wgpu::FilterMode::Linear,
                                tex_id,
                            );
                        }
                    } else {
                        let mut renderer = render_state.renderer.write();
                        self.render_texture_id = Some(renderer.register_native_texture(
                            &render_state.device,
                            &texture.create_view(&wgpu::TextureViewDescriptor::default()),
                            wgpu::FilterMode::Linear,
                        ));
                    }
                }
            }
            #[cfg(debug_assertions)]
            if let Some(err) = pollster::block_on(error_scope.pop()) {
                log::error!("wgpu validation error after 3D render: {:?}", err);
                self.wgpu_error_flag.store(true, Ordering::SeqCst);
            }
            let t_tex = t0.elapsed();

            let total_ms = t_tex.as_secs_f64() * 1000.0;
            let samples_per_frame = if self.render_3d_opts.path_tracing {
                self.renderer_3d
                    .as_ref()
                    .map(|r| r.pt_samples_per_update())
                    .unwrap_or(0)
            } else {
                0
            };
            let now = std::time::Instant::now();
            let interval_ms = self
                .last_render_instant_3d
                .map(|t| (now - t).as_secs_f64() * 1000.0)
                .unwrap_or(0.0);
            self.last_render_instant_3d = Some(now);
            self.last_frame_ms = total_ms as f32;
            self.last_fps = if interval_ms > 0.0 {
                (1000.0 / interval_ms) as f32
            } else {
                0.0
            };
            // 1-second sliding-window bench: stable readings for benchmarking.
            // Push current frame's interval (which captures actual wall time between
            // frames, including idle/limit gaps), drop entries older than 1s, average.
            self.frame_history
                .push_back((now, interval_ms.max(0.0) as f32));
            let cutoff = now - std::time::Duration::from_secs(1);
            while let Some(&(t, _)) = self.frame_history.front() {
                if t < cutoff {
                    self.frame_history.pop_front();
                } else {
                    break;
                }
            }
            if self.frame_history.len() >= 2 {
                let sum_ms: f32 = self.frame_history.iter().map(|(_, m)| *m).sum();
                let avg = sum_ms / self.frame_history.len() as f32;
                self.avg_frame_ms = avg;
                self.avg_fps = if avg > 0.0 { 1000.0 / avg } else { 0.0 };
            }
            if pt_throttled {
                self.render_tick_3d = false;
            }
            self.last_samples_per_frame = samples_per_frame;
            self.last_samples_per_sec = if samples_per_frame > 0 && total_ms > 0.0 {
                samples_per_frame as f32 / (total_ms as f32 / 1000.0)
            } else {
                0.0
            };

            log::info!(
                "3D frame: render={:.1}ms tex={:.1}ms total={:.1}ms",
                t_render.as_secs_f64() * 1000.0,
                (t_tex - t_render).as_secs_f64() * 1000.0,
                total_ms
            );

            if !self.render_3d_opts.path_tracing {
                self.sync_treemap_hover_from_3d_gpu();
            }

            self.viewport.width = w;
            self.viewport.height = h;
            self.last_render_size = (w, h);
            self.needs_layout = false;
        }

        // Display the texture (always, even if we didn't render this frame)
        log::debug!(
            "render_3d_callback: render_texture_id={:?}",
            self.render_texture_id
        );
        if let Some(id) = self.render_texture_id {
            let img_resp = ui.image(egui::load::SizedTexture::new(
                id,
                egui::vec2(w as f32, h as f32),
            ));
            let resp = img_resp.interact(
                egui::Sense::click()
                    .union(egui::Sense::hover())
                    .union(egui::Sense::drag()),
            );

            // Handle 3D camera controls
            self.handle_3d_camera(&resp, &ctx);

            // Handle context menu
            self.handle_context_menu(&ctx);
        }

        // Request repaint only for continuous modes
        if self.render_3d_opts.path_tracing && !pt_throttled {
            // PT: repaint continuously only when not throttled
            ctx.request_repaint();
        }
    }

    /// Zero-copy 2D treemap rendering via register_native_texture.
    /// Mirrors `render_3d_callback` for the 2D-GPU path. Only runs when
    /// the renderer's `GpuContext` was constructed from eframe's device
    /// (so the rendered texture can be sampled by egui directly without
    /// a CPU readback round-trip).
    ///
    /// Architecture note: the same `render_texture_id` field on App is
    /// reused for whichever mode is currently rendering (3D, 2D-GPU,
    /// or — when it lands — the PT denoiser output). Mode/backend
    /// switches clear this field so a stale TextureId doesn't display.
    fn render_2d_callback(&mut self, ui: &mut egui::Ui, w: u32, h: u32) {
        let ctx = ui.ctx().clone();

        // Lazy-init the GPU 2D renderer with the (eframe-backed) GpuContext.
        if self.renderer_2d_gpu.is_none() {
            if let Some(gpu_ctx) = &self.gpu_context {
                self.renderer_2d_gpu = Some(GpuRenderer2D::new(gpu_ctx.clone()));
            }
        }

        let size_changed = self.last_render_size != (w, h);
        let need_render = self.needs_layout || size_changed || self.render_texture_id.is_none();

        if need_render {
            self.viewport.width = w;
            self.viewport.height = h;
            let render_state = self.wgpu_render_state.as_ref().unwrap();

            // Render into the renderer's internal texture (no readback).
            let mut renderer = self.renderer_2d_gpu.take();
            let drew = if let Some(r) = &mut renderer {
                let Some(root) = self.display_root() else {
                    self.renderer_2d_gpu = renderer;
                    return;
                };
                r.render_to_texture(root, &self.viewport, &self.opts)
            } else {
                false
            };

            // Register the texture with egui, or update the binding on resize.
            if drew {
                if let Some(r) = &renderer {
                    if let Some(texture) = r.get_render_texture() {
                        let view = texture.create_view(&wgpu::TextureViewDescriptor::default());
                        let mut egui_renderer = render_state.renderer.write();
                        if let Some(tex_id) = self.render_texture_id {
                            if size_changed {
                                egui_renderer.update_egui_texture_from_wgpu_texture(
                                    &render_state.device,
                                    &view,
                                    wgpu::FilterMode::Linear,
                                    tex_id,
                                );
                            }
                        } else {
                            self.render_texture_id = Some(egui_renderer.register_native_texture(
                                &render_state.device,
                                &view,
                                wgpu::FilterMode::Linear,
                            ));
                        }
                    }
                }
            }
            self.renderer_2d_gpu = renderer;

            self.last_render_size = (w, h);
            self.needs_layout = false;
        }

        // Display the texture + 2D interactions
        if let Some(id) = self.render_texture_id {
            let img_resp = ui.image(egui::load::SizedTexture::new(
                id,
                egui::vec2(w as f32, h as f32),
            ));
            let resp = img_resp.interact(
                egui::Sense::click()
                    .union(egui::Sense::hover())
                    .union(egui::Sense::drag()),
            );
            self.handle_2d_interactions(ui, &resp, &ctx);
            self.handle_context_menu(&ctx);
        }
    }
}

impl App {
    /// Decide whether OIDN should run this frame and, if so, dispatch a
    /// single denoise pass. Sets `self.oidn_display_is_denoised` to indicate
    /// which texture the registration block downstream should bind for
    /// display.
    ///
    /// Trigger logic (one-shot — not per-frame):
    /// - Manual: user pressed "Denoise now" → `oidn_run_requested = true`.
    /// - Auto: `pt_oidn_auto && current_spp >= target_spp && !already_denoised`.
    ///
    /// Camera/scene changes drop `pt_frame_count` back toward 0 — we detect
    /// that and clear `oidn_denoised_this_accumulation` so the next target-spp
    /// arrival fires OIDN again.
    pub(super) fn maybe_run_oidn_denoise(&mut self, w: u32, h: u32) {
        use pt_denoise_oidn::OidnDenoiser;
        use render_shared::OidnModeOption;

        // PT must be running and OIDN enabled, otherwise force raw display.
        let mode_opt = self.render_3d_opts.pt_oidn_mode;
        log::trace!(
            "maybe_run_oidn_denoise enter: path_tracing={} mode={:?} run_requested={} auto={}",
            self.render_3d_opts.path_tracing,
            mode_opt,
            self.oidn_run_requested,
            self.render_3d_opts.pt_oidn_auto,
        );
        if !self.render_3d_opts.path_tracing || mode_opt == OidnModeOption::Off {
            self.oidn_display_is_denoised = false;
            // Honor any pending manual click only when PT comes back up.
            if !self.render_3d_opts.path_tracing {
                self.oidn_run_requested = false;
            }
            log::trace!("OIDN: skip (PT off or mode=Off)");
            return;
        }

        let Some(r) = self.renderer_3d.as_ref() else {
            self.oidn_display_is_denoised = false;
            log::trace!("OIDN: skip (renderer_3d=None)");
            return;
        };
        let current_spp = r.pt_frame_count();
        let target_spp = r.pt_target_spp();
        log::trace!(
            "OIDN trigger eval: current_spp={} target_spp={} denoised_this_acc={} last_frame={}",
            current_spp, target_spp,
            self.oidn_denoised_this_accumulation, self.oidn_last_frame_count
        );

        // Detect accumulation reset (camera / scene change).
        if current_spp < self.oidn_last_frame_count {
            self.oidn_denoised_this_accumulation = false;
            self.oidn_display_is_denoised = false;
            self.oidn_last_interval_spp = 0;
        }
        self.oidn_last_frame_count = current_spp;

        let manual = self.oidn_run_requested;
        let auto_final = self.render_3d_opts.pt_oidn_auto
            && target_spp > 0
            && current_spp >= target_spp
            && !self.oidn_denoised_this_accumulation;
        // Periodic re-run every N samples. Only when interval > 0 and PT
        // hasn't yet reached the final target (otherwise `auto_final` will
        // handle it). `current_spp - last_interval >= interval` keeps us
        // from re-firing on every frame past a multiple of interval.
        let interval = self.render_3d_opts.pt_oidn_interval;
        let auto_interval = self.render_3d_opts.pt_oidn_auto
            && interval > 0
            && current_spp >= interval
            && current_spp.saturating_sub(self.oidn_last_interval_spp) >= interval;
        let auto = auto_final || auto_interval;

        log::trace!(
            "OIDN trigger result: manual={} auto={} → run={}",
            manual, auto, manual || auto
        );
        if !(manual || auto) {
            return;
        }

        let Some(gpu_ctx) = self.gpu_context.clone() else {
            return;
        };
        let Some(output_tex_view) = r.pt_output_texture() else {
            return;
        };
        // `pt_output_texture` returns `&wgpu::Texture` borrowed from the
        // renderer; we need an owned clone to pass into denoise() which
        // also borrows `r` indirectly. wgpu textures are cheap to clone.
        let output_tex = output_tex_view.clone();
        let (pt_w, pt_h) = r.pt_dimensions().unwrap_or((w, h));
        let albedo = r.pt_albedo_buffer().cloned();
        let normal = r.pt_normal_buffer().cloned();

        // Lazy build (or rebuild on resize).
        let need_new = match &self.oidn_denoiser {
            None => true,
            Some(_d) => false, // resize() inside handles dims mismatch
        };
        if need_new {
            // External weights dir is optional now — `embed-hdr` on
            // `oidn-rs` bakes the HDR TZA blobs into the binary, so we
            // can run end-to-end without any `data/oidn-weights/` on
            // disk. The dir, when present, is still consulted as a
            // fallback (env override / clean_aux / lightmap modes).
            let weights_dir = match pt_denoise_oidn::resolve_weights_dir() {
                Ok(dir) => Some(dir),
                Err(e) => {
                    log::info!(
                        "OIDN: no external weights dir found ({e}); \
                         falling back to embedded blobs."
                    );
                    None
                }
            };
            self.oidn_denoiser =
                Some(OidnDenoiser::new(&gpu_ctx, pt_w, pt_h, weights_dir));
        }

        let Some(denoiser) = self.oidn_denoiser.as_mut() else {
            return;
        };
        denoiser.resize(&gpu_ctx, pt_w, pt_h);
        denoiser.set_mode(map_mode(mode_opt));
        denoiser.set_quality(map_quality(self.render_3d_opts.pt_oidn_quality));
        denoiser.set_input_clamp(self.render_3d_opts.pt_oidn_clamp);

        let encoder = gpu_ctx.device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
            label: Some("oidn_denoise_encoder"),
        });
        match denoiser.denoise(&gpu_ctx, encoder, &output_tex, albedo.as_ref(), normal.as_ref()) {
            Ok(()) => {
                self.oidn_last_latency_ms = denoiser.last_latency_ms();
                self.oidn_denoised_this_accumulation = true;
                self.oidn_run_requested = false;
                self.oidn_display_is_denoised = true;
                self.oidn_last_interval_spp = current_spp;
            }
            Err(e) => {
                log::warn!("OIDN denoise failed: {e}");
                self.oidn_run_requested = false;
                self.oidn_display_is_denoised = false;
            }
        }

    }
}

fn map_mode(m: render_shared::OidnModeOption) -> pt_denoise_oidn::OidnMode {
    use pt_denoise_oidn::OidnMode;
    use render_shared::OidnModeOption;
    match m {
        OidnModeOption::Off => OidnMode::Off,
        OidnModeOption::Color => OidnMode::Color,
        OidnModeOption::ColorAlbedo => OidnMode::ColorAlbedo,
        OidnModeOption::ColorAlbedoNormal => OidnMode::ColorAlbedoNormal,
    }
}

fn map_quality(q: render_shared::OidnQualityOption) -> pt_denoise_oidn::Quality {
    use pt_denoise_oidn::Quality;
    use render_shared::OidnQualityOption;
    match q {
        OidnQualityOption::Large => Quality::High,
        OidnQualityOption::Base => Quality::Balanced,
        OidnQualityOption::Small => Quality::Fast,
    }
}
