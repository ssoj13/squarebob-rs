mod app;
mod cache;
mod cli;
mod cli_test;
mod events;
mod exclusions;
mod path_key;
mod renderer;
mod scanner;
mod scanner_ntfs;

// Re-export so existing `crate::CliOptions` references in `app/cli_apply.rs`
// keep working without churn after the cli module extract.
pub use cli::CliOptions;

use log::info;

fn main() -> eframe::Result<()> {
    let cli = cli::parse_args();

    if cli.help {
        cli::print_help();
        return Ok(());
    }

    if let Some(ref test_args) = cli.test_args {
        if let Err(e) = cli_test::run(test_args.as_slice()) {
            eprintln!("{e:#}");
            std::process::exit(1);
        }
        return Ok(());
    }

    let info = auto_allocator::get_allocator_info();
    println!(
        "Using allocator: {:?} | Reason: {}",
        info.allocator_type, info.reason
    );

    // Setup logging based on verbosity
    let log_level = match cli.verbosity {
        0 => "warn",
        1 => "info",
        2 => "debug",
        _ => "trace",
    };

    let mut builder = env_logger::Builder::new();
    let squarebob_level = match log_level {
        "warn" => log::LevelFilter::Warn,
        "info" => log::LevelFilter::Info,
        "debug" => log::LevelFilter::Debug,
        _ => log::LevelFilter::Trace,
    };
    builder.filter_module("squarebob_rs", squarebob_level);
    // OIDN denoiser: TRACE so every step is visible while we're debugging
    // the bridge. Also bump the forked `oidn-rs` inner crates so per-tile
    // forward/in/out stats land in the log.
    builder.filter_module("pt_denoise_oidn", log::LevelFilter::Trace);
    builder.filter_module("oidn_rs", log::LevelFilter::Debug);
    builder.filter_module("oidn_model", log::LevelFilter::Debug);
    builder.filter_module("oidn_tza", log::LevelFilter::Debug);
    if let Some(list) = &cli.log_modules {
        for item in list.split(',').map(|s| s.trim().to_lowercase()) {
            match item.as_str() {
                "pt" => {
                    builder.filter_module("pt_megakernel", log::LevelFilter::Trace);
                    builder.filter_module("pt_core", log::LevelFilter::Trace);
                    builder.filter_module("bvh_gpu", log::LevelFilter::Trace);
                    builder.filter_module("render_3d", log::LevelFilter::Trace);
                }
                "wf" => {
                    builder.filter_module("pt_wavefront::wavefront", log::LevelFilter::Trace);
                }
                "pg" => {
                    builder.filter_module("pt_megakernel::pathguide", log::LevelFilter::Trace);
                }
                "" => {}
                other => {
                    eprintln!("Unknown log module '{}'", other);
                }
            }
        }
    }
    if cli.log_pt {
        builder.filter_module("pt_megakernel", log::LevelFilter::Trace);
        builder.filter_module("pt_core", log::LevelFilter::Trace);
        builder.filter_module("bvh_gpu", log::LevelFilter::Trace);
        builder.filter_module("render_3d", log::LevelFilter::Trace);
    }
    if cli.log_wf {
        builder.filter_module("pt_wavefront::wavefront", log::LevelFilter::Trace);
    }
    if cli.log_pg {
        builder.filter_module("pt_megakernel::pathguide", log::LevelFilter::Trace);
    }
    // Suppress noisy dependencies
    builder.filter_module("naga", log::LevelFilter::Warn);
    builder.filter_module("wgpu", log::LevelFilter::Warn);
    builder.filter_module("eframe", log::LevelFilter::Warn);
    builder.filter_module("egui", log::LevelFilter::Warn);
    builder.format_timestamp_millis();

    if let Some(ref log_path) = cli.log_file {
        use std::fs::File;
        use std::io::Write;
        match File::create(log_path) {
            Ok(file) => {
                let file = std::sync::Mutex::new(file);
                builder.format(move |_buf, record| {
                    let mut f = file.lock().unwrap();
                    writeln!(f, "[{:5}] {}", record.level(), record.args())
                });
            }
            Err(e) => eprintln!("Failed to create log file {log_path:?}: {e}"),
        }
    }

    builder.init();

    info!("squarebob-rs starting (log level: {})", log_level);
    if let Some(mode) = &cli.mode {
        info!("CLI mode: {:?}", mode);
    }
    if let Some(backend) = &cli.backend {
        info!("CLI backend: {:?}", backend);
    }
    if cli.screenshot_delay.is_some() {
        info!(
            "Screenshot mode: delay={:?}s, path={:?}",
            cli.screenshot_delay,
            cli.screenshot_path
                .as_deref()
                .unwrap_or("temp/screenshot.png")
        );
    }

    // Build the wgpu setup ourselves so we can share the same Instance /
    // Adapter / Device / Queue with every consumer: eframe (via
    // `WgpuSetup::Existing`), the path-tracer compute passes, treemap GPU
    // backend, and Burn-wgpu inside `pt-denoise-oidn` for the OIDN denoiser.
    //
    // Limits (POLYGON_MODE_LINE, max_storage_buffers_per_shader_stage = 16)
    // live in `render_core::gpu::GpuContext::new` — this is the single source
    // of wgpu setup truth.
    let gpu_ctx = match render_core::gpu::GpuContext::new() {
        Some(ctx) => std::sync::Arc::new(ctx),
        None => {
            eprintln!("Fatal: failed to initialise wgpu device (see log for details)");
            std::process::exit(1);
        }
    };

    let existing_setup = eframe::egui_wgpu::WgpuSetupExisting {
        instance: (*gpu_ctx.instance).clone(),
        adapter: (*gpu_ctx.adapter).clone(),
        device: (*gpu_ctx.device).clone(),
        queue: (*gpu_ctx.queue).clone(),
    };

    let options = eframe::NativeOptions {
        viewport: eframe::egui::ViewportBuilder::default()
            .with_inner_size([1280.0, 800.0])
            .with_title("squarebob-rs"),
        persist_window: true,
        wgpu_options: eframe::egui_wgpu::WgpuConfiguration {
            wgpu_setup: eframe::egui_wgpu::WgpuSetup::Existing(existing_setup),
            ..Default::default()
        },
        ..Default::default()
    };

    let gpu_for_app = gpu_ctx.clone();
    eframe::run_native(
        "squarebob-rs",
        options,
        Box::new(move |cc| Ok(Box::new(app::App::new(cc, cli, gpu_for_app.clone())))),
    )
}
