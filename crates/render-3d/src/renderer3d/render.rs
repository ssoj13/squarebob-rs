//! Per-frame 3D render entry point: `Renderer3D::render_to_view`.
//!
//! Extracted from `lib.rs` in the post-sprint-3 modularization
//! pass. `render_to_view` is still a method of `Renderer3D` —
//! `impl Renderer3D` is re-opened here.

use std::sync::Arc;

use glam::Vec3;
use log::trace;

use squarebob_core::DirEntry;
use render_shared::{HoverMode, OrbitCamera, Render3DOptions};
use treemap::TreeMapOptions;

use crate::geometry::{self, CubeInstance, NUM_INDICES};
use crate::targets::{DynamicBindGroups, RenderTargets};
use crate::{pt, Renderer3D};

impl Renderer3D {
    pub fn render_to_view(
        &mut self,
        root: &DirEntry,
        width: u32,
        height: u32,
        camera: &OrbitCamera,
        opts: &Render3DOptions,
        treemap_opts: &TreeMapOptions,
    ) {
        use log::{debug, info, warn};
        let render_start = std::time::Instant::now();
        info!(
            "=== render_to_view START: {}x{}, PT={}, wire={} ===",
            width, height, opts.path_tracing, opts.show_wireframe
        );

        // Force xray_alpha = 1.0 for PBR mode (transparency only in PT)
        let mut opts = opts.clone();
        if !opts.path_tracing {
            opts.xray_alpha = 1.0;
        }
        self.pt.pt_backend_kind = pt::backend_from_opts(&opts);

        // Freeze animation time for PT auto-SPP/camera snap between updates
        if opts.path_tracing {
            let snap_enabled = opts.pt_auto_spp || opts.pt_camera_snap;
            if snap_enabled {
                let frame_count = pt::frame_count(self.pt.pt_backend_kind, self);
                let snap_interval = 1.0 / opts.pt_target_fps.max(1.0);
                let elapsed = self.pt.pt_camera_snap_time.elapsed().as_secs_f32();
                let allow_update =
                    elapsed >= snap_interval || !self.pt.pt_snap_valid || frame_count == 0;
                if !allow_update {
                    opts.animation_time = self.pt.pt_snap_anim_time;
                    opts.animate = false;
                }
            }
        }
        let opts = &opts;

        if width == 0 || height == 0 {
            warn!("render_to_view: zero size, skipping");
            return;
        }

        self.ensure_targets(width, height);

        let (layout_w, layout_h) = self.scene_layout_size();

        // Check if we can reuse cached instances. The cache depends on the logical scene layout,
        // not the output texture size, so resizing the window only updates camera/render targets.
        let opts_hash = Self::opts_hash(opts, layout_w, layout_h, camera, height);
        let cache_valid = !opts.animate
            && self.cached_instances.is_some()
            && self.cached_opts_hash == opts_hash
            && self.cached_layout_size == (layout_w, layout_h);

        trace!("cache_valid: {}, opts_hash: 0x{:x}", cache_valid, opts_hash);

        // Own an `Arc` so we can call `pick_from_existing` (`&mut self`) without borrowing `cached_instances`.
        let instances_arc: Arc<Vec<CubeInstance>> = if cache_valid {
            Arc::clone(
                self.cached_instances
                    .as_ref()
                    .expect("cached_instances not built — collect_cubes must run before render"),
            )
        } else {
            log::info!(
                "cache MISS: animate={}, has_cache={}, hash_match={}, size_match={}",
                opts.animate,
                self.cached_instances.is_some(),
                self.cached_opts_hash == opts_hash,
                self.cached_layout_size == (layout_w, layout_h)
            );
            // Layout only on cache miss — rect values are already set when cache is valid
            treemap::layout(
                root,
                0.0,
                0.0,
                layout_w as f32,
                layout_h as f32,
                treemap_opts,
            );
            let world_center = Vec3::new(layout_w as f32 / 2.0, -(layout_h as f32 / 2.0), 0.0);
            let new_instances = self.collect_cubes(
                root,
                opts,
                treemap_opts,
                world_center,
                camera.position(),
                height as f32,
                camera.fov,
            );
            // PT scene must be rebuilt to match new object IDs in id_map
            self.pt.pt_scene_dirty = true;

            let arc = Arc::new(new_instances);
            if !opts.animate {
                self.cached_instances = Some(Arc::clone(&arc));
                self.cached_opts_hash = opts_hash;
                self.cached_layout_size = (layout_w, layout_h);
            } else {
                self.cached_instances = Some(Arc::clone(&arc));
            }
            arc
        };

        let instances: &[CubeInstance] = instances_arc.as_ref();

        self.instance_count = instances.len() as u32;
        info!(
            "render_to_view: instance_count={}, cache_valid={}",
            self.instance_count, cache_valid
        );
        if instances.is_empty() {
            warn!("render_to_view: no instances, skipping");
            return;
        }

        // Upload instances (also when buffer was reset even if cache valid)
        let need_upload = !cache_valid || self.instance_buffer.is_none();
        info!(
            "render_to_view: need_upload={}, buffer_exists={}",
            need_upload,
            self.instance_buffer.is_some()
        );
        if need_upload {
            let need_realloc =
                self.instance_buffer.is_none() || instances.len() > self.instance_buffer_capacity;
            if need_realloc {
                let new_capacity = (instances.len() * 5 / 4).max(1024);
                let new_size = new_capacity * std::mem::size_of::<CubeInstance>();
                self.instance_buffer =
                    Some(self.ctx.device.create_buffer(&wgpu::BufferDescriptor {
                        label: Some("Instance VBO"),
                        size: new_size as u64,
                        usage: wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::COPY_DST,
                        mapped_at_creation: false,
                    }));
                self.instance_buffer_capacity = new_capacity;
            }
            if let Some(ref buf) = self.instance_buffer {
                self.ctx
                    .queue
                    .write_buffer(buf, 0, bytemuck::cast_slice(instances));
            }
        }

        // Match outline/hover uniforms to the *current* cursor: read last frame's object_id buffer
        // at pending pixel before encoding (full render still ends with readback from *this* frame).
        // PT mode skips this path — it uses CPU ray pick (`pt_pick`) driven from the UI thread,
        // not the GPU readback used by PBR/wireframe.
        if !opts.path_tracing && cache_valid && self.instance_count > 0 {
            if let Some((px, py)) = self.picking.pending_pick {
                self.picking.ensure_readback(&self.ctx.device, width);
                self.pick_from_existing();
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
            );

            // Outline overlay in PT mode. Picking is handled separately on
            // the UI thread via `pt_pick` (CPU ray cast on the BVH), so
            // this block runs only when there's something to highlight —
            // no Object ID readback needed. We still do an Object ID pass
            // because the outline shader samples that texture to detect
            // silhouettes of the hovered/selected IDs.
            let has_active_overlay = !self.selected_ids.is_empty() || hovered_id != 0;
            if opts.hover_mode != HoverMode::None && has_active_overlay {
                let targets = self
                    .targets
                    .as_ref()
                    .expect("targets not built — call ensure_render_targets before render");
                let dyn_bgs = self
                    .dyn_bgs
                    .as_ref()
                    .expect("dyn_bgs not built — call ensure_render_targets before render");
                let ib = self.instance_buffer.as_ref().expect(
                    "instance_buffer not built — collect_cubes must upload before encode_passes",
                );

                let mut enc =
                    self.ctx
                        .device
                        .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                            label: Some("PT Outline Encoder"),
                        });

                self.encode_object_id_pass(&mut enc, targets, ib, opts.double_sided);
                self.encode_outline_pass(&mut enc, targets, dyn_bgs);

                self.ctx.queue.submit(std::iter::once(enc.finish()));
            }

            let total_ms = render_start.elapsed().as_secs_f64() * 1000.0;
            debug!(
                "PT render (zero-copy): {:.2}ms ({} cubes)",
                total_ms, num_cubes
            );
            return;
        }

        // Encode PBR/wireframe passes
        let targets = self
            .targets
            .as_ref()
            .expect("targets not built — call ensure_render_targets before render");
        let dyn_bgs = self
            .dyn_bgs
            .as_ref()
            .expect("dyn_bgs not built — call ensure_render_targets before render");
        info!(
            "render_to_view: calling encode_passes, targets {:?}",
            targets.size
        );
        self.encode_passes(&mut encoder, targets, dyn_bgs, opts, hovered_id);
        info!("render_to_view: encode_passes done, submitting");

        // Submit picking readback (uses pending_pick set by set_mouse_pos)
        let targets = self
            .targets
            .as_ref()
            .expect("targets not built — call ensure_render_targets before render");
        self.picking
            .submit_readback(&mut encoder, &targets.object_id_texture, targets.size);

        self.ctx.queue.submit(std::iter::once(encoder.finish()));

        // Blocking poll to read pick result (sync like alembic-rs)
        self.picking.poll_result(&self.ctx.device);

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
    ) {
        pt::megakernel::render_path_traced_no_readback(
            self, instances, camera, opts, width, height,
        );
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
