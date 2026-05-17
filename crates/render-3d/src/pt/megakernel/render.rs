//! See parent `super` for module-level imports.

use super::*;

pub(crate) fn render_path_traced(
    renderer: &mut Renderer3D,
    instances: &[geometry::CubeInstance],
    camera: &OrbitCamera,
    opts: &Render3DOptions,
    width: u32,
    height: u32,
) -> Vec<u8> {
    let pt_start = std::time::Instant::now();
    use pt_core::build::build_instance_bvh;
    use pt_core::gpu_data::{
        build_gpu_data_from_nodes, build_instance_gpu_data, GpuInstanceSceneData,
    };
    use pt_megakernel::{PathTraceCompute, PtCameraUniform};

    // Lazy init path tracer
    let surface_format = wgpu::TextureFormat::Rgba8Unorm;
    if renderer.pt.path_tracer.is_none() {
        let mut pt = PathTraceCompute::new(
            &renderer.ctx.device,
            &renderer.ctx.queue,
            width,
            height,
            surface_format,
        );
        // Forward env map to path tracer
        if opts.env_map_enabled {
            pt.set_environment_texture(
                &renderer.ctx.device,
                &renderer.ctx.queue,
                &renderer.env.texture,
                opts.env_map_intensity,
                true,
            );
            pt.set_environment_cdfs(
                &renderer.ctx.device,
                &renderer.env.marginal_cdf_data,
                &renderer.env.conditional_cdf_data,
                renderer.env.width,
                renderer.env.height,
            );
        }
        renderer.pt.path_tracer = Some(pt);
        renderer.pt.pt_scene_dirty = true;
        renderer.pt.pt_env_dirty = false;
    }

    let pt = renderer.pt.path_tracer.as_mut().unwrap();
    pt.resize(&renderer.ctx.device, width, height);
    pt.samples = opts.pt_samples;
    pt.set_emissive_sampling(
        &renderer.ctx.queue,
        opts.pt_emissive_sampling,
        opts.pt_emissive_samples,
        opts.pt_emissive_min_weight,
    );
    pt.set_bvh_config(opts.pt_gpu_bvh, opts.pt_bvh_refit);
    if renderer.pt.pt_accum_reset {
        pt.reset_accumulation();
        renderer.pt.pt_accum_reset = false;
    }

    // Auto-SPP/camera snap throttling (freeze camera/scene between target frames)
    let snap_enabled = opts.pt_auto_spp || opts.pt_camera_snap;
    let snap_interval = if snap_enabled {
        1.0 / opts.pt_target_fps.max(1.0)
    } else {
        0.0
    };
    let elapsed = if snap_enabled {
        renderer.pt.pt_camera_snap_time.elapsed().as_secs_f32()
    } else {
        0.0
    };
    let allow_update = !snap_enabled
        || elapsed >= snap_interval
        || !renderer.pt.pt_snap_valid
        || pt.frame_count == 0;

    if allow_update && snap_enabled {
        renderer.pt.pt_camera_snap_time = std::time::Instant::now();
        renderer.pt.pt_snap_valid = true;
        renderer.pt.pt_snap_anim_time = opts.animation_time;
    }

    // Upload scene: rebuild when dirty or actively animating
    let anim_active = opts.animate && opts.hash_effect != HashTransformEffect::None;
    let animated = anim_active && allow_update;
    let mut reset_by_scene = false;
    let mut reset_by_cam = false;
    let mut reset_by_slice = false;
    let mut reset_by_anim = false;
    if allow_update && (renderer.pt.pt_scene_dirty || animated) && !instances.is_empty() {
        let scene_start = std::time::Instant::now();
        let (materials_arc, pt_instances) =
            crate::renderer3d::material_cache::prepare_pt_expanded_materials(
                &opts.material_library,
                &mut renderer.mat_cache,
                &renderer.picking,
                &mut renderer.pt_expand_cache,
                instances,
                opts,
            );
        let materials = materials_arc.as_ref();

        // Build BVH: GPU-accelerated or CPU fallback based on config
        let bvh_start = std::time::Instant::now();
        debug!(
            "PT BVH start: gpu={}, instances={}",
            opts.pt_gpu_bvh,
            pt_instances.len()
        );
        let gpu_data = if opts.pt_gpu_bvh && opts.pt_bvh_refit {
            let data = GpuInstanceSceneData {
                nodes: Vec::new(),
                instances: Vec::new(),
                materials: materials.clone(),
            };
            pt.upload_scene_smart(
                &renderer.ctx.device,
                &renderer.ctx.queue,
                &pt_instances,
                &data,
                animated,
            );
            reset_by_scene = true;
            pt.reset_accumulation();
            None
        } else if opts.pt_gpu_bvh {
            let (nodes, sorted_indices) =
                pt.build_bvh(&renderer.ctx.device, &renderer.ctx.queue, &pt_instances);
            debug!(
                "PT BVH done (GPU): nodes={}, sorted_indices={}",
                nodes.len(),
                sorted_indices.len()
            );
            Some(build_gpu_data_from_nodes(
                nodes,
                &sorted_indices,
                &pt_instances,
                materials,
            ))
        } else {
            let bvh = build_instance_bvh(&pt_instances);
            debug!(
                "PT BVH done (CPU): nodes={}, sorted_indices={}",
                bvh.nodes.len(),
                bvh.tri_indices.len()
            );
            Some(build_instance_gpu_data(&bvh, &pt_instances, materials))
        };
        let bvh_ms = bvh_start.elapsed().as_secs_f64() * 1000.0;
        debug!(
            "  bvh_build: {:.2}ms ({})",
            bvh_ms,
            if opts.pt_gpu_bvh { "GPU" } else { "CPU" }
        );

        let upload_start = std::time::Instant::now();
        if let Some(gpu_data) = gpu_data {
            debug!(
                "PT upload: nodes={}, instances={}, materials={}",
                gpu_data.nodes.len(),
                gpu_data.instances.len(),
                gpu_data.materials.len()
            );
            pt.upload_scene(
                &renderer.ctx.device,
                &renderer.ctx.queue,
                &gpu_data,
                Some(&pt_instances),
            );
            reset_by_scene = true;
            pt.reset_accumulation();
        }
        renderer.pt.pt_scene_dirty = false;
        let upload_ms = upload_start.elapsed().as_secs_f64() * 1000.0;
        debug!("  scene_upload: {:.2}ms", upload_ms);

        let scene_ms = scene_start.elapsed().as_secs_f64() * 1000.0;
        debug!(
            "collect_cubes: {:.2}ms ({} cubes)",
            scene_ms,
            pt_instances.len()
        );
    }

    // Build camera uniform
    let aspect = width as f32 / height as f32;
    let view = camera.view_matrix();
    let proj = camera.projection_matrix(aspect);
    let mut inv_view = view.inverse();
    let mut inv_proj = proj.inverse();
    let mut pos = camera.position();
    let mut view_for_vp = view;
    let mut proj_for_vp = proj;

    if snap_enabled {
        if allow_update {
            renderer.pt.pt_snap_inv_view = inv_view;
            renderer.pt.pt_snap_inv_proj = inv_proj;
            renderer.pt.pt_snap_pos = pos;
            view_for_vp = view;
            proj_for_vp = proj;
        } else {
            inv_view = renderer.pt.pt_snap_inv_view;
            inv_proj = renderer.pt.pt_snap_inv_proj;
            pos = renderer.pt.pt_snap_pos;
            view_for_vp = inv_view.inverse();
            proj_for_vp = inv_proj.inverse();
        }
    }

    let cam_uniform = PtCameraUniform {
        inv_view: inv_view.to_cols_array_2d(),
        inv_proj: inv_proj.to_cols_array_2d(),
        position: pos.to_array(),
        _pad0: 0,
        frame_count: pt.frame_count + 1,
        max_bounces: opts.pt_max_bounces,
        max_transmission_depth: opts.pt_max_transmission_depth,
        dof_enabled: if opts.pt_dof_enabled { 1 } else { 0 },
        aperture: opts.effective_aperture(),
        focus_distance: opts.effective_focus_distance(),
        _pad1: [0; 2],
        slice_enabled: if opts.slice_enabled { 1.0 } else { 0.0 },
        slice_position: compute_slice_position(opts),
        slice_invert: if opts.slice_invert { 1.0 } else { 0.0 },
        _pad2: 0.0,
        slice_normal: compute_slice_normal(opts),
        _pad3: 0.0,
        spectral_mode: opts.pt_spectral_mode as u32,
        spectral_samples: opts.pt_spectral_samples.max(1),
        spectral_dispersion: if opts.pt_spectral_dispersion { 1 } else { 0 },
        sampler_mode: opts.pt_sampler_mode as u32,
    };
    // Update ReSTIR gbuffer camera matrices and detect camera movement.
    let cam_pos = pos.to_array();
    let cam_vp = (proj_for_vp * view_for_vp).to_cols_array_2d();
    let prev_vp = pt.last_view_proj.unwrap_or(cam_vp);
    pt.update_view_proj(&renderer.ctx.queue, prev_vp, cam_vp);
    let cam_jump = pt.last_camera_pos.is_some_and(|p| {
        let prev = glam::Vec3::from(p);
        let delta = (prev - pos).length();
        let base = prev.length().max(1.0);
        delta / base > 0.25
    });
    let vp_jump = pt.last_view_proj.is_some_and(|vp| {
        let mut max_diff = 0.0f32;
        for r in 0..4 {
            for c in 0..4 {
                max_diff = max_diff.max((vp[r][c] - cam_vp[r][c]).abs());
            }
        }
        max_diff > 0.05
    });
    if cam_jump || vp_jump {
        pt.mark_history_dirty();
    }
    let cam_moved = (pt.last_camera_pos != Some(cam_pos)) || (pt.last_view_proj != Some(cam_vp));
    // Always roll prev_view_proj forward (independent of cam_moved) so a
    // static camera after motion has a coherent prev/curr pair, and ReSTIR
    // temporal can correctly see "this pixel didn't move" rather than a
    // stale matrix from earlier in the session.
    pt.prev_view_proj = pt.last_view_proj;
    pt.last_view_proj = Some(cam_vp);
    if cam_moved {
        pt.mark_history_dirty();
        reset_by_cam = true;
        pt.reset_accumulation();
        pt.last_camera_pos = Some(cam_pos);
    }

    // Check slice plane changes
    let slice_params = (
        opts.slice_enabled,
        compute_slice_normal(opts),
        compute_slice_position(opts),
        opts.slice_invert,
    );
    let slice_changed = pt.last_slice_params.is_some_and(|p| p != slice_params);
    if slice_changed {
        reset_by_slice = true;
        pt.reset_accumulation();
    }
    pt.last_slice_params = Some(slice_params);

    if opts.env_map_enabled && renderer.pt.pt_env_dirty {
        pt.set_environment_texture(
            &renderer.ctx.device,
            &renderer.ctx.queue,
            &renderer.env.texture,
            opts.env_map_intensity,
            true,
        );
        pt.set_environment_cdfs(
            &renderer.ctx.device,
            &renderer.env.marginal_cdf_data,
            &renderer.env.conditional_cdf_data,
            renderer.env.width,
            renderer.env.height,
        );
        renderer.pt.pt_env_dirty = false;
    }

    let anim_time_used = if snap_enabled {
        if allow_update {
            opts.animation_time
        } else {
            renderer.pt.pt_snap_anim_time
        }
    } else {
        opts.animation_time
    };
    // Env time is accumulated independently in render_loop so the sky
    // keeps rolling even when object animation is paused.
    let _ = anim_time_used;
    let env_time = opts.env_time;
    if anim_active && allow_update {
        pt.mark_history_dirty();
        reset_by_anim = true;
        pt.reset_accumulation();
    }
    if opts.animate && allow_update {
        renderer.pt.pt_anim_log_frame = renderer.pt.pt_anim_log_frame.wrapping_add(1);
        if renderer.pt.pt_anim_log_frame.is_multiple_of(30) {
            let dt = anim_time_used - renderer.pt.pt_last_anim_time;
            info!(
                    "PT animate: t={:.3} dt={:.3} frame_count={} reset[scene={},cam={},slice={},anim={}] scene_dirty={} env_dirty={} auto_spp={} spp_update={} wavefront={}",
                    anim_time_used,
                    dt,
                    pt.frame_count,
                    reset_by_scene,
                    reset_by_cam,
                    reset_by_slice,
                    reset_by_anim,
                    renderer.pt.pt_scene_dirty,
                    renderer.pt.pt_env_dirty,
                    opts.pt_auto_spp,
                    renderer.pt.pt_samples_per_update,
                    opts.pt_wavefront
                );
        }
    }
    if allow_update {
        renderer.pt.pt_last_anim_time = anim_time_used;
    }
    pt.update_camera(&renderer.ctx.queue, &cam_uniform);
    pt.set_restir_enabled(&renderer.ctx.device, opts.pt_restir_di, opts.pt_restir_gi);
    pt.set_restir_options(
        opts.pt_restir_temporal,
        opts.pt_restir_spatial,
        opts.pt_restir_m_max,
    );
    pt.set_adaptive_enabled(
        &renderer.ctx.device,
        &renderer.ctx.queue,
        opts.pt_adaptive_sampling,
    );
    // Adaptive per-pixel SPP range derives from the single global samples
    // knob — one slider drives all sampling budgets.
    let derived_min = (opts.pt_samples / 16).max(8);
    let derived_max = opts.pt_samples.max(derived_min);
    pt.set_adaptive_config(
        &renderer.ctx.queue,
        derived_min,
        derived_max,
        opts.pt_adaptive_variance,
        opts.pt_adaptive_interval,
    );
    pt.set_pathguide_enabled(&renderer.ctx.device, opts.pt_path_guiding);
    pt.set_wavefront_rr_enabled(opts.pt_russian_roulette);
    pt.set_spectral_options(
        opts.pt_spectral_mode as u32,
        opts.pt_spectral_samples,
        if opts.pt_spectral_dispersion { 1 } else { 0 },
    );
    pt.update_env_params(
        &renderer.ctx.queue,
        opts.env_map_intensity,
        opts.env_map_rotation,
        opts.env_map_enabled,
        opts.pt_env_importance_sampling && renderer.env.width > 1 && renderer.env.height > 1,
        renderer.env.width,
        renderer.env.height,
        1.0,
        env_time,
    );

    // Dispatch compute shader + blit to render texture
    let frame_start = std::time::Instant::now();
    let mut encoder = renderer
        .ctx
        .device
        .create_command_encoder(&wgpu::CommandEncoderDescriptor {
            label: Some("PT Encoder"),
        });
    if !opts.pt_auto_spp {
        renderer.pt.pt_samples_per_update = opts.pt_samples_per_update.max(1);
    }
    let mut samples_this_frame = if opts.pt_auto_spp {
        renderer.pt.pt_samples_per_update.max(1)
    } else {
        opts.pt_samples_per_update.max(1)
    };
    if opts.pt_auto_spp {
        let target_ms = 1000.0 / opts.pt_target_fps.max(1.0);
        if renderer.pt.pt_last_render_ms < target_ms * 0.85 {
            samples_this_frame = (samples_this_frame + 1).min(64);
        } else if renderer.pt.pt_last_render_ms > target_ms * 1.15 {
            samples_this_frame = samples_this_frame.saturating_sub(1).max(1);
        }
        renderer.pt.pt_samples_per_update = samples_this_frame;
    }
    let dispatch_start = std::time::Instant::now();
    // Use wavefront or megakernel dispatch
    let mut actual_samples_this_frame = 0u32;
    if opts.pt_wavefront {
        pt.set_wavefront_enabled(&renderer.ctx.device, true);
        for _ in 0..samples_this_frame {
            if !pt.dispatch_wavefront(
                &renderer.ctx.device,
                &mut encoder,
                &renderer.ctx.queue,
                opts.pt_max_bounces,
                env_time,
            ) {
                break;
            }
            actual_samples_this_frame += 1;
        }
    } else {
        for _ in 0..samples_this_frame {
            if !pt.dispatch(&mut encoder, &renderer.ctx.queue) {
                break;
            }
            actual_samples_this_frame += 1;
        }
        pt.update_adaptive_sample_map(&mut encoder, &renderer.ctx.queue);
    }
    let dispatch_ms = dispatch_start.elapsed().as_secs_f64() * 1000.0;
    debug!(
        "  dispatch: {:.2}ms ({} samples)",
        dispatch_ms, actual_samples_this_frame
    );

    let blit_start = std::time::Instant::now();
    let state = renderer
        .render_state
        .as_ref()
        .expect("render_state not built — call ensure_render_targets before render");
    let targets = &state.targets;
    // Denoising (OIDN) is now invoked from the app layer, after PT output is
    // sample-normalized — see `pt-denoise-oidn`. The raw blit always uses
    // the PT accumulator; if OIDN is active, the app blits its result texture
    // separately on top.
    pt.set_blit_exposure(&renderer.ctx.queue, opts.effective_exposure_multiplier());
    pt.blit(&mut encoder, &targets.render_view);
    let blit_ms = blit_start.elapsed().as_secs_f64() * 1000.0;
    debug!("  blit: {:.2}ms", blit_ms);

    let output_buffer = gpu::readback_texture(
        &renderer.ctx,
        &mut encoder,
        &targets.render_texture,
        width,
        height,
    );

    let submit_start = std::time::Instant::now();
    renderer.ctx.queue.submit(std::iter::once(encoder.finish()));
    let submit_ms = submit_start.elapsed().as_secs_f64() * 1000.0;
    debug!("  submit: {:.2}ms", submit_ms);

    renderer.pt.pt_last_render_ms = frame_start.elapsed().as_secs_f32() * 1000.0;

    let readback_start = std::time::Instant::now();
    let result = gpu::map_readback(&renderer.ctx, &output_buffer, width, height);
    let readback_ms = readback_start.elapsed().as_secs_f64() * 1000.0;
    debug!("  readback: {:.2}ms ({}x{})", readback_ms, width, height);

    let total_ms = pt_start.elapsed().as_secs_f64() * 1000.0;
    info!(
        "PT render: {:.2}ms ({} cubes, {} samples, frame {})",
        total_ms,
        instances.len(),
        actual_samples_this_frame,
        pt.frame_count
    );
    trace!(
        "  scene_dirty: {}, env_dirty: {}",
        renderer.pt.pt_scene_dirty,
        renderer.pt.pt_env_dirty
    );

    result
}
