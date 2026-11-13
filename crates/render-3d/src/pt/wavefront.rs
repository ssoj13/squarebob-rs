//! CUDA SIMT path-tracing backend (`pt-cuda` when compiled with `--features cuda`),
//! enforcing strict no-silent-fallback for `PtComputeBackend::Cuda` and falling back
//! cleanly to `pt-megakernel` WGPU compute only when `PtComputeBackend::Auto` is selected.

use crate::{Renderer3D, geometry};
use render_shared::{OrbitCamera, Render3DOptions, SpectralMode};

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
) -> &'a wgpu::Texture {
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
    &slot.as_ref().expect("just initialized").0
}

#[cfg(feature = "cuda")]
pub(crate) fn ensure_cuda_aov_buffer<'a>(
    device: &wgpu::Device,
    slot: &'a mut Option<(wgpu::Buffer, usize)>,
    pixels: usize,
    label: &'static str,
) -> &'a wgpu::Buffer {
    let byte_len = pixels.max(1).saturating_mul(16);
    let recreate = match slot {
        Some((_, len)) => *len != byte_len,
        None => true,
    };
    if recreate {
        let buf = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some(label),
            size: byte_len as u64,
            usage: wgpu::BufferUsages::STORAGE
                | wgpu::BufferUsages::COPY_DST
                | wgpu::BufferUsages::COPY_SRC,
            mapped_at_creation: false,
        });
        *slot = Some((buf, byte_len));
    }
    &slot.as_ref().expect("just initialized").0
}

#[cfg(feature = "cuda")]
fn render_cuda_frame(
    renderer: &mut Renderer3D,
    instances: &[geometry::CubeInstance],
    camera: &OrbitCamera,
    opts: &Render3DOptions,
    (width, height): (u32, u32),
    need_cpu_readback: bool,
    strict_cuda: bool,
) -> Result<Option<Vec<u8>>, render_core::ReadbackError> {
    use crate::{compute_slice_normal, compute_slice_position};
    use render_core::gpu;
    use render_shared::HashTransformEffect;

    if strict_cuda && opts.pt_spectral_mode != SpectralMode::Off {
        return Err(render_core::ReadbackError::SceneBuild(
            "CUDA path-tracing backend does not support spectral rendering; set Spectral to Off or select Auto/WGPU".into(),
        ));
    }

    if renderer.pt.cuda_tracer.is_none() {
        let budget = Some(renderer.ctx.work_context.memory().clone());
        if strict_cuda {
            if !pt_cuda::CudaPathTracer::is_available() {
                return Err(render_core::ReadbackError::SceneBuild(
                    "CUDA path-tracing backend was explicitly requested, but no compatible CUDA device/driver/PTX module is available".into(),
                ));
            }
            let tracer =
                pt_cuda::CudaPathTracer::new(0, width, height, budget).map_err(cuda_err)?;
            log::info!(
                "PT CUDA backend initialized on {} ({}x{})",
                tracer.device_name(),
                width,
                height
            );
            renderer.pt.cuda_tracer = Some(tracer);
            renderer.pt.cuda_scene_dirty = true;
            renderer.pt.cuda_transforms_dirty = false;
            renderer.pt.cuda_tlas_order.clear();
            renderer.pt.cuda_env_dirty = true;
            renderer.pt.cuda_last_accum_key = None;
            renderer.pt.cuda_guides_synced = false;
            renderer.pt.cuda_init_failed = false;
        } else if !renderer.pt.cuda_init_failed {
            if !pt_cuda::CudaPathTracer::is_available() {
                log::info!("PT Auto backend: CUDA unavailable; falling back to WGPU megakernel");
                renderer.pt.cuda_init_failed = true;
            } else {
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
                        renderer.pt.cuda_transforms_dirty = false;
                        renderer.pt.cuda_tlas_order.clear();
                        renderer.pt.cuda_env_dirty = true;
                        renderer.pt.cuda_last_accum_key = None;
                        renderer.pt.cuda_guides_synced = false;
                    }
                    Err(e) => {
                        log::warn!(
                            "PT CUDA backend unavailable ({e}); falling back to WGPU megakernel"
                        );
                        renderer.pt.cuda_init_failed = true;
                    }
                }
            }
        }
    }

    if renderer.pt.cuda_tracer.is_none() {
        debug_assert!(!strict_cuda, "strict CUDA must return Err above");
        let mut local_opts = opts.clone();
        local_opts.pt_compute_backend = render_settings::PtComputeBackend::Wgpu;
        local_opts.pt_wavefront = false;
        return if need_cpu_readback {
            super::megakernel::render_path_traced(
                renderer,
                instances,
                camera,
                &local_opts,
                width,
                height,
            )
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

    let frame_start = std::time::Instant::now();
    let surface_format = render_core::DISPLAY_TEXTURE_FORMAT;
    if renderer.pt.blit.is_none() {
        renderer.pt.blit = Some(crate::pt::blit::PtDisplayBlit::new_with_adapter(
            &renderer.ctx.device,
            &renderer.ctx.queue,
            &renderer.adapter_info,
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

    renderer.pt.active_backend = render_settings::PtComputeBackend::Cuda;
    renderer.pt.pt_target_samples = opts.pt_samples.max(1);

    let cuda = renderer
        .pt
        .cuda_tracer
        .as_mut()
        .expect("checked Some above");
    if (cuda.width(), cuda.height()) != (width, height) {
        cuda.resize(width, height).map_err(cuda_err)?;
        renderer.pt.cuda_guides_synced = false;
    }
    if renderer.pt.cuda_oidn_mode != opts.pt_oidn_mode {
        renderer.pt.cuda_oidn_mode = opts.pt_oidn_mode;
        renderer.pt.cuda_guides_synced = false;
    }
    let (need_albedo, need_normal) = opts.pt_oidn_mode.requires_aov();
    if !need_albedo {
        renderer.pt.cuda_albedo_buf = None;
        renderer.pt.cuda_staging_albedo.clear();
    }
    if !need_normal {
        renderer.pt.cuda_normal_buf = None;
        renderer.pt.cuda_staging_normal.clear();
    }
    cuda.set_emissive_sampling(opts.pt_emissive_sampling)
        .map_err(cuda_err)?;

    let anim_active = opts.animate && opts.hash_effect != HashTransformEffect::None;
    if (renderer.pt.cuda_scene_dirty || renderer.pt.cuda_transforms_dirty || anim_active)
        && !instances.is_empty()
    {
        let (materials_arc, materials_ext_arc, pt_instances, materials_key) =
            crate::renderer3d::material_cache::prepare_pt_expanded_materials(
                &opts.material_library,
                &mut renderer.mat_cache,
                &renderer.picking,
                &mut renderer.pt_expand_cache,
                instances,
                opts,
            );
        let can_refit = !renderer.pt.cuda_scene_dirty
            && opts.pt_bvh_refit
            && renderer.pt.cuda_tlas_order.len() == pt_instances.len()
            && renderer.pt.cuda_tlas_materials_key == materials_key
            && !pt_instances.is_empty();
        let mut refit_ok = false;
        if can_refit {
            renderer.pt.cuda_refit_instances.clear();
            renderer.pt.cuda_refit_instances.reserve(pt_instances.len());
            for &orig_idx in &renderer.pt.cuda_tlas_order {
                let mut g = pt_instances[orig_idx].to_gpu();
                g.blas_root = pt_core::ANALYTIC_UNIT_CUBE_BLAS_ROOT;
                g.tri_base = 0;
                renderer.pt.cuda_refit_instances.push(g);
            }
            match cuda.refit_two_level_instances_gpu(&renderer.pt.cuda_refit_instances) {
                Ok(()) => refit_ok = true,
                Err(err) => {
                    log::debug!("PT CUDA TLAS refit fell back to full upload: {err}");
                }
            }
        }
        if !refit_ok {
            let tlas = pt_core::build_instance_bvh(&pt_instances);
            renderer.pt.cuda_tlas_order.clone_from(&tlas.tri_indices);
            let mut tlas_data =
                pt_core::build_instance_gpu_data(&tlas, &pt_instances, materials_arc.as_ref());
            for inst in &mut tlas_data.instances {
                inst.blas_root = pt_core::ANALYTIC_UNIT_CUBE_BLAS_ROOT;
                inst.tri_base = 0;
            }
            let two_level = pt_core::TwoLevelGpuData {
                tlas_nodes: tlas_data.nodes,
                blas_nodes: vec![pt_core::BvhNode {
                    aabb_min: [-0.5, -0.5, -0.5],
                    left_or_first: 0,
                    aabb_max: [0.5, 0.5, 0.5],
                    count: 0,
                }],
                triangles: Vec::new(),
                instances: tlas_data.instances,
                materials: tlas_data.materials,
            };
            cuda.upload_two_level_data(
                &two_level,
                bytemuck::cast_slice(materials_ext_arc.as_slice()),
            )
            .map_err(cuda_err)?;
            renderer.pt.cuda_tlas_materials_key = materials_key;
        }
        renderer.pt.cuda_scene_dirty = false;
        renderer.pt.cuda_transforms_dirty = false;
        renderer.pt.cuda_guides_synced = false;
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
        renderer.pt.cuda_guides_synced = false;
    }

    let aspect = width as f32 / height.max(1) as f32;
    let view = camera.view_matrix();
    let proj = camera.projection_matrix(aspect);
    let inv_view = view.inverse().to_cols_array_2d();
    let inv_proj = proj.inverse().to_cols_array_2d();
    let slice_n = compute_slice_normal(opts);
    let slice_p = compute_slice_position(opts);
    renderer.pt.cuda_last_slice = (opts.slice_enabled, opts.slice_invert, slice_p, slice_n);

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
    if !opts.pt_auto_spp {
        renderer.pt.pt_samples_per_update = opts.pt_samples_per_update.max(1);
    }
    let mut spp_per_update = if opts.pt_auto_spp {
        renderer.pt.pt_samples_per_update.max(1)
    } else {
        opts.pt_samples_per_update.max(1)
    };
    if opts.pt_auto_spp {
        let target_ms = 1000.0 / opts.pt_target_fps.max(1.0);
        if renderer.pt.pt_last_render_ms < target_ms * 0.85 {
            spp_per_update = (spp_per_update + 1).min(64);
        } else if renderer.pt.pt_last_render_ms > target_ms * 1.15 {
            spp_per_update = spp_per_update.saturating_sub(1).max(1);
        }
        renderer.pt.pt_samples_per_update = spp_per_update;
    }

    let cfg = pt_cuda::CudaLaunchConfig {
        width,
        height,
        sample_origin: 0,
        samples_per_launch: spp_per_update,
        max_bounces: opts.pt_max_bounces,
        max_transmission_depth: opts.pt_max_transmission_depth,
        // Same budget as the WGSL megakernel: equal to `max_bounces` disables truncation.
        max_diffuse_bounces: opts.pt_max_bounces,
        sampler_mode: opts.pt_sampler_mode.as_u32(),
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

    let accum_key = crate::CudaAccumKey::from_config(&cfg, opts.pt_emissive_sampling);
    if renderer.pt.cuda_accum_reset || renderer.pt.cuda_last_accum_key != Some(accum_key) {
        cuda.reset_accumulation().map_err(cuda_err)?;
        renderer.pt.cuda_accum_reset = false;
        renderer.pt.cuda_guides_synced = false;
        renderer.pt.cuda_last_accum_key = Some(accum_key);
    }

    let target_spp = opts.pt_samples.max(1);
    if cuda.frame_count() < target_spp && !cuda.is_converged() {
        let remaining = target_spp.saturating_sub(cuda.frame_count());
        let mut step_cfg = cfg;
        step_cfg.samples_per_launch = spp_per_update.min(remaining).max(1);
        cuda.dispatch(&step_cfg).map_err(cuda_err)?;
        renderer.pt.cuda_guides_synced = false;
    }

    let pixels = (width.max(1) as usize).saturating_mul(height.max(1) as usize);
    if renderer.pt.cuda_staging_linear.len() != pixels {
        renderer.pt.cuda_staging_linear.resize(pixels, [0.0; 4]);
    }
    cuda.read_linear_into(&mut renderer.pt.cuda_staging_linear)
        .map_err(cuda_err)?;

    let output_tex = ensure_cuda_output_texture(
        &renderer.ctx.device,
        &mut renderer.pt.cuda_output_tex,
        width,
        height,
    )
    .clone();
    renderer.ctx.queue.write_texture(
        wgpu::TexelCopyTextureInfo {
            texture: &output_tex,
            mip_level: 0,
            origin: wgpu::Origin3d::ZERO,
            aspect: wgpu::TextureAspect::All,
        },
        bytemuck::cast_slice(&renderer.pt.cuda_staging_linear),
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

    let mut encoder = renderer
        .ctx
        .device
        .create_command_encoder(&wgpu::CommandEncoderDescriptor {
            label: Some("PT CUDA Blit Encoder"),
        });
    renderer.encode_pt_display_pass(&mut encoder, &output_tex, opts)?;

    if need_cpu_readback {
        let state = renderer
            .render_state
            .as_ref()
            .ok_or(render_core::ReadbackError::MissingTarget)?;
        gpu::readback_texture(
            &renderer.ctx,
            &mut encoder,
            &state.targets.render_texture,
            width,
            height,
            &mut renderer.readback,
        )?;
    }
    renderer.ctx.queue.submit(std::iter::once(encoder.finish()));
    renderer.pt.pt_last_render_ms = frame_start.elapsed().as_secs_f32() * 1000.0;

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
    strict_cuda: bool,
) -> Result<(), render_core::ReadbackError> {
    #[cfg(feature = "cuda")]
    {
        render_cuda_frame(
            renderer,
            instances,
            camera,
            opts,
            (width, height),
            false,
            strict_cuda,
        )
        .map(|_| ())
    }
    #[cfg(not(feature = "cuda"))]
    {
        if strict_cuda {
            return Err(render_core::ReadbackError::SceneBuild(
                "CUDA path-tracing backend was explicitly requested, but render-3d was compiled without the `cuda` feature".into(),
            ));
        }
        if opts.pt_spectral_mode != SpectralMode::Off {
            super::spectral::render_path_traced_no_readback(
                renderer, instances, camera, opts, width, height,
            )
        } else {
            renderer.render_path_traced_no_readback(instances, camera, opts, width, height)
        }
    }
}

pub fn render_path_traced(
    renderer: &mut Renderer3D,
    instances: &[geometry::CubeInstance],
    camera: &OrbitCamera,
    opts: &Render3DOptions,
    width: u32,
    height: u32,
    strict_cuda: bool,
) -> Result<Vec<u8>, render_core::ReadbackError> {
    #[cfg(feature = "cuda")]
    {
        render_cuda_frame(
            renderer,
            instances,
            camera,
            opts,
            (width, height),
            true,
            strict_cuda,
        )?
        .ok_or(render_core::ReadbackError::MissingTarget)
    }
    #[cfg(not(feature = "cuda"))]
    {
        if strict_cuda {
            return Err(render_core::ReadbackError::SceneBuild(
                "CUDA path-tracing backend was explicitly requested, but render-3d was compiled without the `cuda` feature".into(),
            ));
        }
        if opts.pt_spectral_mode != SpectralMode::Off {
            super::spectral::render_path_traced(renderer, instances, camera, opts, width, height)
        } else {
            renderer.render_path_traced(instances, camera, opts, width, height)
        }
    }
}

pub fn frame_count(renderer: &Renderer3D) -> u32 {
    #[cfg(feature = "cuda")]
    if renderer.pt.active_backend == render_settings::PtComputeBackend::Cuda
        && let Some(cuda) = renderer.pt.cuda_tracer.as_ref()
    {
        return cuda.frame_count();
    }
    renderer.pt_frame_count_impl()
}

pub fn pick(renderer: &mut Renderer3D, origin: glam::Vec3, dir: glam::Vec3) -> Option<(u32, f32)> {
    #[cfg(feature = "cuda")]
    if renderer.pt.active_backend == render_settings::PtComputeBackend::Cuda {
        let cuda = renderer.pt.cuda_tracer.as_mut()?;
        let (slice_enabled, slice_invert, slice_position, slice_normal) =
            renderer.pt.cuda_last_slice;
        let cfg = pt_cuda::CudaLaunchConfig {
            two_level: true,
            slice_enabled,
            slice_invert,
            slice_position,
            slice_normal,
            ..pt_cuda::CudaLaunchConfig::default()
        };
        return match cuda.pick_world_ray(&cfg, origin.to_array(), dir.to_array()) {
            Ok(res) => res.hit.then_some((res.object_id, res.distance)),
            Err(err) => {
                log::warn!("CUDA PT pick failed: {err}");
                None
            }
        };
    }
    renderer.pt_pick_impl(origin, dir)
}
