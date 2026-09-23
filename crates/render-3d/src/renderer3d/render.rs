//! Per-frame 3D render entry point: `Renderer3D::render_to_view`.
//!
//! Extracted from `lib.rs` in the post-sprint-3 modularization
//! pass. `render_to_view` is still a method of `Renderer3D` —
//! `impl Renderer3D` is re-opened here.

use std::sync::Arc;

use render_shared::{HoverMode, OrbitCamera, Render3DOptions};
use squarebob_core::DirEntry;
use treemap::TreeMapOptions;

use crate::geometry::{self, NUM_INDICES};
use crate::targets::{DynamicBindGroups, RenderTargets};
use crate::{Renderer3D, pt};

impl Renderer3D {
    pub fn render_to_view(
        &mut self,
        root: &DirEntry,
        width: u32,
        height: u32,
        camera: &OrbitCamera,
        opts: &Render3DOptions,
        treemap_opts: &TreeMapOptions,
        selected_ids: Option<&mut std::collections::HashSet<u32>>,
    ) -> Result<(), render_core::ReadbackError> {
        use log::{debug, info, warn};
        let render_start = std::time::Instant::now();
        info!(
            "=== render_to_view START: {}x{}, PT={}, wire={} ===",
            width, height, opts.path_tracing, opts.show_wireframe
        );

        if width == 0 || height == 0 {
            warn!("render_to_view: zero size, skipping");
            return Ok(());
        }
        let (opts, instances_arc, cache_valid) =
            self.prepare_scene(root, width, height, camera, opts, treemap_opts)?;
        if let Some(ids) = selected_ids {
            ids.clone_from(&self.selected_ids);
        }
        let opts = &opts;
        let instances = instances_arc.as_slice();
        info!(
            "render_to_view: instance_count={}, cache_valid={}",
            self.instance_count, cache_valid
        );
        if instances.is_empty() {
            self.update_uniforms(camera, opts, width, height, 0);
            let state = self
                .render_state
                .as_ref()
                .expect("prepare_scene builds targets");
            let mut encoder =
                self.ctx
                    .device
                    .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                        label: Some("Empty 3D Encoder"),
                    });
            self.encode_passes(&mut encoder, &state.targets, &state.dyn_bgs, opts, 0);
            self.ctx.queue.submit(std::iter::once(encoder.finish()));
            return Ok(());
        }

        // Match outline/hover uniforms to the *current* cursor: read last frame's object_id buffer
        // at pending pixel before encoding (full render still ends with readback from *this* frame).
        // PT mode skips this path — it uses CPU ray pick (`pt_pick`) driven from the UI thread,
        // not the GPU readback used by PBR/wireframe.
        if !opts.path_tracing && cache_valid && self.instance_count > 0 {
            if let Some((px, py)) = self.picking.pending_pick {
                self.picking.ensure_readback(&self.ctx.device, width)?;
                self.pick_from_existing()?;
                self.picking.request_pick(px, py);
            }
        }

        let hovered_id = self.picking.hovered_id;

        self.update_uniforms(camera, opts, width, height, hovered_id);
        let cam_pos = camera.position();
        info!(
            "render_to_view: camera pos=({:.1},{:.1},{:.1}), dist={:.1}",
            cam_pos.x, cam_pos.y, cam_pos.z, camera.distance
        );

        let mut encoder = self
            .ctx
            .device
            .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                label: Some("3D Encoder (zero-copy)"),
            });

        // Path tracing mode
        if opts.path_tracing {
            info!("render_to_view: PATH TRACING mode");
            drop(encoder);
            // Arc clone to break borrow conflict (cheap - only refcount bump)
            let instances_pt = Arc::clone(&instances_arc);
            let num_cubes = instances_pt.len();
            pt::render_path_traced_no_readback(
                self.pt.pt_backend_kind,
                self,
                &instances_pt,
                camera,
                opts,
                width,
                height,
            )?;

            // PT uses CPU ray picking, but the outline shader still needs a
            // current Object ID texture. Keep it ready for selection-only
            // recomposites that do not run another full PT frame.
            let has_active_overlay = !self.selected_ids.is_empty() || hovered_id != 0;
            if opts.hover_mode != HoverMode::None {
                let state = self
                    .render_state
                    .as_ref()
                    .expect("render_state not built — call ensure_render_targets before render");
                let ib = self.instance_buffer.as_ref().expect(
                    "instance_buffer not built — collect_cubes must upload before encode_passes",
                );

                let mut enc =
                    self.ctx
                        .device
                        .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                            label: Some("PT Outline Encoder"),
                        });

                self.encode_object_id_pass(&mut enc, &state.targets, ib, opts.double_sided);
                if has_active_overlay {
                    self.encode_outline_pass(&mut enc, &state.targets, &state.dyn_bgs);
                }

                self.ctx.queue.submit(std::iter::once(enc.finish()));
            }

            let total_ms = render_start.elapsed().as_secs_f64() * 1000.0;
            debug!(
                "PT render (zero-copy): {:.2}ms ({} cubes)",
                total_ms, num_cubes
            );
            return Ok(());
        }

        // Encode PBR/wireframe passes — bundle guarantees both halves valid
        // for the same size, so one Some-check covers both reads.
        let state = self
            .render_state
            .as_ref()
            .expect("render_state not built — call ensure_render_targets before render");
        info!(
            "render_to_view: calling encode_passes, targets {:?}",
            state.targets.size
        );
        self.encode_passes(
            &mut encoder,
            &state.targets,
            &state.dyn_bgs,
            opts,
            hovered_id,
        );
        info!("render_to_view: encode_passes done, submitting");

        // Submit picking readback (uses pending_pick set by set_mouse_pos)
        self.picking.submit_readback(
            &mut encoder,
            &state.targets.object_id_texture,
            state.targets.size,
        )?;

        self.ctx.queue.submit(std::iter::once(encoder.finish()));

        // Blocking poll to read pick result (sync like alembic-rs)
        self.picking.poll_result(&self.ctx.device)?;

        let total_ms = render_start.elapsed().as_secs_f64() * 1000.0;
        let mode = if opts.show_wireframe {
            "Wire"
        } else if opts.xray_alpha < 1.0 {
            "XRay"
        } else {
            "PBR"
        };
        info!(
            "{} render (zero-copy): {:.2}ms ({} cubes)",
            mode,
            total_ms,
            instances.len()
        );
        Ok(())
    }

    // NOTE: render_to_command_buffer and render_path_traced_to_buffer removed (unused callback path)

    /// Path tracing without readback (called from render_to_view)
    pub(crate) fn render_path_traced_no_readback(
        &mut self,
        instances: &[geometry::CubeInstance],
        camera: &OrbitCamera,
        opts: &Render3DOptions,
        width: u32,
        height: u32,
    ) -> Result<(), render_core::ReadbackError> {
        pt::megakernel::render_path_traced_no_readback(self, instances, camera, opts, width, height)
    }

    // ========================================================================
    // Picking / outline shared passes
    // ========================================================================
    //
    // These two helpers eliminate the structural duplication between
    // the PBR/wireframe flow (`encode_passes` in `lib.rs`) and the PT
    // outline+picking encoder (further up in this file). Both modes
    // now call exactly the same code, so future depth/blend/format
    // tweaks land once and apply uniformly.

    /// Render the per-pixel u32 Object ID texture used by hover
    /// picking. Caller gates on `opts.hover_mode != None`; the pass
    /// itself runs unconditionally so `picking::pick_from_existing`
    /// can read out a fresh ID texture even when nothing is hovered
    /// or selected yet (otherwise the overlay can never bootstrap).
    pub(crate) fn encode_object_id_pass(
        &self,
        encoder: &mut wgpu::CommandEncoder,
        targets: &RenderTargets,
        ib: &wgpu::Buffer,
        double_sided: bool,
    ) {
        let pipe = if double_sided {
            &self.pipes.object_id_double
        } else {
            &self.pipes.object_id
        };
        let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("Object ID"),
            color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                view: &targets.object_id_view,
                resolve_target: None,
                depth_slice: None,
                ops: wgpu::Operations {
                    load: wgpu::LoadOp::Clear(wgpu::Color::BLACK),
                    store: wgpu::StoreOp::Store,
                },
            })],
            depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment {
                view: &targets.depth_view,
                depth_ops: Some(wgpu::Operations {
                    load: wgpu::LoadOp::Clear(0.0), // reversed-Z: far = 0.0
                    store: wgpu::StoreOp::Discard,
                }),
                stencil_ops: None,
            }),
            ..Default::default()
        });
        pass.set_pipeline(pipe);
        pass.set_bind_group(0, &self.obj_id_bg0, &[]);
        pass.set_vertex_buffer(0, self.vertex_buffer.slice(..));
        pass.set_vertex_buffer(1, ib.slice(..));
        pass.set_index_buffer(self.index_buffer.slice(..), wgpu::IndexFormat::Uint16);
        pass.draw_indexed(0..NUM_INDICES, 0, 0..self.instance_count);
    }

    /// Composite the fullscreen outline overlay onto
    /// `targets.render_view`. Caller gates on
    /// `selected_ids.is_empty() && hovered_id == 0` (we don't waste a
    /// fullscreen blit on idle frames).
    pub(crate) fn encode_outline_pass(
        &self,
        encoder: &mut wgpu::CommandEncoder,
        targets: &RenderTargets,
        dyn_bgs: &DynamicBindGroups,
    ) {
        let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("Outline"),
            color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                view: &targets.render_view,
                resolve_target: None,
                depth_slice: None,
                ops: wgpu::Operations {
                    load: wgpu::LoadOp::Load,
                    store: wgpu::StoreOp::Store,
                },
            })],
            depth_stencil_attachment: None,
            ..Default::default()
        });
        pass.set_pipeline(&self.pipes.outline);
        pass.set_bind_group(0, &dyn_bgs.outline, &[]);
        pass.draw(0..3, 0..1);
    }
}
