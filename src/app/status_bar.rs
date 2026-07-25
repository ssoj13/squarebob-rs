//! Bottom status bar: scan progress, file info, hover info.

use eframe::egui;

use super::App;
use super::helpers::{disk_free_info, fmt_size};
use super::icons;
use crate::cache;

impl App {
    /// Render bottom status bar
    pub(super) fn ui_status_bar(&mut self, ui: &mut egui::Ui) {
        egui::Panel::bottom("status").show(ui, |ui| {
            ui.horizontal(|ui| {
                if self.progress.scanning {
                    ui.spinner();
                    let elapsed = self
                        .progress
                        .start_time
                        .map(|t| t.elapsed().as_secs_f32())
                        .unwrap_or(0.0);
                    let err_str = if self.progress.errors > 0 {
                        format!(" | {} errors", self.progress.errors)
                    } else {
                        String::new()
                    };
                    let engine = self.progress.scan_engine_label.as_deref().unwrap_or("…");
                    let detail = match self.progress.phase {
                        Some(crate::scanner::ScanPhase::IndexingVolume) => format!(
                            "Indexing volume: {} MFT records ({} files, {} dirs)",
                            self.progress.items, self.progress.files, self.progress.dirs
                        ),
                        Some(crate::scanner::ScanPhase::SelectingTree) => format!(
                            "Reading selected tree: {} MFT records ({} files, {} dirs)",
                            self.progress.items, self.progress.files, self.progress.dirs
                        ),
                        Some(crate::scanner::ScanPhase::MeasuringTree) => format!(
                            "Measuring selected tree: {} files, {} dirs, {}",
                            self.progress.files,
                            self.progress.dirs,
                            fmt_size(self.progress.bytes)
                        ),
                        Some(crate::scanner::ScanPhase::Walking) | None => format!(
                            "Scanning: {} files, {} dirs, {}",
                            self.progress.files,
                            self.progress.dirs,
                            fmt_size(self.progress.bytes)
                        ),
                    };
                    ui.label(format!("[{engine}] {detail} ({elapsed:.1}s){err_str}"));
                    let anim = (elapsed * 2.0).sin() * 0.5 + 0.5;
                    ui.add(egui::ProgressBar::new(anim).desired_width(100.0));
                } else if let Some(err) = &self.progress.error {
                    ui.colored_label(egui::Color32::RED, format!("Error: {err}"));
                } else if let Some(tree) = &self.tree {
                    let disk_info = disk_free_info(&self.scan_path);
                    let time_info = if let Some(age) = self.cache_age {
                        ui.colored_label(egui::Color32::from_rgb(180, 180, 80), icons::DOT);
                        format!(" cached: {}", cache::format_age(age))
                    } else {
                        format!(" in {:.1}s", self.progress.elapsed_secs)
                    };
                    ui.label(format!(
                        "{} files | {} dirs | {}{}{}",
                        tree.file_count,
                        tree.dir_count,
                        fmt_size(tree.size),
                        time_info,
                        disk_info,
                    ));
                } else {
                    ui.label("Select a folder and click Scan to analyze disk usage");
                }

                // Right side stats + hover
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    let now = std::time::Instant::now();
                    if now.duration_since(self.last_mem_update).as_secs_f32() > 0.5 {
                        // RAM via sysinfo — fast OS syscall, fine on the UI
                        // thread.
                        self.sys.refresh_memory();
                        let total_kb = self.sys.total_memory();
                        let used_kb = self.sys.used_memory();
                        self.mem_total_mb = (total_kb / 1024).max(1);
                        self.mem_used_mb = used_kb / 1024;
                        self.last_mem_update = now;
                    }
                    // VRAM is polled on a background thread (see
                    // `ensure_gpu_info_worker`). On Windows the underlying
                    // `gpu_mem::query()` shells out to `nvidia-smi`, which
                    // can block the UI thread for 50–200 ms per call — a
                    // visible per-second hiccup at every refresh. The
                    // worker pushes updates through a `crossbeam_channel`
                    // and we just drain whatever's available this frame.
                    self.ensure_gpu_info_worker();
                    if let Some(rx) = &self.gpu_info_rx {
                        while let Ok(info) = rx.try_recv() {
                            let to_mib = |b: u64| b / (1024 * 1024);
                            self.vram_total_mb = to_mib(info.dedicated_vram);
                            self.vram_free_mb = to_mib(info.free_vram);
                            self.vram_unified = info.unified;
                            if self.vram_name != info.name {
                                self.vram_name = info.name;
                            }
                        }
                    }
                    if self.vram_total_mb > 0 {
                        // Show used VRAM when `gpu-mem` could read free
                        // VRAM (NVIDIA all platforms, AMD on Linux, Apple
                        // Silicon). On AMD/Intel Windows / Intel macOS
                        // only total is available — fall back to a
                        // total-only readout. Unified-memory parts get a
                        // small `*` so users know VRAM and RAM are the
                        // same pool.
                        let star = if self.vram_unified { "*" } else { "" };
                        if self.vram_free_mb > 0 {
                            let used = self.vram_total_mb.saturating_sub(self.vram_free_mb);
                            ui.label(format!(
                                "VRAM {used} / {total} MB{star}",
                                total = self.vram_total_mb,
                            ));
                        } else {
                            ui.label(format!("VRAM {} MB{star}", self.vram_total_mb));
                        }
                    }
                    if self.mem_total_mb > 0 {
                        ui.label(format!(
                            "RAM {} / {} MB",
                            self.mem_used_mb, self.mem_total_mb
                        ));
                    }
                    if self.last_frame_ms > 0.0 {
                        // Show 1-second averaged FPS/ms when we have enough samples,
                        // otherwise fall back to the instantaneous reading. Stable
                        // values are easier to read while benchmarking.
                        let (fps, ms) = if self.frame_history.len() >= 2 {
                            (self.avg_fps, self.avg_frame_ms)
                        } else {
                            (self.last_fps, self.last_frame_ms)
                        };
                        let mut stats = format!(
                            "{:.1} FPS | {:.2} ms (1s avg, n={})",
                            fps,
                            ms,
                            self.frame_history.len()
                        );
                        if self.last_samples_per_sec > 0.0 {
                            stats.push_str(&format!(" | {:.0} spp/s", self.last_samples_per_sec));
                        }
                        ui.label(stats);

                        // Sample progress: current / target rendered as
                        // a `ProgressBar` with the `samples: N/M` text
                        // overlaid on the fill. `oidn_last_frame_count`
                        // mirrors `pt_frame_count()` (refreshed every
                        // PT step via `evaluate_oidn_trigger`), so it
                        // doubles as a current-spp readout without re-
                        // borrowing `renderer_3d` here.
                        if self.render_3d_opts.path_tracing && self.render_3d_opts.pt_samples > 0 {
                            let current = self.oidn_last_frame_count;
                            let target = self.render_3d_opts.pt_samples.max(1);
                            let fraction = (current as f32 / target as f32).clamp(0.0, 1.0);
                            ui.add(
                                egui::ProgressBar::new(fraction)
                                    .desired_width(140.0)
                                    .text(format!("samples: {}/{}", current, target)),
                            );
                        }
                    }
                    // OIDN stats: surface here so the user can see the
                    // last denoise cost without opening the Denoiser
                    // section. Only shown when OIDN is enabled and has
                    // produced at least one pass.
                    if self.render_3d_opts.pt_oidn_mode != render_shared::OidnModeOption::Off
                        && let Some(ms) = self.oidn_last_latency_ms
                    {
                        let state = if self.oidn_display_is_denoised {
                            "shown"
                        } else {
                            "stale"
                        };
                        ui.label(format!(
                            "OIDN: {:.0} ms @ {} spp ({})",
                            ms, self.oidn_last_interval_spp, state
                        ));
                    }
                    if let Some(hover) = &self.hovered {
                        ui.label(format!("{} ({})", hover.path, fmt_size(hover.size)));
                    }
                });
            });
        });
    }

    /// Spawn the background `gpu_mem::query()` poller exactly once.
    /// The thread runs forever (joined at process exit), sleeps
    /// 2 s between queries, and pushes each successful result
    /// through a `crossbeam_channel`. Polling cadence is
    /// deliberately slower than the 0.5 s sysinfo heartbeat — VRAM
    /// rarely changes that fast in normal use, and `nvidia-smi`
    /// is the expensive call (subprocess spawn + driver round-trip).
    pub(super) fn ensure_gpu_info_worker(&mut self) {
        if self.gpu_info_rx.is_some() {
            return;
        }
        let (tx, rx) = crossbeam_channel::bounded::<gpu_mem::GpuMemInfo>(2);
        let handle = std::thread::Builder::new()
            .name("squarebob-gpu-mem-poller".to_string())
            .spawn(move || {
                loop {
                    if let Some(info) = gpu_mem::query() {
                        if tx.send(info).is_err() {
                            // Receiver dropped — app shutting down.
                            break;
                        }
                    }
                    std::thread::sleep(std::time::Duration::from_secs(2));
                }
            })
            .ok();
        self.gpu_info_rx = Some(rx);
        self.gpu_info_thread = handle;
    }
}
