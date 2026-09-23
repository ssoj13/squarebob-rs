//! Automated screenshot capture and PNG saving.

use eframe::egui;
use log::info;

use crate::renderer::{self, RenderBackend, RenderMode};

use super::App;

impl App {
    /// Handle automated screenshot capture.
    pub(super) fn handle_screenshot(&mut self, ctx: &egui::Context) {
        if self.screenshot_taken {
            return;
        }
        if let Some(error) = self.screenshot_error.clone() {
            egui::Window::new("Screenshot failed")
                .resizable(false)
                .show(ctx, |ui| {
                    ui.label(error);
                    ui.horizontal(|ui| {
                        if ui.button("Retry").clicked() {
                            self.screenshot_error = None;
                            self.screenshot_start_time = Some(std::time::Instant::now());
                            self.screenshot_delay = Some(0.0);
                        }
                        if ui.button("Dismiss").clicked() {
                            self.screenshot_error = None;
                            self.screenshot_delay = None;
                        }
                    });
                });
            return;
        }
        let Some(delay) = self.screenshot_delay else {
            return;
        };
        let Some(start) = self.screenshot_start_time else {
            return;
        };

        if start.elapsed().as_secs_f32() < delay {
            ctx.request_repaint();
            return;
        }

        let (w, h) = self.last_render_size;
        let result: Result<String, String> = (|| {
            let path = self
                .screenshot_path
                .clone()
                .ok_or("Screenshot output path is unavailable")?;
            if w == 0 || h == 0 {
                return Err("No render available for screenshot".to_owned());
            }
            let pixels = self.capture_viewport(w, h)?;
            info!("Taking screenshot: {}x{} -> {}", w, h, path);
            save_png(&path, w, h, pixels)
                .map_err(|error| format!("Failed to save screenshot: {error}"))?;
            Ok(path)
        })();

        match result {
            Ok(path) => {
                self.screenshot_taken = true;
                info!("Screenshot saved: {}", path);
                if self.exit_after_screenshot {
                    ctx.send_viewport_cmd(egui::ViewportCommand::Close);
                }
            }
            Err(error) => {
                log::error!("{error}");
                self.screenshot_error = Some(error);
            }
        }
    }

    /// Capture and validate viewport pixels for screenshots and image sequences.
    pub(super) fn capture_viewport(&mut self, w: u32, h: u32) -> Result<Vec<u8>, String> {
        let pixels = match self.render_mode {
            RenderMode::Mode2D => match self.render_backend {
                RenderBackend::Cpu => {
                    let root_ptr = self
                        .display_root()
                        .ok_or("No display tree available for CPU capture")?
                        as *const _;
                    // SAFETY: root_ptr aliases self.tree, which remains owned and unchanged
                    // for the duration of this render call.
                    let root = unsafe { &*root_ptr };
                    renderer::cpu::render(root, &self.viewport, &self.opts)
                        .map_err(|error| format!("CPU capture render failed: {error}"))?
                }
                RenderBackend::Gpu => {
                    let root_ptr = self
                        .display_root()
                        .ok_or("No display tree available for GPU capture")?
                        as *const _;
                    let mut renderer = self.renderer_2d_gpu.take();
                    let result = if let Some(r) = &mut renderer {
                        // SAFETY: root_ptr aliases self.tree; rendering does not mutate it.
                        let root = unsafe { &*root_ptr };
                        r.render(root, &self.viewport, &self.opts)
                            .map_err(|error| format!("GPU 2D capture readback failed: {error}"))
                    } else {
                        Err("GPU 2D renderer is unavailable".to_owned())
                    };
                    self.renderer_2d_gpu = renderer;
                    result?
                }
            },
            RenderMode::Mode3D => {
                if self.last_render_frame_3d == self.frame_count
                    && let Some(renderer) = &mut self.renderer_3d
                {
                    renderer
                        .readback_render_texture()
                        .map_err(|error| format!("3D capture readback failed: {error}"))?
                } else {
                    let root_ptr = self
                        .display_root()
                        .ok_or("No display tree available for 3D capture")?
                        as *const _;
                    let renderer = self
                        .renderer_3d
                        .as_mut()
                        .ok_or("3D renderer is unavailable")?;
                    // SAFETY: root_ptr aliases self.tree; rendering reads it without mutation.
                    let root = unsafe { &*root_ptr };
                    renderer
                        .render_to_view(
                            root,
                            w,
                            h,
                            &self.orbit_camera,
                            &self.render_3d_opts,
                            &self.opts,
                            Some(&mut self.selected_3d_ids),
                        )
                        .map_err(|error| format!("3D capture render failed: {error}"))?;
                    self.last_render_frame_3d = self.frame_count;
                    renderer
                        .readback_render_texture()
                        .map_err(|error| format!("3D capture readback failed: {error}"))?
                }
            }
        };

        let expected = (w as usize)
            .checked_mul(h as usize)
            .and_then(|pixels| pixels.checked_mul(4))
            .ok_or("Capture dimensions overflow")?;
        if pixels.len() != expected {
            return Err(format!(
                "Capture returned {} bytes for {w}x{h}; expected {expected}",
                pixels.len()
            ));
        }
        Ok(pixels)
    }
}

/// Save RGBA pixels as PNG using image crate.
fn save_png(path: &str, w: u32, h: u32, pixels: Vec<u8>) -> Result<(), Box<dyn std::error::Error>> {
    if let Some(parent) = std::path::Path::new(path).parent()
        && !parent.as_os_str().is_empty()
        && !parent.exists()
    {
        std::fs::create_dir_all(parent)?;
    }
    let img = image::RgbaImage::from_raw(w, h, pixels).ok_or("Invalid image dimensions")?;
    img.save(path)?;
    Ok(())
}
