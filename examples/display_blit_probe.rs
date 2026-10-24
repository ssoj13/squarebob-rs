//! Validate the assembled production PT blit and raster shaders on a headless GPU.
//! Run with: cargo run --locked --example display_blit_probe
use std::sync::Arc;

use anyhow::{Context, Result, ensure};
use render_core::{
    DISPLAY_TEXTURE_FORMAT,
    gpu::{GpuContext, TextureReadback, TextureReadbackLayout, map_readback, readback_texture_bytes},
};

fn main() -> Result<()> {
    let ctx = Arc::new(GpuContext::new().context("GPU initialization failed")?);
    let scope = ctx.device.push_error_scope(wgpu::ErrorFilter::Validation);
    let result = (|| -> Result<Vec<f32>> {
        // This constructs the real PBR/skybox/outline/wireframe pipelines.
        let _renderer = render_3d::Renderer3D::new(ctx.clone());
        let mut tracer = pt_megakernel::PathTraceCompute::new(
            &ctx.device,
            &ctx.queue,
            4,
            1,
            DISPLAY_TEXTURE_FORMAT,
        )?;
        tracer.set_blit_exposure(&ctx.queue, 1.0);
        tracer.set_blit_odt_tag(&ctx.queue, 0);
        tracer.set_blit_color(&ctx.queue, 0, 0.0, 1.0, 0.0);
        let source = ctx.device.create_texture(&wgpu::TextureDescriptor {
            label: Some("frozen display blit input"),
            size: wgpu::Extent3d {
                width: 4,
                height: 1,
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
        let input: Vec<f32> = [0.18, 1.0, 4.0, -0.18]
            .into_iter()
            .flat_map(|v| [v, v, v, 1.0])
            .collect();
        ctx.queue.write_texture(
            wgpu::TexelCopyTextureInfo {
                texture: &source,
                mip_level: 0,
                origin: wgpu::Origin3d::ZERO,
                aspect: wgpu::TextureAspect::All,
            },
            bytemuck::cast_slice(&input),
            wgpu::TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(64),
                rows_per_image: Some(1),
            },
            source.size(),
        );
        let target = ctx.device.create_texture(&wgpu::TextureDescriptor {
            label: Some("extended sRGB blit capture"),
            size: source.size(),
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: DISPLAY_TEXTURE_FORMAT,
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_SRC,
            view_formats: &[],
        });
        let mut encoder = ctx.device.create_command_encoder(&Default::default());
        tracer.blit_with_source(
            &ctx.device,
            &mut encoder,
            &target.create_view(&Default::default()),
            Some(&source.create_view(&Default::default())),
        );
        let mut staging = TextureReadback::default();
        readback_texture_bytes(
            &ctx,
            &mut encoder,
            &target,
            TextureReadbackLayout::new("float display blit capture", 4, 1, 8)?,
            &mut staging,
        )?;
        ctx.queue.submit([encoder.finish()]);
        let bytes = map_readback(&ctx, &staging)?;
        ensure!(bytes.len() == 32, "unexpected raw RGBA16Float capture size");
        let output = bytes
            .chunks_exact(2)
            .map(|c| half::f16::from_bits(u16::from_le_bytes([c[0], c[1]])).to_f32())
            .collect();
        // Both PT and OIDN source contracts are immutable scene-linear RGBA32F.
        ctx.queue.write_texture(
            wgpu::TexelCopyTextureInfo {
                texture: tracer.output_texture(),
                mip_level: 0,
                origin: wgpu::Origin3d::ZERO,
                aspect: wgpu::TextureAspect::All,
            },
            bytemuck::cast_slice(&input),
            wgpu::TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(64),
                rows_per_image: Some(1),
            },
            source.size(),
        );
        // The OCIO lane: the window-canvas processor (HDR canvas, 203-nit white) as the oracle.
        let settings = render_color::ColorPipelineSettings {
            mode: render_color::ColorMode::Ocio,
            ..Default::default()
        };
        let pipeline = render_color::ColorPipeline::new(&settings);
        ensure!(pipeline.error().is_none(), "OCIO processor failed");
        let canvas = pipeline.canvas_processor(true, 203.0)?;
        let mut expected: Vec<[f32; 3]> = input
            .chunks_exact(4)
            .map(|pixel| [pixel[0] * 2.0, pixel[1] * 2.0, pixel[2] * 2.0])
            .collect();
        canvas.apply_rgb(&mut expected);
        // Canvas-encoded values come out of the blit as they are: compare them directly.
        let check = |bytes: &[u8], what: &str| -> Result<()> {
            for (pixel, expected) in bytes.chunks_exact(8).zip(&expected) {
                for channel in 0..3 {
                    let actual = half::f16::from_bits(u16::from_le_bytes([
                        pixel[channel * 2],
                        pixel[channel * 2 + 1],
                    ]))
                    .to_f32();
                    let want = expected[channel];
                    ensure!(
                        (actual - want).abs() < 0.002 * want.abs().max(1.0),
                        "{what}: actual {actual}, CPU oracle {want}"
                    );
                }
            }
            Ok(())
        };

        // GPU: the exact OCIO WGSL in the blit (tag 6), camera exposure 2 applied in the shader.
        let shader = render_color_gpu::OcioShader::new(
            &ctx.device,
            &ctx.queue,
            &canvas,
            wgpu::ShaderStages::FRAGMENT,
        )?;
        tracer.set_display_transform(&ctx.device, Some(std::sync::Arc::new(shader)));
        tracer.set_blit_exposure(&ctx.queue, 2.0);
        tracer.set_blit_color(&ctx.queue, render_color::OCIO_TAG, 0.0, 1.0, 0.0);
        tracer.set_blit_source_encoded(&ctx.queue, false);
        let mut encoder = ctx.device.create_command_encoder(&Default::default());
        tracer.blit_with_source(
            &ctx.device,
            &mut encoder,
            &target.create_view(&Default::default()),
            Some(&source.create_view(&Default::default())),
        );
        readback_texture_bytes(
            &ctx,
            &mut encoder,
            &target,
            TextureReadbackLayout::new("GPU OCIO blit capture", 4, 1, 8)?,
            &mut staging,
        )?;
        ctx.queue.submit([encoder.finish()]);
        check(&map_readback(&ctx, &staging)?, "GPU OCIO lane vs CPU processor")?;
        println!("gpu_ocio_lane_matches_cpu=true");

        // CPU: the same processor applied after readback; the blit passes its codes through.
        tracer.set_blit_exposure(&ctx.queue, 1.0);
        tracer.set_blit_color(&ctx.queue, 0, 0.0, 1.0, 0.0);
        tracer.set_blit_source_encoded(&ctx.queue, true);
        let mut cpu_reference = None;
        for external in [true, true, false, false] {
            let scratch =
                tracer.apply_cpu_color_in_place(&ctx, external.then_some(&source), |pixels| {
                    for pixel in pixels.iter_mut() {
                        *pixel = pixel.map(|value| value * 2.0);
                    }
                    canvas.apply_rgb(pixels);
                })?;
            let mut encoder = ctx.device.create_command_encoder(&Default::default());
            tracer.blit_with_source(
                &ctx.device,
                &mut encoder,
                &target.create_view(&Default::default()),
                Some(&scratch),
            );
            readback_texture_bytes(
                &ctx,
                &mut encoder,
                &target,
                TextureReadbackLayout::new("CPU display scratch", 4, 1, 8)?,
                &mut staging,
            )?;
            ctx.queue.submit([encoder.finish()]);
            let cpu_bytes = map_readback(&ctx, &staging)?;
            if let Some(reference) = &cpu_reference {
                ensure!(
                    &cpu_bytes == reference,
                    "Repeated CPU display compounded a transform"
                );
            } else {
                cpu_reference = Some(cpu_bytes.clone());
            }
            check(&cpu_bytes, "CPU OCIO codepath vs CPU processor")?;
            for raw_source in [&source, tracer.output_texture()] {
                let mut encoder = ctx.device.create_command_encoder(&Default::default());
                readback_texture_bytes(
                    &ctx,
                    &mut encoder,
                    raw_source,
                    TextureReadbackLayout::new("Raw display source preservation", 4, 1, 16)?,
                    &mut staging,
                )?;
                ctx.queue.submit([encoder.finish()]);
                ensure!(
                    map_readback(&ctx, &staging)? == bytemuck::cast_slice::<f32, u8>(&input),
                    "CPU display changed raw PT or denoiser source"
                );
            }
        }
        println!("cpu_display_source_preservation=true repeated_cpu_display=true");
        Ok(output)
    })();
    // Always collect validation failures, including failures during constructor work.
    let validation = pollster::block_on(scope.pop());
    ensure!(
        validation.is_none(),
        "assembled production shader validation failed: {validation:?}"
    );
    let output = result?;
    let mut maximum_error = 0.0_f32;
    for (pixel, linear) in [0.18_f32, 1.0, 4.0, -0.18].into_iter().enumerate() {
        let magnitude = linear.abs();
        let oracle = linear.signum()
            * if magnitude <= 0.0031308 {
                magnitude * 12.92
            } else {
                1.055 * magnitude.powf(1.0 / 2.4) - 0.055
            };
        for channel in 0..3 {
            let actual = output[pixel * 4 + channel];
            let error = (actual - oracle).abs();
            maximum_error = maximum_error.max(error);
            ensure!(
                actual.is_finite() && error < 0.001,
                "pixel {pixel} channel {channel}: actual {actual}, oracle {oracle}"
            );
        }
        ensure!(
            output[pixel * 4 + 3] == 1.0,
            "alpha changed at pixel {pixel}"
        );
    }
    ensure!(
        output[8] > 1.0 && output[12] < 0.0,
        "extended range was clipped"
    );
    println!(
        "{}",
        serde_json::json!({
            "adapter": ctx.adapter.get_info().name,
            "backend": format!("{:?}", ctx.adapter.get_info().backend),
            "format": format!("{DISPLAY_TEXTURE_FORMAT:?}"),
            "validation": "passed",
            "raster_pipeline_creation": "passed",
            "linear_input": [0.18, 1.0, 4.0, -0.18],
            "rgba_output": output,
            "maximum_absolute_error": maximum_error,
        })
    );
    Ok(())
}
