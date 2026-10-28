//! Megakernel path-tracing frame execution and display blit orchestration.
//!
//! Coordinates scene/environment publication, camera/adaptive/spectral configuration,
//! sample recording/submission on `pt_megakernel::PathTraceCompute`, and final
//! presentation via `PtDisplayBlit`.

use super::*;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum PtOutput {
    GpuOnly,
    CpuReadback,
}

fn pt_err(err: impl std::fmt::Display) -> render_core::ReadbackError {
    render_core::ReadbackError::SceneBuild(err.to_string())
}

fn settle_pt(
    device: &wgpu::Device,
    pt: &mut pt_megakernel::PathTraceCompute,
) -> Result<(), render_core::ReadbackError> {
    if pt.pending() > 0 {
        device
            .poll(wgpu::PollType::wait_indefinitely())
            .map_err(render_core::ReadbackError::PollFailed)?;
    }
    pt.collect().map(|_| ()).map_err(pt_err)
}

pub(crate) fn render_path_traced(
    renderer: &mut Renderer3D,
    instances: &[geometry::CubeInstance],
    camera: &OrbitCamera,
    opts: &Render3DOptions,
    width: u32,
    height: u32,
) -> Result<Vec<u8>, render_core::ReadbackError> {
    render_path_traced_frame(
        renderer,
        instances,
        camera,
        opts,
        width,
        height,
        PtOutput::CpuReadback,
    )?
    .ok_or(render_core::ReadbackError::MissingTarget)
}

pub(super) fn render_path_traced_frame(
    renderer: &mut Renderer3D,
    instances: &[geometry::CubeInstance],
    camera: &OrbitCamera,
    opts: &Render3DOptions,
    width: u32,
    height: u32,
    output: PtOutput,
) -> Result<Option<Vec<u8>>, render_core::ReadbackError> {
    let pt_start = std::time::Instant::now();
    use pt_megakernel::{PathTraceCompute, PtCameraUniform};

    // Lazy init path tracer and host display blit
    let surface_format = render_core::DISPLAY_TEXTURE_FORMAT;
    if renderer.pt.path_tracer.is_none() {
        let mut pt = PathTraceCompute::new(
            &renderer.ctx.work_context,
            width,
            height,
            surface_format,
        )
        .map_err(pt_err)?;
        settle_pt(&renderer.ctx.device, &mut pt)?;

        // Forward env map to path tracer when enabled
        if opts.env_map_enabled {
            pt.set_environment_texture(
                pt_megakernel::EnvironmentTexture::Untracked(&renderer.env.texture),
                opts.env_map_intensity,
                true,
            )
            .map_err(pt_err)?;
            settle_pt(&renderer.ctx.device, &mut pt)?;
            pt.set_environment_cdfs(
                &renderer.env.marginal_cdf_data,
                &renderer.env.conditional_cdf_data,
                renderer.env.width,
                renderer.env.height,
            )
            .map_err(pt_err)?;
            settle_pt(&renderer.ctx.device, &mut pt)?;
            renderer.pt.pt_env_dirty = false;
        } else {
            renderer.pt.pt_env_dirty = true;
        }
        renderer.pt.path_tracer = Some(pt);
        renderer.pt.display_key = None;
        renderer.pt.pt_scene_dirty = true;
    }

    if renderer.pt.blit.is_none() {
        renderer.pt.blit = Some(crate::pt::blit::PtDisplayBlit::new(
            &renderer.ctx.device,
            &renderer.ctx.queue,
            width,
            height,
            surface_format,
        ));
        renderer.pt.display_key = None;
    }
    renderer.install_display_transform();
    if let Some(blit) = renderer.pt.blit.as_mut() {
        blit.resize(width, height);
    }

    let pt = renderer
        .pt
        .path_tracer
        .as_mut()
        .ok_or(render_core::ReadbackError::MissingTarget)?;
    pt.resize(width, height).map_err(pt_err)?;
    settle_pt(&renderer.ctx.device, pt)?;
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
        || pt.frame_count() == 0;

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

        // Build two-level acceleration structure: GPU or CPU analytic-instance packing
        let bvh_start = std::time::Instant::now();
        debug!(
            "PT BVH start: gpu={}, instances={}",
            opts.pt_gpu_bvh,
            pt_instances.len()
        );
        if opts.pt_gpu_bvh {
            pt.build_analytic_instances_gpu(&pt_instances, materials, None, None)
                .map_err(pt_err)?;
            settle_pt(&renderer.ctx.device, pt)?;
        } else {
            let two_level = pt_core::pack_analytic_instances(&pt_instances, materials);
            debug!(
                "PT BVH done (CPU two-level): tlas_nodes={}, blas_nodes={}, instances={}, materials={}",
                two_level.tlas_nodes.len(),
                two_level.blas_nodes.len(),
                two_level.instances.len(),
                two_level.materials.len()
            );
            let upload_start = std::time::Instant::now();
            pt.upload_scene_two_level(&two_level).map_err(pt_err)?;
            settle_pt(&renderer.ctx.device, pt)?;
            let upload_ms = upload_start.elapsed().as_secs_f64() * 1000.0;
            debug!("  scene_upload: {:.2}ms", upload_ms);
        }
        let bvh_ms = bvh_start.elapsed().as_secs_f64() * 1000.0;
        debug!(
            "  bvh_build: {:.2}ms ({})",
            bvh_ms,
            if opts.pt_gpu_bvh { "GPU" } else { "CPU" }
        );
        reset_by_scene = true;
        pt.reset_accumulation();
        renderer.pt.pt_scene_dirty = false;

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

    let derived_min = (opts.pt_samples / 16).max(8);
    let derived_max = opts.pt_samples.max(derived_min);
    let cam_uniform = PtCameraUniform {
        inv_view: inv_view.to_cols_array_2d(),
        inv_proj: inv_proj.to_cols_array_2d(),
        position: pos.to_array(),
        _pad0: 0,
        frame_count: 1,
        max_bounces: opts.pt_max_bounces,
        max_transmission_depth: opts.pt_max_transmission_depth,
        dof_enabled: u32::from(opts.pt_dof_enabled),
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
        spectral_samples: opts.pt_spectral_samples,
        spectral_dispersion: u32::from(opts.pt_spectral_dispersion),
        sampler_mode: opts.pt_sampler_mode as u32,
        two_level: 1,
        adaptive_enabled: u32::from(opts.pt_adaptive_sampling),
        adaptive_max_spp: derived_max,
        _pad4: 0,
    };

    // Update camera matrices and detect camera movement.
    let cam_pos = pos.to_array();
    let cam_vp = (proj_for_vp * view_for_vp).to_cols_array_2d();
    let prev_vp = pt.last_view_proj.unwrap_or(cam_vp);
    pt.update_view_proj(prev_vp, cam_vp);
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
            pt_megakernel::EnvironmentTexture::Untracked(&renderer.env.texture),
            opts.env_map_intensity,
            true,
        )
        .map_err(pt_err)?;
        settle_pt(&renderer.ctx.device, pt)?;
        pt.set_environment_cdfs(
            &renderer.env.marginal_cdf_data,
            &renderer.env.conditional_cdf_data,
            renderer.env.width,
            renderer.env.height,
        )
        .map_err(pt_err)?;
        settle_pt(&renderer.ctx.device, pt)?;
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
    let env_time = if opts.env_animate {
        opts.env_time
    } else {
        0.0
    };
    if animated && (anim_time_used - renderer.pt.pt_last_anim_time).abs() > 1e-5 {
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
                pt.frame_count(),
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

    let env_use_importance = opts.pt_env_importance_sampling
        && opts.env_map_enabled
        && renderer.env.width > 1
        && renderer.env.height > 1
        && !renderer.env.marginal_cdf_data.is_empty()
        && !renderer.env.conditional_cdf_data.is_empty();
    let env_w = if opts.env_map_enabled {
        renderer.env.width as f32
    } else {
        0.0
    };
    let env_h = if opts.env_map_enabled {
        renderer.env.height as f32
    } else {
        0.0
    };
    pt.configure(pt_megakernel::Configuration {
        camera: Some(cam_uniform),
        max_samples: Some(opts.pt_samples),
        adaptive: Some(pt_megakernel::adaptive::AdaptiveConfig {
            enabled: opts.pt_adaptive_sampling,
            min_spp: derived_min,
            max_spp: derived_max,
            variance_threshold: opts.pt_adaptive_variance,
            update_interval: opts.pt_adaptive_interval,
        }),
        spectral: Some([
            opts.pt_spectral_mode as u32,
            opts.pt_spectral_samples,
            u32::from(opts.pt_spectral_dispersion),
        ]),
        environment: Some(pt_megakernel::PtEnvUniform {
            params0: [
                opts.env_map_intensity,
                0.0,
                f32::from(opts.env_map_enabled),
                f32::from(env_use_importance),
            ],
            params1: [env_w, env_h, 1.0, env_time],
            env_to_world: glam::Mat4::from_rotation_y(opts.env_map_rotation).to_cols_array_2d(),
        }),
    })
    .map_err(pt_err)?;
    settle_pt(&renderer.ctx.device, pt)?;

    // Dispatch compute shader + blit to render texture
    let frame_start = std::time::Instant::now();
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
    let mut actual_samples_this_frame = 0u32;
    let mut remaining = samples_this_frame;
    while remaining > 0 {
        let chunk = remaining.min(pt_megakernel::MAX_SAMPLES_PER_RECORDING);
        let Some(samples) = pt.dispatch(chunk).map_err(pt_err)? else {
            break;
        };
        let n = samples.samples();
        let _submission = pt.submit(samples).map_err(pt_err)?;
        settle_pt(&renderer.ctx.device, pt)?;
        actual_samples_this_frame += n;
        remaining = remaining.saturating_sub(n);
        if n == 0 {
            break;
        }
    }
    let frame_count = pt.frame_count();
    let output_view = pt.output_texture_view_untracked().clone();
    let dispatch_ms = dispatch_start.elapsed().as_secs_f64() * 1000.0;
    debug!(
        "  dispatch: {:.2}ms ({} samples)",
        dispatch_ms, actual_samples_this_frame
    );

    let blit_start = std::time::Instant::now();
    let mut encoder = renderer
        .ctx
        .device
        .create_command_encoder(&wgpu::CommandEncoderDescriptor {
            label: Some("PT Encoder"),
        });
    let state = renderer
        .render_state
        .as_ref()
        .expect("render_state not built — call ensure_render_targets before render");
    let targets = &state.targets;
    let blit = renderer
        .pt
        .blit
        .as_mut()
        .ok_or(render_core::ReadbackError::MissingTarget)?;
    blit.set_blit_exposure(&renderer.ctx.queue, opts.effective_exposure_multiplier());
    blit.set_blit_source_encoded(&renderer.ctx.queue, false);
    let (tm_tag, ev, wb, gc) = opts.blit_color_lane();
    blit.set_blit_color(&renderer.ctx.queue, tm_tag, ev, wb, gc);
    blit.blit_texture(
        &renderer.ctx.device,
        &mut encoder,
        &targets.render_view,
        &output_view,
    );
    let blit_ms = blit_start.elapsed().as_secs_f64() * 1000.0;
    debug!("  blit: {:.2}ms", blit_ms);

    if output == PtOutput::CpuReadback {
        gpu::readback_texture(
            &renderer.ctx,
            &mut encoder,
            &targets.render_texture,
            width,
            height,
            &mut renderer.readback,
        )?;
    }

    let submit_start = std::time::Instant::now();
    renderer.ctx.queue.submit(std::iter::once(encoder.finish()));
    let submit_ms = submit_start.elapsed().as_secs_f64() * 1000.0;
    debug!("  submit: {:.2}ms", submit_ms);

    renderer.pt.pt_last_render_ms = frame_start.elapsed().as_secs_f32() * 1000.0;

    let result = if output == PtOutput::CpuReadback {
        let readback_start = std::time::Instant::now();
        let pixels = gpu::map_readback(&renderer.ctx, &renderer.readback)?;
        let readback_ms = readback_start.elapsed().as_secs_f64() * 1000.0;
        debug!("  readback: {:.2}ms ({}x{})", readback_ms, width, height);
        Some(pixels)
    } else {
        None
    };

    let total_ms = pt_start.elapsed().as_secs_f64() * 1000.0;
    info!(
        "PT render: {:.2}ms ({} cubes, {} samples, frame {})",
        total_ms,
        instances.len(),
        actual_samples_this_frame,
        frame_count
    );
    trace!(
        "  scene_dirty: {}, env_dirty: {}",
        renderer.pt.pt_scene_dirty, renderer.pt.pt_env_dirty
    );

    Ok(result)
}
