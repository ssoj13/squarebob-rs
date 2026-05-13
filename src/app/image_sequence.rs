//! Render export adapter for the reusable `imageseq-rs` crate.

use eframe::egui;

use crate::renderer::RenderMode;

use super::{settings::tinted_section, App};

impl App {
    pub(super) fn ui_image_sequence_panel(&mut self, ui: &mut egui::Ui) {
        tinted_section(
            ui,
            "Render Export",
            true,
            self.settings_tint_mix,
            self.settings_section_header_height,
            |ui| {
                let running = self.image_sequence_progress.is_some();
                let snapshot = self
                    .image_sequence_progress
                    .as_ref()
                    .map(imageseq_rs::SequenceProgress::snapshot);
                let response = self.image_sequence_dialog.ui(
                    ui,
                    running,
                    snapshot.as_ref(),
                    self.image_sequence_error.as_deref(),
                );

                if response.browse_output {
                    self.browse_image_sequence_output();
                }
                if let Some(request) = response.start {
                    self.start_image_sequence_export(request);
                }
                if response.stop {
                    self.stop_image_sequence_export("Stopped");
                }
            },
        );
    }

    pub(super) fn handle_image_sequence(&mut self, ctx: &egui::Context) {
        if self.image_sequence_progress.is_none() {
            return;
        }

        ctx.request_repaint();

        let samples = self
            .renderer_3d
            .as_ref()
            .map(|renderer| renderer.pt_frame_count())
            .unwrap_or(0);

        let ready_path = {
            let Some(progress) = &mut self.image_sequence_progress else {
                return;
            };
            progress.observe_samples(samples);
            if progress.current_frame_ready() {
                Some(progress.current_frame_path())
            } else {
                None
            }
        };

        let Some(path) = ready_path else {
            return;
        };

        let (width, height) = self.last_render_size;
        if width == 0 || height == 0 {
            self.stop_image_sequence_export("No rendered viewport is available");
            return;
        }

        let pixels = self.capture_viewport(width, height);
        let expected_len = width as usize * height as usize * 4;
        if pixels.len() != expected_len {
            self.stop_image_sequence_export("Renderer returned an invalid RGBA buffer");
            return;
        }

        if let Err(err) = imageseq_rs::save_rgba_png(&path, width, height, &pixels) {
            self.stop_image_sequence_export(format!("Failed to write {}: {err}", path.display()));
            return;
        }

        let mut finished = false;
        let mut next_frame_seconds = None;
        if let Some(progress) = &mut self.image_sequence_progress {
            progress.complete_current_frame();
            finished = progress.is_finished();
            if !finished {
                next_frame_seconds = Some(progress.current_frame_time_seconds());
            }
        }

        if finished {
            self.finish_image_sequence_export();
        } else if let Some(frame_seconds) = next_frame_seconds {
            self.apply_image_sequence_time(frame_seconds);
            if let Some(renderer) = &mut self.renderer_3d {
                renderer.reset_pt_accumulation();
            }
        }
    }

    fn browse_image_sequence_output(&mut self) {
        let selected = rfd::FileDialog::new()
            .set_title("Choose render output")
            .add_filter("PNG sequence", &["png"])
            .set_file_name("frame_####.png")
            .save_file();

        if let Some(path) = selected {
            self.image_sequence_dialog
                .set_output_path(path.to_string_lossy().to_string());
        }
    }

    fn start_image_sequence_export(&mut self, request: imageseq_rs::EncodeRequest) {
        self.image_sequence_error = None;

        if request.export_mode != imageseq_rs::ExportMode::ImageSequence {
            self.image_sequence_error =
                Some("Video export is configured, but FFmpeg sink is not enabled yet".to_string());
            return;
        }

        if self.display_root().is_none() {
            self.image_sequence_error = Some("Scan data is not ready".to_string());
            return;
        }

        if self.renderer_3d.is_none() {
            self.image_sequence_error = Some("3D renderer is not ready".to_string());
            return;
        }

        if self.last_render_size.0 == 0 || self.last_render_size.1 == 0 {
            self.image_sequence_error = Some("Render viewport is not ready".to_string());
            return;
        }

        let job = request.sequence_job();
        self.render_mode = RenderMode::Mode3D;
        self.image_sequence_base_animation_time = self.render_3d_opts.animation_time;
        self.image_sequence_base_env_time = self.render_3d_opts.env_time;
        self.image_sequence_restore_animate = self.render_3d_opts.animate;
        self.image_sequence_restore_env_animate = self.render_3d_opts.env_animate;
        self.image_sequence_restore_path_tracing = self.render_3d_opts.path_tracing;
        self.image_sequence_restore_pt_max_samples = self.render_3d_opts.pt_max_samples;

        self.render_3d_opts.path_tracing = true;
        self.render_3d_opts.pt_max_samples = job.max_samples;
        self.render_3d_opts.animate = false;
        self.render_3d_opts.env_animate = false;
        self.image_sequence_progress = Some(imageseq_rs::SequenceProgress::new(job));
        self.apply_image_sequence_time(0.0);

        if let Some(renderer) = &mut self.renderer_3d {
            renderer.reset_pt_accumulation();
        }
    }

    fn stop_image_sequence_export(&mut self, message: impl Into<String>) {
        self.restore_image_sequence_render_state();
        self.image_sequence_progress = None;
        self.image_sequence_error = Some(message.into());
    }

    fn finish_image_sequence_export(&mut self) {
        self.restore_image_sequence_render_state();
        self.image_sequence_progress = None;
        self.image_sequence_error = Some("Complete".to_string());
    }

    fn restore_image_sequence_render_state(&mut self) {
        self.render_3d_opts.animate = self.image_sequence_restore_animate;
        self.render_3d_opts.env_animate = self.image_sequence_restore_env_animate;
        self.render_3d_opts.path_tracing = self.image_sequence_restore_path_tracing;
        if self.image_sequence_restore_pt_max_samples > 0 {
            self.render_3d_opts.pt_max_samples = self.image_sequence_restore_pt_max_samples;
        }
    }

    fn apply_image_sequence_time(&mut self, frame_seconds: f32) {
        self.render_3d_opts.animation_time = self.image_sequence_base_animation_time
            + frame_seconds * self.render_3d_opts.animation_speed;
        self.render_3d_opts.env_time = self.image_sequence_base_env_time
            + frame_seconds * self.render_3d_opts.animation_speed * self.render_3d_opts.env_speed;
    }
}
