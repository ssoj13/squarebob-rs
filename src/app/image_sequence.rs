//! Encode dialog adapter for the reusable `media-encoder` crate.

use std::fmt;
use std::sync::Arc;

use eframe::egui;
use media_encoder::{Comp, Frame, FrameSource};

use super::App;

struct ViewportFrameSource {
    width: usize,
    height: usize,
    pixels: Arc<Vec<u8>>,
}

impl ViewportFrameSource {
    fn new(width: u32, height: u32, pixels: Vec<u8>) -> Self {
        Self {
            width: width as usize,
            height: height as usize,
            pixels: Arc::new(pixels),
        }
    }
}

impl fmt::Display for ViewportFrameSource {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "Dirstat viewport")
    }
}

impl FrameSource for ViewportFrameSource {
    fn play_range(&self, _clamp_to_available: bool) -> (i32, i32) {
        (0, 0)
    }

    fn get_frame(&self, frame_idx: i32, _blocking: bool) -> Option<Frame> {
        if frame_idx != 0 {
            return None;
        }

        Some(Frame::rgba8(
            self.width,
            self.height,
            self.pixels.as_ref().clone(),
        ))
    }
}

impl App {
    pub(super) fn ui_encode_dialog_window(&mut self, ctx: &egui::Context) {
        if !self.show_encode_panel {
            return;
        }

        self.refresh_encode_source();

        let project = media_encoder::Project::default();
        let active_comp = self.encode_source.clone();
        if !self
            .encode_dialog
            .render(ctx, &project, active_comp.as_ref())
        {
            self.show_encode_panel = false;
        }
    }

    fn refresh_encode_source(&mut self) {
        if self.encode_dialog.is_encoding {
            return;
        }

        let (w, h) = self.last_render_size;
        if w == 0 || h == 0 {
            self.encode_source = None;
            self.encode_source_size = (0, 0);
            return;
        }

        if self.encode_source.is_some() && self.encode_source_size == (w, h) {
            return;
        }

        let pixels = self.capture_viewport(w, h);
        let expected_len = w as usize * h as usize * 4;
        if pixels.len() != expected_len {
            self.encode_source = None;
            self.encode_source_size = (0, 0);
            return;
        }

        let source: Comp = Arc::new(ViewportFrameSource::new(w, h, pixels));
        self.encode_source = Some(source);
        self.encode_source_size = (w, h);
    }

    pub(super) fn handle_image_sequence(&mut self, _ctx: &egui::Context) {}
}
