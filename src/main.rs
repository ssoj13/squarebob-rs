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
    let dirstat_level = match log_level {
        "warn" => log::LevelFilter::Warn,
        "info" => log::LevelFilter::Info,
        "debug" => log::LevelFilter::Debug,
        _ => log::LevelFilter::Trace,
    };
    builder.filter_module("dirstat_rs", dirstat_level);
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

    info!("dirstat-rs starting (log level: {})", log_level);
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

    // Configure wgpu to request POLYGON_MODE_LINE for wireframe rendering
    let mut wgpu_setup = eframe::egui_wgpu::WgpuSetupCreateNew::without_display_handle();
    wgpu_setup.device_descriptor = std::sync::Arc::new(|_adapter| wgpu::DeviceDescriptor {
        label: Some("dirstat-rs device"),
        required_features: wgpu::Features::POLYGON_MODE_LINE,
        required_limits: wgpu::Limits::default(),
        memory_hints: Default::default(),
        trace: Default::default(),
        experimental_features: Default::default(),
    });

    let options = eframe::NativeOptions {
        viewport: eframe::egui::ViewportBuilder::default()
            .with_inner_size([1280.0, 800.0])
            .with_title("dirstat-rs"),
        persist_window: true,
        wgpu_options: eframe::egui_wgpu::WgpuConfiguration {
            wgpu_setup: eframe::egui_wgpu::WgpuSetup::CreateNew(wgpu_setup),
            ..Default::default()
        },
        ..Default::default()
    };

    eframe::run_native(
        "dirstat-rs",
        options,
        Box::new(move |cc| Ok(Box::new(app::App::new(cc, cli)))),
    )
}
