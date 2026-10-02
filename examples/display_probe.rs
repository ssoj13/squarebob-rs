//! Headless GPU verification of the same egui-display PresentPass used by the window.
//! Run: cargo run --locked --example display_probe
//! This verifies signal encoding, not monitor capability negotiation or perception.
use anyhow::{Context, Result, ensure};
use egui_display::{Output, PresentPass, Target, screenshot::Pixels, transfer};
use render_core::gpu::GpuContext;

fn canvas(ctx: &GpuContext, present: &mut PresentPass, linear: [f32; 3]) -> [f32; 3] {
    let gamma = linear.map(transfer::oetf);
    present.prepare_canvas(&ctx.device, &ctx.queue, 8, 8);
    let view = present.canvas(&ctx.device, 8, 8);
    let mut encoder = ctx
        .device
        .create_command_encoder(&wgpu::CommandEncoderDescriptor {
            label: Some("Display probe canvas"),
        });
    {
        let _pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("Known extended-sRGB color"),
            color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                view,
                resolve_target: None,
                depth_slice: None,
                ops: wgpu::Operations {
                    load: wgpu::LoadOp::Clear(wgpu::Color {
                        r: gamma[0] as f64,
                        g: gamma[1] as f64,
                        b: gamma[2] as f64,
                        a: 1.0,
                    }),
                    store: wgpu::StoreOp::Store,
                },
            })],
            ..Default::default()
        });
    }
    ctx.queue.submit([encoder.finish()]);
    gamma.map(|value| transfer::eotf(half::f16::from_f32(value).to_f32()))
}

fn main() -> Result<()> {
    let ctx = GpuContext::new().context("No GPU available for display probe")?;
    println!("adapter={:?}", ctx.adapter.get_info());
    ensure!(
        render_core::DISPLAY_TEXTURE_FORMAT == egui_display::CANVAS_FORMAT,
        "Renderer and compositor canvas formats differ"
    );
    let mut present = PresentPass::new(
        &ctx.device,
        wgpu::TextureFormat::Rgb10a2Unorm,
        Output::Hdr10,
    );
    // Isolate transfer/quantization from intentional display dithering.
    present.dither = false;
    let target = Target {
        hdr: true,
        white: 203.0,
        peak: 1000.0,
    };
    let mut pq_white = 0.0;
    let mut pq_highlight = 0.0;
    for nits in [0.0_f32, 36.54, 100.0, 203.0, 1000.0] {
        let linear = nits / target.white;
        let gamma = transfer::oetf(linear);
        // The real float canvas rounds its extended-sRGB values to f16.
        let stored_gamma = half::f16::from_f32(gamma).to_f32();
        let stored_linear = transfer::eotf(stored_gamma);
        canvas(&ctx, &mut present, [linear; 3]);

        let capture = present
            .capture(&ctx.device, &ctx.queue, Output::Hdr10, target)?
            .wait(&ctx.device)?;
        let Pixels::Rgba16(pq_pixels) = capture.pixels else {
            anyhow::bail!("PQ capture is not 10-bit codes");
        };
        let expected_pq = egui_display::pq(stored_linear * target.white);
        let mut max_pq_error = 0.0_f32;
        for pixel in pq_pixels.chunks_exact(4) {
            ensure!(pixel[3] == u16::MAX, "PQ alpha changed");
            for channel in &pixel[..3] {
                let signal = *channel as f32 / u16::MAX as f32;
                max_pq_error = max_pq_error.max((signal - expected_pq).abs());
            }
        }
        ensure!(
            max_pq_error <= 2.5 / 1023.0,
            "PQ transfer mismatch at {nits} nits: {max_pq_error}"
        );
        let pq_signal = pq_pixels[0] as f32 / u16::MAX as f32;
        if nits == 203.0 {
            pq_white = pq_signal;
        }
        if nits == 1000.0 {
            pq_highlight = pq_signal;
        }

        let sdr = present
            .capture(&ctx.device, &ctx.queue, Output::Sdr8, Target::default())?
            .wait(&ctx.device)?;
        let Pixels::Rgba8(sdr_pixels) = sdr.pixels else {
            anyhow::bail!("SDR capture is not RGBA8");
        };
        let expected_sdr = transfer::oetf(stored_linear).clamp(0.0, 1.0);
        let mut max_sdr_error = 0.0_f32;
        for pixel in sdr_pixels.chunks_exact(4) {
            ensure!(pixel[3] == u8::MAX, "SDR alpha changed");
            for channel in &pixel[..3] {
                max_sdr_error = max_sdr_error.max((*channel as f32 / 255.0 - expected_sdr).abs());
            }
        }
        ensure!(
            max_sdr_error <= 1.5 / 255.0,
            "SDR transfer mismatch at {nits} nits: {max_sdr_error}"
        );
        if nits == 1000.0 {
            ensure!(
                sdr_pixels[0] == 255,
                "SDR must clip highlights at reference white"
            );
        }

        let scrgb = present
            .capture(&ctx.device, &ctx.queue, Output::Scrgb, target)?
            .wait(&ctx.device)?;
        let Pixels::RgbaF16(scrgb_pixels) = scrgb.pixels else {
            anyhow::bail!("scRGB capture is not f16");
        };
        let expected_scrgb = stored_linear * target.white / egui_display::SCRGB_NITS;
        let max_scrgb_error = scrgb_pixels
            .chunks_exact(4)
            .flat_map(|pixel| pixel[..3].iter())
            .map(|value| (value.to_f32() - expected_scrgb).abs())
            .fold(0.0_f32, f32::max);
        ensure!(
            max_scrgb_error <= expected_scrgb.abs().max(1.0) * 0.002,
            "scRGB transfer mismatch at {nits} nits: {max_scrgb_error}"
        );
        println!(
            "nits={nits:.2} canvas_gamma={stored_gamma:.6} pq={pq_signal:.6} pq_expected={expected_pq:.6} pq_error={max_pq_error:.6} sdr_error={max_sdr_error:.6} scrgb_error={max_scrgb_error:.6}"
        );
    }
    for (name, primary) in [
        ("red", [1.0, 0.0, 0.0]),
        ("green", [0.0, 1.0, 0.0]),
        ("blue", [0.0, 0.0, 1.0]),
    ] {
        for peak in [203.0, 1000.0] {
            let linear = primary.map(|channel| channel * peak / target.white);
            let stored_linear = canvas(&ctx, &mut present, linear);
            let expected =
                egui_display::rec2020_nits(stored_linear, target.white).map(egui_display::pq);
            let capture = present
                .capture(&ctx.device, &ctx.queue, Output::Hdr10, target)?
                .wait(&ctx.device)?;
            let Pixels::Rgba16(pixels) = capture.pixels else {
                anyhow::bail!("PQ primary capture is not 10-bit");
            };
            let mut max_error = 0.0_f32;
            for pixel in pixels.chunks_exact(4) {
                for channel in 0..3 {
                    max_error = max_error
                        .max((pixel[channel] as f32 / u16::MAX as f32 - expected[channel]).abs());
                }
            }
            ensure!(
                max_error <= 2.5 / 1023.0,
                "Rec709 to Rec2020/PQ mismatch for {name} at {peak} nits: {max_error}"
            );
            println!(
                "primary={name} nits={peak:.0} pq={:?} expected={expected:?} max_error={max_error:.6}",
                pixels[..3]
                    .iter()
                    .map(|value| *value as f32 / u16::MAX as f32)
                    .collect::<Vec<_>>()
            );
        }
    }
    ensure!(
        pq_highlight > pq_white + 0.1,
        "PQ highlights lost headroom above reference white"
    );
    println!("display_probe_passed=true pq_white={pq_white:.6} pq_highlight={pq_highlight:.6}");
    Ok(())
}
