//! Secondary PT backend (`pt-cuda` SIMT path tracer when compiled with `--features cuda`,
//! falling back to the `pt-megakernel` compute backend otherwise).

use crate::{Renderer3D, geometry};
use render_shared::{OrbitCamera, Render3DOptions};

#[cfg(feature = "cuda")]
fn cuda_err(err: impl std::fmt::Display) -> render_core::ReadbackError {
    render_core::ReadbackError::SceneBuild(err.to_string())
}

#[cfg(feature = "cuda")]
fn ensure_cuda_output_texture<'a>(
    device: &wgpu::Device,
    slot: &'a mut Option<(wgpu::Texture, wgpu::TextureView, u32, u32)>,
    width: u32,
    height: u32,
) -> &'a wgpu::TextureView {
    let recreate = match slot {
        Some((_, _, w, h)) => *w != width || *h != height,
        None => true,
    };
    if recreate {
        let tex = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("pt_cuda_linear_output"),
            size: wgpu::Extent3d {
                width: width.max(1),
                height: height.max(1),
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::Rgba32Float,
            usage: wgpu::TextureUsages::TEXTURE_BINDING
                | wgpu::TextureUsages::COPY_DST
                | wgpu::TextureUsages::COPY_SRC,
            view_formats: &[],
        });
        let view = tex.create_view(&wgpu::TextureViewDescriptor::default());
        *slot = Some((tex, view, width, height));
    }
    &slot.as_ref().expect("just initialized").1
}

#[cfg(feature = "cuda")]
fn render_cuda_frame(
    renderer: &mut Renderer3D,
    instances: &[geometry::CubeInstance],
    camera: &OrbitCamera,
    opts: &Render3DOptions,
    width: u32,
    height: u32,
    need_cpu_readback: bool,
) -> Result<Option<Vec<u8>>, render_core::ReadbackError> {
    use crate::{compute_slice_normal, compute_slice_position};
    use render_core::gpu;
    use render_shared::HashTransformEffect;

    if renderer.pt.cuda_tracer.is_none() && !renderer.pt.cuda_init_failed {
        let budget = Some(renderer.ctx.work_context.memory().clone());
        match pt_cuda::CudaPathTracer::new(0, width, height, budget) {
            Ok(tracer) => {
                log::info!(
                    "PT CUDA backend initialized on {} ({}x{})",
                    tracer.device_name(),
                    width,
                    height
                );
                renderer.pt.cuda_tracer = Some(tracer);
                renderer.pt.cuda_scene_dirty = true;
                renderer.pt.cuda_env_dirty = true;
                renderer.pt.cuda_last_cam = None;
            }
            Err(e) => {
                log::warn!(
                    "PT CUDA backend unavailable ({e}); falling back to wgpu megakernel"
                );
                renderer.pt.cuda_init_failed = true;
            }
        }
    }

    if renderer.pt.cuda_tracer.is_none() {
        let mut local_opts = opts.clone();
        local_opts.pt_wavefront = false;
        return if need_cpu_readback {
            super::megakernel::render_path_traced(renderer, instances, camera, &local_opts, width, height)
                .map(Some)
        } else {
            super::megakernel::render_path_traced_no_readback(
                renderer,
                instances,
                camera,
                &local_opts,
                width,
                height,
            )
            .map(|()| None)
        };
    }

    let surface_format = render_core::DISPLAY_TEXTURE_FORMAT;
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

    let cuda = renderer
        .pt
        .cuda_tracer
        .as_mut()
        .expect("checked Some above");
    if (cuda.width(), cuda.height()) != (width, height) {
        cuda.resize(width, height).map_err(cuda_err)?;
    }
    if renderer.pt.cuda_accum_reset {
        cuda.reset_accumulation().map_err(cuda_err)?;
        renderer.pt.cuda_accum_reset = false;
    }

    let anim_active = opts.animate && opts.hash_effect != HashTransformEffect::None;
    if (renderer.pt.cuda_scene_dirty || anim_active) && !instances.is_empty() {
        let (materials_arc, pt_instances) =
            crate::renderer3d::material_cache::prepare_pt_expanded_materials(
                &opts.material_library,
                &mut renderer.mat_cache,
                &renderer.picking,
                &mut renderer.pt_expand_cache,
                instances,
                opts,
            );
        cuda.upload_analytic_instances(&pt_instances, materials_arc.as_ref(), &[])
            .map_err(cuda_err)?;
        renderer.pt.cuda_scene_dirty = false;
    }

    if opts.env_map_enabled && renderer.pt.cuda_env_dirty {
        cuda.upload_environment(
            renderer.env.width,
            renderer.env.height,
            &renderer.env.texels_rgb,
            &renderer.env.marginal_cdf_data,
            &renderer.env.conditional_cdf_data,
        )
        .map_err(cuda_err)?;
        renderer.pt.cuda_env_dirty = false;
    }

    let aspect = width as f32 / height.max(1) as f32;
    let view = camera.view_matrix();
    let proj = camera.projection_matrix(aspect);
    let inv_view = view.inverse().to_cols_array_2d();
    let inv_proj = proj.inverse().to_cols_array_2d();
    let pos = camera.position().to_array();
    let slice_n = compute_slice_normal(opts);
    let slice_p = compute_slice_position(opts);
    let cam_key = crate::CudaCameraKey {
        inv_view,
        inv_proj,
        pos,
        slice_enabled: opts.slice_enabled,
        slice_normal: slice_n,
        slice_position: slice_p,
        slice_invert: opts.slice_invert,
    };
    if renderer.pt.cuda_last_cam != Some(cam_key) {
        cuda.reset_accumulation().map_err(cuda_err)?;
        renderer.pt.cuda_last_cam = Some(cam_key);
    }

    let derived_min = (opts.pt_samples / 16).max(8);
    let derived_max = opts.pt_samples.max(derived_min);
    let instance_tint_mix = if opts.mat_source == pt_mats::MaterialSource::None {
        1.0_f32
    } else {
        (1.0 - opts.materialize_mix).clamp(0.0, 1.0)
    };
    let env_use_importance = opts.pt_env_importance_sampling
        && opts.env_map_enabled
        && renderer.env.width > 1
        && renderer.env.height > 1
        && !renderer.env.marginal_cdf_data.is_empty()
        && !renderer.env.conditional_cdf_data.is_empty();
    let env_time = if opts.env_animate { opts.env_time } else { 0.0 };
    let spp_per_update = opts.pt_samples_per_update.max(1);

    let cfg = pt_cuda::CudaLaunchConfig {
        width,
        height,
        sample_origin: 0,
        samples_per_launch: spp_per_update,
        max_bounces: opts.pt_max_bounces,
        max_transmission_depth: opts.pt_max_transmission_depth,
        sampler_mode: opts.pt_sampler_mode as u32,
        two_level: true,
        dof_enabled: opts.pt_dof_enabled,
        aperture: opts.effective_aperture(),
        focus_distance: opts.effective_focus_distance(),
        global_opacity: opts.xray_alpha.clamp(0.0, 1.0),
        slice_enabled: opts.slice_enabled,
        slice_position: slice_p,
        slice_invert: opts.slice_invert,
        slice_normal: slice_n,
        env_enabled: opts.env_map_enabled,
        env_importance: env_use_importance,
        env_width: renderer.env.width,
        env_height: renderer.env.height,
        env_intensity: opts.env_map_intensity,
        env_time,
        exposure: opts.effective_exposure_multiplier(),
        adaptive_enabled: opts.pt_adaptive_sampling,
        adaptive_min_spp: derived_min,
        adaptive_max_spp: derived_max,
        adaptive_threshold: opts.pt_adaptive_variance,
        pick_x: 0.0,
        pick_y: 0.0,
        instance_tint_mix,
        inv_view,
        inv_proj,
        env_to_world: glam::Mat4::from_rotation_y(opts.env_map_rotation).to_cols_array_2d(),
    };

    if cuda.frame_count() < opts.pt_samples.max(1) {
        let remaining = opts.pt_samples.max(1).saturating_sub(cuda.frame_count());
        let mut step_cfg = cfg;
        step_cfg.samples_per_launch = spp_per_update.min(remaining).max(1);
        cuda.dispatch(&step_cfg).map_err(cuda_err)?;
    }

    let linear_rgba = cuda.read_linear_rgba32f().map_err(cuda_err)?;
    let output_view = ensure_cuda_output_texture(
        &renderer.ctx.device,
        &mut renderer.pt.cuda_output_tex,
        width,
        height,
    )
    .clone();
    if let Some((tex, _, _, _)) = renderer.pt.cuda_output_tex.as_ref() {
        renderer.ctx.queue.write_texture(
            wgpu::TexelCopyTextureInfo {
                texture: tex,
                mip_level: 0,
                origin: wgpu::Origin3d::ZERO,
                aspect: wgpu::TextureAspect::All,
            },
            bytemuck::cast_slice(&linear_rgba),
            wgpu::TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(width.max(1) * 16),
                rows_per_image: Some(height.max(1)),
            },
            wgpu::Extent3d {
                width: width.max(1),
                height: height.max(1),
                depth_or_array_layers: 1,
            },
        );
    }

    let mut encoder = renderer
        .ctx
        .device
        .create_command_encoder(&wgpu::CommandEncoderDescriptor {
            label: Some("PT CUDA Blit Encoder"),
        });
    let state = renderer
        .render_state
        .as_ref()
        .ok_or(render_core::ReadbackError::MissingTarget)?;
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

    if need_cpu_readback {
        gpu::readback_texture(
            &renderer.ctx,
            &mut encoder,
            &targets.render_texture,
            width,
            height,
            &mut renderer.readback,
        )?;
    }
    renderer.ctx.queue.submit(std::iter::once(encoder.finish()));

    if need_cpu_readback {
        let pixels = gpu::map_readback(&renderer.ctx, &renderer.readback)?;
        Ok(Some(pixels))
    } else {
        Ok(None)
    }
}

pub fn render_path_traced_no_readback(
    renderer: &mut Renderer3D,
    instances: &[geometry::CubeInstance],
    camera: &OrbitCamera,
    opts: &Render3DOptions,
    width: u32,
    height: u32,
) -> Result<(), render_core::ReadbackError> {
    #[cfg(feature = "cuda")]
    {
        render_cuda_frame(renderer, instances, camera, opts, width, height, false).map(|_| ())
    }
    #[cfg(not(feature = "cuda"))]
    {
        let mut local_opts = opts.clone();
        local_opts.pt_wavefront = true;
        renderer.render_path_traced_no_readback(instances, camera, &local_opts, width, height)
    }
}

pub fn render_path_traced(
    renderer: &mut Renderer3D,
    instances: &[geometry::CubeInstance],
    camera: &OrbitCamera,
    opts: &Render3DOptions,
    width: u32,
    height: u32,
) -> Result<Vec<u8>, render_core::ReadbackError> {
    #[cfg(feature = "cuda")]
    {
        render_cuda_frame(renderer, instances, camera, opts, width, height, true)?
            .ok_or(render_core::ReadbackError::MissingTarget)
    }
    #[cfg(not(feature = "cuda"))]
    {
        let mut local_opts = opts.clone();
        local_opts.pt_wavefront = true;
        renderer.render_path_traced(instances, camera, &local_opts, width, height)
    }
}

pub fn frame_count(renderer: &Renderer3D) -> u32 {
    #[cfg(feature = "cuda")]
    if let Some(cuda) = renderer.pt.cuda_tracer.as_ref() {
        return cuda.frame_count();
    }
    renderer.pt_frame_count_impl()
}

pub fn pick(renderer: &mut Renderer3D, origin: glam::Vec3, dir: glam::Vec3) -> Option<(u32, f32)> {
    #[cfg(feature = "cuda")]
    if let Some(cuda) = renderer.pt.cuda_tracer.as_mut() {
        let cfg = pt_cuda::CudaLaunchConfig::default();
        if let Ok(res) = cuda.pick_world_ray(&cfg, origin.to_array(), dir.to_array()) {
            return res.hit.then_some((res.object_id, res.distance));
        }
    }
    renderer.pt_pick_impl(origin, dir)
}
