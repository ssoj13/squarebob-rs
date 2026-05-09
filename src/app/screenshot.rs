//! Automated screenshot capture and PNG saving.
//!
//! Extracted from `mod.rs` for review/merge sanity. No behaviour change.

use eframe::egui;
use log::info;

use crate::renderer::{self, RenderBackend, RenderMode};

use super::App;

impl App {
    /// Handle automated screenshot capture
    pub(super) fn handle_screenshot(&mut self, ctx: &egui::Context) {
        if self.screenshot_taken {
            return;
        }
        let Some(delay) = self.screenshot_delay else {
            return;
        };
        let Some(start) = self.screenshot_start_time else {
            return;
        };

        let elapsed = start.elapsed().as_secs_f32();
        if elapsed < delay {
            ctx.request_repaint();
            return;
        }

        // Time to take screenshot
        self.screenshot_taken = true;

        let path = self.screenshot_path.clone().unwrap_or_else(|| {
            let temp = std::env::temp_dir();
            temp.join("dirstat_screenshot.png")
                .to_string_lossy()
                .to_string()
        });

        // Re-render to capture latest state
        let (w, h) = self.last_render_size;
        if w > 0 && h > 0 {
            let pixels = self.capture_viewport(w, h);
            if !pixels.is_empty() {
                info!("Taking screenshot: {}x{} -> {}", w, h, path);
                if let Err(e) = save_png(&path, w, h, pixels) {
                    log::error!("Failed to save screenshot: {}", e);
                } else {
                    info!("Screenshot saved: {}", path);
                }
            }
        } else {
            log::warn!("No render available for screenshot");
        }

        if self.exit_after_screenshot {
            ctx.send_viewport_cmd(egui::ViewportCommand::Close);
        }
    }

    /// Capture viewport pixels for screenshot
    fn capture_viewport(&mut self, w: u32, h: u32) -> Vec<u8> {
        match self.render_mode {
            RenderMode::Mode2D => {
                match self.render_backend {
                    RenderBackend::Cpu => {
                        let root_ptr = match self.display_root() {
                            Some(root) => root as *const _,
                            None => return Vec::new(),
                        };
                        // Safe: root lives in self for duration of this call.
                        let root = unsafe { &*root_ptr };
                        renderer::cpu::render(root, &self.viewport, &self.opts)
                    }
                    RenderBackend::Gpu => {
                        let root_ptr = match self.display_root() {
                            Some(root) => root as *const _,
                            None => return Vec::new(),
                        };
                        let mut renderer = self.renderer_2d_gpu.take();
                        let pixels = if let Some(r) = &mut renderer {
                            // Safe: root lives in self for duration of this call.
                            let root = unsafe { &*root_ptr };
                            r.render(root, &self.viewport, &self.opts)
                        } else {
                            Vec::new()
                        };
                        self.renderer_2d_gpu = renderer;
                        pixels
                    }
                }
            }
            RenderMode::Mode3D => {
                // If we already rendered this frame, just read back.
                if self.last_render_frame_3d == self.frame_count {
                    if let Some(r) = &self.renderer_3d {
                        return r.readback_render_texture();
                    }
                }

                // Otherwise, render once and read back.
                let root_ptr = match self.display_root() {
                    Some(root) => root as *const _,
                    None => return Vec::new(),
                };
                if let Some(r) = &mut self.renderer_3d {
                    // Safe: root lives in self for duration of this call.
                    let root = unsafe { &*root_ptr };
                    r.render_to_view(
                        root,
                        w,
                        h,
                        &self.orbit_camera,
                        &self.render_3d_opts,
                        &self.opts,
                    );
                    self.last_render_frame_3d = self.frame_count;
                    r.readback_render_texture()
                } else {
                    Vec::new()
                }
            }
        }
    }
}

/// Save RGBA pixels as PNG using image crate
fn save_png(path: &str, w: u32, h: u32, pixels: Vec<u8>) -> Result<(), Box<dyn std::error::Error>> {
    // Create parent directory if needed
    if let Some(parent) = std::path::Path::new(path).parent() {
        if !parent.as_os_str().is_empty() && !parent.exists() {
            std::fs::create_dir_all(parent)?;
        }
    }
    let img = image::RgbaImage::from_raw(w, h, pixels).ok_or("Invalid image dimensions")?;
    img.save(path)?;
    Ok(())
}
