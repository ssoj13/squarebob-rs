//! Encode dialog adapter for the reusable `media-encoder` crate.

use std::fmt;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::Duration;

use crossbeam_channel::{Receiver, Sender};
use eframe::egui;
use media_encoder::{Comp, Frame, FrameSource};

use crate::renderer::RenderMode;

use super::App;

pub(super) struct SquarebobEncodeSource {
    width: usize,
    height: usize,
    frame_start: i32,
    frame_end: i32,
    fps: f32,
    request_tx: Sender<EncodeFrameRequest>,
    request_rx: Receiver<EncodeFrameRequest>,
    cancelled: AtomicBool,
}

impl SquarebobEncodeSource {
    fn new(width: u32, height: u32, frame_start: i32, frame_end: i32, fps: f32) -> Self {
        let (request_tx, request_rx) = crossbeam_channel::unbounded();
        Self {
            width: width as usize,
            height: height as usize,
            frame_start,
            frame_end: frame_end.max(frame_start),
            fps: fps.max(1.0),
            request_tx,
            request_rx,
            cancelled: AtomicBool::new(false),
        }
    }

    fn matches(&self, width: u32, height: u32, frame_start: i32, frame_end: i32, fps: f32) -> bool {
        self.width == width as usize
            && self.height == height as usize
            && self.frame_start == frame_start
            && self.frame_end == frame_end.max(frame_start)
            && (self.fps - fps.max(1.0)).abs() < f32::EPSILON
    }

    pub(super) fn try_next_request(&self) -> Option<EncodeFrameRequest> {
        self.request_rx.try_recv().ok()
    }

    pub(super) fn frame_time_seconds(&self, frame_idx: i32) -> f32 {
        (frame_idx - self.frame_start).max(0) as f32 / self.fps
    }

    pub(super) fn cancel(&self) {
        self.cancelled.store(true, Ordering::Relaxed);
    }
}

impl fmt::Display for SquarebobEncodeSource {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "Squarebob viewport")
    }
}

impl FrameSource for SquarebobEncodeSource {
    fn play_range(&self, _clamp_to_available: bool) -> (i32, i32) {
        (self.frame_start, self.frame_end)
    }

    fn get_frame(&self, frame_idx: i32, _blocking: bool) -> Option<Frame> {
        if frame_idx < self.frame_start
            || frame_idx > self.frame_end
            || self.cancelled.load(Ordering::Relaxed)
        {
            return None;
        }

        let (response_tx, response_rx) = crossbeam_channel::bounded(1);
        if self
            .request_tx
            .send(EncodeFrameRequest {
                frame_idx,
                response_tx,
            })
            .is_err()
        {
            return None;
        }

        loop {
            if self.cancelled.load(Ordering::Relaxed) {
                return None;
            }

            match response_rx.recv_timeout(Duration::from_millis(100)) {
                Ok(frame) => return frame,
                Err(crossbeam_channel::RecvTimeoutError::Timeout) => {}
                Err(crossbeam_channel::RecvTimeoutError::Disconnected) => return None,
            }
        }
    }
}

pub(super) struct EncodeFrameRequest {
    frame_idx: i32,
    response_tx: Sender<Option<Frame>>,
}

impl EncodeFrameRequest {
    fn frame_idx(&self) -> i32 {
        self.frame_idx
    }

    fn complete(self, frame: Option<Frame>) {
        let _ = self.response_tx.send(frame);
    }
}

impl App {
    pub(super) fn ui_encode_dialog_window(&mut self, ctx: &egui::Context) {
        if !self.show_encode_panel {
            return;
        }

        let was_encoding = self.encode_dialog.is_encoding;
        if !was_encoding {
            self.refresh_encode_source();
        }

        let project = media_encoder::Project;
        let active_comp = self.encode_source.clone();
        let keep_open = self
            .encode_dialog
            .render(ctx, &project, active_comp.as_ref());

        if !keep_open {
            self.cancel_encode_sequence_source();
            self.show_encode_panel = false;
            return;
        }

        if was_encoding && !self.encode_dialog.is_encoding {
            self.cancel_encode_sequence_source();
        }

        if !self.encode_dialog.is_encoding {
            self.refresh_encode_source();
        }
    }

    fn refresh_encode_source(&mut self) {
        if self.encode_dialog.is_encoding {
            return;
        }

        let (w, h) = self.last_render_size;
        if w == 0 || h == 0 {
            self.encode_source = None;
            self.encode_sequence_source = None;
            self.encode_source_size = (0, 0);
            return;
        }

        let frame_start = self.encode_dialog.frame_start;
        let frame_end = self.encode_dialog.frame_end.max(frame_start);
        let fps = self.encode_dialog.fps.max(1.0);

        if let Some(source) = &self.encode_sequence_source {
            if source.matches(w, h, frame_start, frame_end, fps) {
                self.encode_source_size = (w, h);
                return;
            }
        }

        let source = Arc::new(SquarebobEncodeSource::new(w, h, frame_start, frame_end, fps));
        let comp: Comp = source.clone();
        self.encode_sequence_source = Some(source);
        self.encode_source = Some(comp);
        self.encode_source_size = (w, h);
    }

    pub(super) fn handle_image_sequence(&mut self, ctx: &egui::Context) {
        if !self.encode_dialog.is_encoding {
            return;
        }

        let Some(source) = self.encode_sequence_source.clone() else {
            return;
        };

        ctx.request_repaint();

        if self.encode_active_frame.is_none() {
            if let Some(request) = source.try_next_request() {
                let frame_idx = request.frame_idx();
                if self.start_encode_frame(&source, frame_idx) {
                    self.encode_active_frame = Some(request);
                } else {
                    request.complete(None);
                }
            }
            return;
        }

        if !self.encode_current_frame_ready() {
            return;
        }

        let Some(request) = self.encode_active_frame.take() else {
            return;
        };

        let (width, height) = self.last_render_size;
        if width == 0 || height == 0 {
            request.complete(None);
            self.cancel_encode_sequence_source();
            return;
        }

        let pixels = self.capture_viewport(width, height);
        let expected_len = width as usize * height as usize * 4;
        if pixels.len() != expected_len {
            request.complete(None);
            self.cancel_encode_sequence_source();
            return;
        }

        request.complete(Some(Frame::rgba8(width as usize, height as usize, pixels)));
    }

    fn start_encode_frame(&mut self, source: &SquarebobEncodeSource, frame_idx: i32) -> bool {
        if self.display_root().is_none() || self.last_render_size == (0, 0) {
            return false;
        }

        if !self.encode_render_state_active {
            self.begin_encode_render_state();
        }

        let frame_seconds = source.frame_time_seconds(frame_idx);
        self.apply_encode_frame_time(frame_seconds);

        if self.render_mode == RenderMode::Mode3D {
            self.needs_render_3d = true;
            self.needs_layout = true;
            if let Some(renderer) = &mut self.renderer_3d {
                renderer.reset_pt_accumulation();
            }
        }

        true
    }

    fn encode_current_frame_ready(&self) -> bool {
        if self.render_mode != RenderMode::Mode3D {
            return true;
        }

        if self.last_render_frame_3d != self.frame_count {
            return false;
        }

        if !self.render_3d_opts.path_tracing {
            return true;
        }

        let target_samples = self.render_3d_opts.pt_samples.max(1);
        self.renderer_3d
            .as_ref()
            .map(|renderer| renderer.pt_frame_count() >= target_samples)
            .unwrap_or(false)
    }

    fn begin_encode_render_state(&mut self) {
        self.encode_render_state_active = true;
        self.encode_restore_render_mode = self.render_mode;
        self.encode_base_animation_time = self.render_3d_opts.animation_time;
        self.encode_base_env_time = self.render_3d_opts.env_time;
        self.encode_restore_animate = self.render_3d_opts.animate;
        self.encode_restore_env_animate = self.render_3d_opts.env_animate;

        self.render_3d_opts.animate = false;
        self.render_3d_opts.env_animate = false;
        self.last_anim_tick = None;
    }

    fn cancel_encode_sequence_source(&mut self) {
        if let Some(source) = &self.encode_sequence_source {
            source.cancel();
        }
        if let Some(request) = self.encode_active_frame.take() {
            request.complete(None);
        }
        self.restore_encode_render_state();
        self.encode_source = None;
        self.encode_sequence_source = None;
        self.encode_source_size = (0, 0);
    }

    fn restore_encode_render_state(&mut self) {
        if !self.encode_render_state_active {
            return;
        }

        self.render_mode = self.encode_restore_render_mode;
        self.render_3d_opts.animate = self.encode_restore_animate;
        self.render_3d_opts.env_animate = self.encode_restore_env_animate;
        self.encode_render_state_active = false;
        self.last_anim_tick = None;
    }

    fn apply_encode_frame_time(&mut self, frame_seconds: f32) {
        self.render_3d_opts.animation_time =
            self.encode_base_animation_time + frame_seconds * self.render_3d_opts.animation_speed;
        self.render_3d_opts.env_time = self.encode_base_env_time
            + frame_seconds * self.render_3d_opts.animation_speed * self.render_3d_opts.env_speed;
    }
}
