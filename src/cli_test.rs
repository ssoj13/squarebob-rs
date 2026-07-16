//! Manual / diagnostic CLI tests: `squarebob-rs test <name> [...]`
//!
//! Not `cargo test`; avoids GUI and persists for on-machine checks.

use std::path::PathBuf;

pub fn run(args: &[String]) -> anyhow::Result<()> {
    let sub = args.first().map(|s| s.as_str()).unwrap_or("help");

    match sub {
        "help" | "-h" | "--help" => {
            print_help();
            Ok(())
        }
        "ping" => {
            println!("cli_test pong");
            Ok(())
        }
        "gpu-pt-smoke" => gpu_pt_smoke(),
        "ntfs-available" => {
            let path = ntfs_sample_path(args.get(1));
            println!("checking: {:?}", path.display());
            #[cfg(windows)]
            {
                use crate::scanner_ntfs;
                let ok = scanner_ntfs::is_ntfs_available(&path);
                println!("is_ntfs_available: {}", ok);
                Ok(())
            }
            #[cfg(not(windows))]
            {
                println!("is_ntfs_available: n/a (not Windows)");
                Ok(())
            }
        }
        "volume-open" => {
            let path = ntfs_sample_path(args.get(1));
            println!("opening raw volume for: {:?}", path.display());
            #[cfg(windows)]
            {
                crate::scanner_ntfs::probe_raw_volume_access(&path)?;
                println!("volume-open: OK (handle opened and closed)");
                Ok(())
            }
            #[cfg(not(windows))]
            {
                anyhow::bail!("volume-open is Windows-only")
            }
        }
        "mft-ready" => {
            #[cfg(windows)]
            {
                use crate::scanner_ntfs;
                let path = ntfs_sample_path(args.get(1));
                let max_diag = args
                    .get(2)
                    .and_then(|s| s.parse::<usize>().ok())
                    .unwrap_or(3);
                println!("path: {:?}", path.display());
                let fs_ok = scanner_ntfs::is_ntfs_available(&path);
                println!("is_ntfs_available: {}", fs_ok);
                if !fs_ok {
                    println!(
                        "MFT fast path: no (not NTFS or path has no drive letter). App will use jwalk here."
                    );
                    return Ok(());
                }
                scanner_ntfs::probe_raw_volume_access(&path)?;
                println!("volume device open: OK");
                let report = scanner_ntfs::diagnose_fsctl_enum_usn(&path, max_diag)?;
                println!("---\n{}\n---", report);
                println!(
                    "MFT_ioctl: works on this volume — you may enable Settings → Scanner → NTFS MFT."
                );
                println!(
                    "Default GUI scanner is jwalk (Standard), NOT MFT unless you change saved settings.",
                );
                Ok(())
            }
            #[cfg(not(windows))]
            {
                let _ = args;
                anyhow::bail!("mft-ready is Windows-only")
            }
        }
        "enum-diagnose" => {
            #[cfg(windows)]
            {
                let path = ntfs_sample_path(args.get(1));
                let max_lp = args
                    .get(2)
                    .and_then(|s| s.parse::<usize>().ok())
                    .unwrap_or(8);
                let report = crate::scanner_ntfs::diagnose_fsctl_enum_usn(&path, max_lp)?;
                println!("{}", report);
                Ok(())
            }
            #[cfg(not(windows))]
            {
                let _ = args;
                anyhow::bail!("enum-diagnose is Windows-only")
            }
        }
        "mft-list" => {
            #[cfg(windows)]
            {
                let path = ntfs_sample_path(args.get(1));
                let n = args
                    .get(2)
                    .and_then(|s| s.parse::<usize>().ok())
                    .unwrap_or(40);
                print!("{}", crate::scanner_ntfs::mft_dump_names(&path, n)?);
                Ok(())
            }
            #[cfg(not(windows))]
            {
                let _ = args;
                anyhow::bail!("mft-list is Windows-only")
            }
        }
        _ => anyhow::bail!(
            "unknown test {:?}; run `squarebob-rs test help` for commands",
            sub
        ),
    }
}

fn gpu_pt_smoke() -> anyhow::Result<()> {
    const WIDTH: u32 = 64;
    const HEIGHT: u32 = 64;

    let _ = env_logger::Builder::from_env(env_logger::Env::default()).try_init();
    let ctx = render_core::gpu::GpuContext::new()
        .ok_or_else(|| anyhow::anyhow!("GPU initialization failed"))?;
    let device = &ctx.device;
    let queue = &ctx.queue;
    let error_scope = device.push_error_scope(wgpu::ErrorFilter::Validation);
    let smoke_result = (|| -> anyhow::Result<(Vec<u8>, Vec<u8>)> {
        let mut tracer = pt_megakernel::PathTraceCompute::new(
            device,
            queue,
            WIDTH,
            HEIGHT,
            wgpu::TextureFormat::Rgba8Unorm,
        )?;
        tracer.samples = 4;

        let instance = pt_core::Instance::from_cube(
            glam::Mat4::from_scale(glam::Vec3::splat(1.4)),
            [0.9, 0.2, 0.1, 1.0],
            1,
            0,
        );
        let mut material = pt_core::GpuMaterial::default();
        material.emission_color_weight = glam::Vec4::new(4.0, 1.0, 0.25, 1.0);
        let instances = [instance];
        let bvh = pt_core::build_instance_bvh(&instances);
        let scene = pt_core::build_instance_gpu_data(&bvh, &instances, &[material]);
        tracer.upload_scene(device, queue, &scene, Some(&instances))?;

        let position = glam::Vec3::new(0.0, 0.0, 3.0);
        let view = glam::Mat4::look_at_rh(position, glam::Vec3::ZERO, glam::Vec3::Y);
        let projection = glam::Mat4::perspective_rh(45.0_f32.to_radians(), 1.0, 0.1, 100.0);
        tracer.update_camera(
            queue,
            &pt_megakernel::PtCameraUniform {
                inv_view: view.inverse().to_cols_array_2d(),
                inv_proj: projection.inverse().to_cols_array_2d(),
                position: position.to_array(),
                _pad0: 0,
                frame_count: 1,
                max_bounces: 4,
                max_transmission_depth: 4,
                dof_enabled: 0,
                aperture: 0.0,
                focus_distance: 3.0,
                rr_enabled: 0,
                _pad1: 0,
                slice_enabled: 0.0,
                slice_position: 0.0,
                slice_invert: 0.0,
                _pad2: 0.0,
                slice_normal: [0.0, 1.0, 0.0],
                _pad3: 0.0,
                spectral_mode: 0,
                spectral_samples: 1,
                spectral_dispersion: 0,
                sampler_mode: 0,
                materialize_mix: 0.0,
                _pad4: [0.0; 3],
            },
        );
        tracer.set_restir_enabled(device, true, true)?;
        tracer.set_adaptive_enabled(device, queue, true)?;
        tracer.set_pathguide_enabled(device, true);

        let display = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("gpu-pt-smoke display"),
            size: wgpu::Extent3d {
                width: WIDTH,
                height: HEIGHT,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::Rgba8Unorm,
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_SRC,
            view_formats: &[],
        });
        let display_view = display.create_view(&wgpu::TextureViewDescriptor::default());
        let mut raw_readback = render_core::gpu::TextureReadback::default();
        let mut display_readback = render_core::gpu::TextureReadback::default();
        let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
            label: Some("gpu-pt-smoke encoder"),
        });

        anyhow::ensure!(
            tracer.dispatch(&mut encoder, queue),
            "megakernel refused dispatch after scene upload"
        );
        tracer.set_blit_exposure(queue, 1.0);
        tracer.blit(&mut encoder, &display_view);
        render_core::gpu::readback_texture_bytes(
            &ctx,
            &mut encoder,
            tracer.output_texture(),
            WIDTH,
            HEIGHT,
            16,
            "gpu-pt-smoke raw readback",
            &mut raw_readback,
        )?;
        render_core::gpu::readback_texture(
            &ctx,
            &mut encoder,
            &display,
            WIDTH,
            HEIGHT,
            &mut display_readback,
        )?;
        queue.submit(std::iter::once(encoder.finish()));

        let raw = render_core::gpu::map_readback(&ctx, &raw_readback)?;
        let display = render_core::gpu::map_readback(&ctx, &display_readback)?;
        Ok((raw, display))
    })();

    let validation_error = pollster::block_on(error_scope.pop());
    if let Some(error) = validation_error {
        anyhow::bail!("wgpu validation failed: {error:?}");
    }
    let (raw, display) = smoke_result?;

    let mut raw_finite = 0usize;
    let mut raw_nonzero = 0usize;
    let mut raw_min = f32::INFINITY;
    let mut raw_max = f32::NEG_INFINITY;
    for bytes in raw.chunks_exact(4) {
        let value = f32::from_ne_bytes(bytes.try_into().expect("four-byte chunk"));
        if value.is_finite() {
            raw_finite += 1;
            raw_min = raw_min.min(value);
            raw_max = raw_max.max(value);
            if value.abs() > 1.0e-6 {
                raw_nonzero += 1;
            }
        }
    }

    let first = display.get(..4).unwrap_or(&[]);
    let display_nonuniform = display
        .chunks_exact(4)
        .filter(|pixel| *pixel != first)
        .count();
    let center = ((HEIGHT / 2 * WIDTH + WIDTH / 2) * 4) as usize;
    let center_pixel = display.get(center..center + 4).unwrap_or(&[]);
    println!(
        "gpu-pt-smoke raw: finite={raw_finite}/{} nonzero={raw_nonzero} range=[{raw_min:.6}, {raw_max:.6}]",
        raw.len() / 4
    );
    println!(
        "gpu-pt-smoke display: nonuniform={display_nonuniform}/{} first={first:?} center={center_pixel:?}",
        display.len() / 4
    );

    anyhow::ensure!(
        raw_finite == raw.len() / 4,
        "raw PT output contains NaN/Inf"
    );
    anyhow::ensure!(raw_nonzero > 0, "raw PT output is entirely zero");
    anyhow::ensure!(display_nonuniform > 0, "display blit is a uniform field");
    println!("gpu-pt-smoke: OK");
    Ok(())
}

fn ntfs_sample_path(extra: Option<&String>) -> PathBuf {
    if let Some(p) = extra {
        return PathBuf::from(p);
    }
    #[cfg(windows)]
    {
        PathBuf::from("C:\\")
    }
    #[cfg(not(windows))]
    {
        PathBuf::from("/")
    }
}

fn print_help() {
    eprintln!(
        r#"squarebob-rs test — diagnostic CLI harness (not `cargo test`)

USAGE:
    squarebob-rs test [NAME] [ARGS...]

TESTS:
    help               This list
    ping               Sanity check (prints pong)
    gpu-pt-smoke       Megakernel + display blit + GPU readback regression check
    ntfs-available [PATH]   Print whether `is_ntfs_available` is true (default: C:\ on Windows)
    volume-open    [PATH]   Try opening \\.\X: like MFT scanner (often needs admin)
    mft-ready [PATH] [N] IOCTL smoke + hint; N = enum-diagnose rounds (default 3)
    mft-list [PATH] [N]  First N FILE/DIR names from MFT enumeration (default 40); not full paths
    enum-diagnose [PATH] [N] Peek USN enumeration (histogram); N=max IOCTL rounds (default 8)
"#
    );
}
