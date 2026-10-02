//! Headless diagnostic of the production OIDN bridge on frozen HDR inputs.
//! Run with: cargo run --locked --example oidn_highlight_probe
//! This isolates sample-dependent input clamping; it does not reproduce a scene.
use anyhow::{Context, Result, ensure};
use pt_denoise_oidn::{OidnDenoiser, OidnMode, Quality};
use render_core::gpu::{GpuContext, TextureReadback, map_readback, readback_texture_bytes};
use wgpu::util::DeviceExt;

const W: u32 = 96;
const H: u32 = 64;

fn capture(ctx: &GpuContext, source: &wgpu::TextureView) -> Result<Vec<f32>> {
    let texture = ctx.device.create_texture(&wgpu::TextureDescriptor {
        label: Some("OIDN diagnostic capture"),
        size: wgpu::Extent3d {
            width: W,
            height: H,
            depth_or_array_layers: 1,
        },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format: wgpu::TextureFormat::Rgba32Float,
        usage: wgpu::TextureUsages::STORAGE_BINDING | wgpu::TextureUsages::COPY_SRC,
        view_formats: &[],
    });
    let view = texture.create_view(&Default::default());
    let layout = ctx
        .device
        .create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("OIDN diagnostic copy layout"),
            entries: &[
                wgpu::BindGroupLayoutEntry {
                    binding: 0,
                    visibility: wgpu::ShaderStages::COMPUTE,
                    ty: wgpu::BindingType::Texture {
                        sample_type: wgpu::TextureSampleType::Float { filterable: false },
                        view_dimension: wgpu::TextureViewDimension::D2,
                        multisampled: false,
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 1,
                    visibility: wgpu::ShaderStages::COMPUTE,
                    ty: wgpu::BindingType::StorageTexture {
                        access: wgpu::StorageTextureAccess::WriteOnly,
                        format: wgpu::TextureFormat::Rgba32Float,
                        view_dimension: wgpu::TextureViewDimension::D2,
                    },
                    count: None,
                },
            ],
        });
    let group = ctx.device.create_bind_group(&wgpu::BindGroupDescriptor {
        label: Some("OIDN diagnostic copy group"),
        layout: &layout,
        entries: &[
            wgpu::BindGroupEntry {
                binding: 0,
                resource: wgpu::BindingResource::TextureView(source),
            },
            wgpu::BindGroupEntry {
                binding: 1,
                resource: wgpu::BindingResource::TextureView(&view),
            },
        ],
    });
    let shader = ctx
        .device
        .create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("OIDN diagnostic copy shader"),
            source: wgpu::ShaderSource::Wgsl(
                "@group(0) @binding(0) var src: texture_2d<f32>;
             @group(0) @binding(1) var dst: texture_storage_2d<rgba32float, write>;
             @compute @workgroup_size(8,8)
             fn main(@builtin(global_invocation_id) id: vec3<u32>) {
                 if any(id.xy >= textureDimensions(src)) { return; }
                 let p = vec2<i32>(id.xy);
                 textureStore(dst, p, textureLoad(src, p, 0));
             }"
                .into(),
            ),
        });
    let pipeline_layout = ctx
        .device
        .create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("OIDN diagnostic pipeline layout"),
            bind_group_layouts: &[Some(&layout)],
            immediate_size: 0,
        });
    let pipeline = ctx
        .device
        .create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
            label: Some("OIDN diagnostic copy pipeline"),
            layout: Some(&pipeline_layout),
            module: &shader,
            entry_point: Some("main"),
            compilation_options: Default::default(),
            cache: None,
        });
    let mut encoder = ctx.device.create_command_encoder(&Default::default());
    {
        let mut pass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor {
            label: Some("OIDN diagnostic capture"),
            timestamp_writes: None,
        });
        pass.set_pipeline(&pipeline);
        pass.set_bind_group(0, &group, &[]);
        pass.dispatch_workgroups(W.div_ceil(8), H.div_ceil(8), 1);
    }
    let mut staging = TextureReadback::default();
    readback_texture_bytes(
        ctx,
        &mut encoder,
        &texture,
        W,
        H,
        16,
        "OIDN probe readback",
        &mut staging,
    )?;
    ctx.queue.submit([encoder.finish()]);
    let bytes = map_readback(ctx, &staging)?;
    ensure!(
        bytes.len() == (W * H * 16) as usize,
        "capture length mismatch"
    );
    Ok(bytes
        .chunks_exact(4)
        .map(|b| f32::from_ne_bytes(b.try_into().expect("four-byte chunk")))
        .collect())
}

fn region_stats(data: &[f32], bright: bool) -> (f32, f32, f32) {
    let mut sum = 0.0_f64;
    let mut squared = 0.0_f64;
    let mut max = 0.0_f32;
    let mut count = 0_usize;
    for y in 0..H {
        for x in 0..W {
            let inside = (20..76).contains(&x) && (20..44).contains(&y);
            let dark = x < 8 || x >= W - 8;
            if (bright && !inside) || (!bright && !dark) {
                continue;
            }
            let i = ((y * W + x) * 4) as usize;
            let lum = data[i] * 0.2126 + data[i + 1] * 0.7152 + data[i + 2] * 0.0722;
            sum += f64::from(lum);
            squared += f64::from(lum).powi(2);
            max = max.max(lum);
            count += 1;
        }
    }
    let mean = sum / count as f64;
    let sd = (squared / count as f64 - mean * mean).max(0.0).sqrt();
    (mean as f32, sd as f32, max)
}

fn difference(a: &[f32], b: &[f32]) -> f32 {
    a.chunks_exact(4)
        .zip(b.chunks_exact(4))
        .flat_map(|(a, b)| (0..3).map(move |c| (a[c] - b[c]).abs()))
        .fold(0.0, f32::max)
}

fn main() -> Result<()> {
    env_logger::Builder::from_env(env_logger::Env::default().default_filter_or("warn")).init();
    for name in ["OIDN_INPUT_SCALE", "OIDN_STANDALONE_DEVICE"] {
        ensure!(
            std::env::var_os(name).is_none(),
            "unset {name} for this controlled diagnostic"
        );
    }
    let ctx = GpuContext::new().context("GPU initialization failed")?;
    println!("adapter={:?}", ctx.adapter.get_info());
    let mut pixels = Vec::<f32>::with_capacity((W * H * 4) as usize);
    for y in 0..H {
        for x in 0..W {
            let value = if (12..84).contains(&x) && (12..52).contains(&y) {
                if (x + y) % 2 == 0 { 48.0 } else { 8.0 }
            } else {
                0.25
            };
            pixels.extend([value, value * 0.8, value * 0.6, 1.0]);
        }
    }
    let color = ctx.device.create_texture(&wgpu::TextureDescriptor {
        label: Some("Frozen HDR probe input"),
        size: wgpu::Extent3d {
            width: W,
            height: H,
            depth_or_array_layers: 1,
        },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format: wgpu::TextureFormat::Rgba32Float,
        usage: wgpu::TextureUsages::COPY_DST | wgpu::TextureUsages::COPY_SRC,
        view_formats: &[],
    });
    ctx.queue.write_texture(
        wgpu::TexelCopyTextureInfo {
            texture: &color,
            mip_level: 0,
            origin: wgpu::Origin3d::ZERO,
            aspect: wgpu::TextureAspect::All,
        },
        bytemuck::cast_slice(&pixels),
        wgpu::TexelCopyBufferLayout {
            offset: 0,
            bytes_per_row: Some(W * 16),
            rows_per_image: Some(H),
        },
        wgpu::Extent3d {
            width: W,
            height: H,
            depth_or_array_layers: 1,
        },
    );
    let albedo_data = [0.7_f32, 0.7, 0.7, 1.0].repeat((W * H) as usize);
    let normal_data = [0.0_f32, 0.0, 1.0, 1.0].repeat((W * H) as usize);
    let albedo = ctx
        .device
        .create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("Frozen albedo"),
            contents: bytemuck::cast_slice(&albedo_data),
            usage: wgpu::BufferUsages::COPY_SRC,
        });
    let normal = ctx
        .device
        .create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("Frozen normal"),
            contents: bytemuck::cast_slice(&normal_data),
            usage: wgpu::BufferUsages::COPY_SRC,
        });
    let mut denoiser = OidnDenoiser::new(&ctx, W, H, None);
    denoiser.set_mode(OidnMode::ColorAlbedoNormal);
    denoiser.set_quality(Quality::Balanced);
    denoiser.set_external_input_scale(Some(0.02));
    denoiser.set_input_clamp(10.0);
    println!(
        "input=frozen synthetic HDR checkerboard; fixed_scale=0.02; clamp=10; mode=full_AOV; quality=Balanced"
    );
    println!("case,spp,highlight_mean,highlight_sd,highlight_max,dark_mean");
    let mut outputs = Vec::new();
    for (adaptive, spp) in [
        (true, 1),
        (true, 64),
        (true, 128),
        (true, 256),
        (true, 256),
        (false, 1),
        (false, 256),
    ] {
        denoiser.set_adaptive_clamp(adaptive);
        let encoder = ctx.device.create_command_encoder(&Default::default());
        denoiser.denoise(&ctx, encoder, &color, Some(&albedo), Some(&normal), spp)?;
        let output = capture(&ctx, denoiser.result_view())?;
        ensure!(output.iter().all(|x| x.is_finite()), "nonfinite output");
        let (mean, sd, max) = region_stats(&output, true);
        let (dark, _, _) = region_stats(&output, false);
        println!(
            "{},{spp},{mean:.6},{sd:.6},{max:.6},{dark:.6}",
            if adaptive { "adaptive" } else { "fixed" }
        );
        outputs.push(output);
    }
    let repeat = difference(&outputs[3], &outputs[4]);
    let fixed = difference(&outputs[5], &outputs[6]);
    let adaptive = difference(&outputs[0], &outputs[3]);
    println!(
        "max_abs_repeat={repeat:.9}; max_abs_fixed_spp={fixed:.9}; max_abs_adaptive_spp={adaptive:.9}"
    );
    let tolerance = 1e-4 * region_stats(&outputs[6], true).2.max(1.0);
    ensure!(
        repeat <= tolerance,
        "frozen repeated output changed beyond {tolerance}"
    );
    ensure!(
        fixed <= tolerance,
        "fixed-clamp output changed with spp beyond {tolerance}"
    );
    ensure!(
        adaptive > tolerance,
        "synthetic input did not expose sample-dependent clamp change"
    );
    let mut repeated_max = 0.0_f32;
    for _ in 0..32 {
        let encoder = ctx.device.create_command_encoder(&Default::default());
        denoiser.denoise(&ctx, encoder, &color, Some(&albedo), Some(&normal), 256)?;
        let output = capture(&ctx, denoiser.result_view())?;
        ensure!(
            output.iter().all(|x| x.is_finite()),
            "nonfinite repeated output"
        );
        repeated_max = repeated_max.max(difference(&outputs[6], &output));
    }
    println!("frozen_repeats=32; max_abs_repeated_series={repeated_max:.9}");
    ensure!(
        repeated_max <= tolerance,
        "frozen series changed beyond {tolerance}"
    );
    Ok(())
}
