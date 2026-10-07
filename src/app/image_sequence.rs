//! Encode dialog adapter for the reusable `media-encoder` crate.

use std::fmt;
use std::sync::Arc;
use std::time::Duration;

use crossbeam_channel::{Receiver, Sender};
use eframe::egui;
use media_encoder::{
    Comp, EncodeError, EncodeLaunchRequest, EncodeSessionToken, Frame, FrameSource,
};

use crate::renderer::{OrbitCamera, Render3DOptions, RenderBackend, RenderMode};

use super::App;

#[cfg(test)]
mod motion_tests;

/// UI-owned render inputs. Only this copy's animation clocks advance during export.
pub(super) struct FrozenRenderSession {
    pub(super) root: squarebob_core::DirEntry,
    pub(super) mode: RenderMode,
    pub(super) backend: RenderBackend,
    pub(super) options: treemap::TreeMapOptions,
    pub(super) viewport: render_core::Viewport,
    pub(super) camera: OrbitCamera,
    pub(super) render_options: Arc<Render3DOptions>,
    pub(super) selected_ids: std::collections::HashSet<u32>,
    pub(super) pipeline: color_pipeline::ColorPipeline,
    pub(super) display_light: media_encoder::hdr::DisplayLight,
    pub(super) extent: (u32, u32),
    base_animation_time: f32,
    base_env_time: f32,
    animate: bool,
    env_animate: bool,
    pub(super) error: Option<String>,
    pub(super) layout_dirty: bool,
    pub(super) render_dirty: bool,
    pub(super) renderer_3d: Option<render_3d::Renderer3D>,
    pub(super) renderer_2d: Option<treemap::GpuRenderer2D>,
}

impl FrozenRenderSession {
    fn advance_to(&mut self, seconds: f32) {
        let options = Arc::make_mut(&mut self.render_options);
        options.animation_time = self.base_animation_time
            + if self.animate {
                seconds * options.animation_speed
            } else {
                0.0
            };
        options.env_time = self.base_env_time
            + if self.animate && self.env_animate {
                seconds * options.animation_speed * options.env_speed
            } else {
                0.0
            };
    }
}

pub(super) struct SquarebobEncodeSource {
    width: usize,
    height: usize,
    frame_start: i32,
    frame_end: i32,
    fps: f32,
    output_encoding: media_encoder::hdr::PngEncoding,
    white_nits: f32,
    request_tx: Sender<EncodeFrameRequest>,
    request_rx: Receiver<EncodeFrameRequest>,
    session_token: EncodeSessionToken,
}

impl SquarebobEncodeSource {
    fn new(
        width: u32,
        height: u32,
        frame_start: i32,
        frame_end: i32,
        fps: f32,
        session_token: EncodeSessionToken,
    ) -> Self {
        let (request_tx, request_rx) = crossbeam_channel::unbounded();
        Self {
            width: width as usize,
            height: height as usize,
            frame_start,
            frame_end: frame_end.max(frame_start),
            fps: fps.max(1.0),
            output_encoding: media_encoder::hdr::PngEncoding::Sdr8,
            white_nits: 203.0,
            request_tx,
            request_rx,
            session_token,
        }
    }

    fn matches(
        &self,
        width: u32,
        height: u32,
        frame_start: i32,
        frame_end: i32,
        fps: f32,
        generation: u64,
    ) -> bool {
        self.session_token.generation() == generation
            && self.width == width as usize
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
        self.session_token.cancel();
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

    fn get_frame(&self, frame_idx: i32, _blocking: bool) -> Result<Frame, EncodeError> {
        if self.session_token.is_cancelled() {
            return Err(EncodeError::Cancelled);
        }
        if frame_idx < self.frame_start || frame_idx > self.frame_end {
            return Err(EncodeError::EncodeFrameFailed(format!(
                "Frame {frame_idx} outside {}..{}",
                self.frame_start, self.frame_end
            )));
        }

        let (response_tx, response_rx) = crossbeam_channel::bounded(1);
        if self
            .request_tx
            .send(EncodeFrameRequest {
                generation: self.session_token.generation(),
                frame_idx,
                response_tx,
            })
            .is_err()
        {
            return Err(EncodeError::Cancelled);
        }

        loop {
            if self.session_token.is_cancelled() {
                return Err(EncodeError::Cancelled);
            }

            match response_rx.recv_timeout(Duration::from_millis(100)) {
                Ok(frame) => return frame,
                Err(crossbeam_channel::RecvTimeoutError::Timeout) => {}
                Err(crossbeam_channel::RecvTimeoutError::Disconnected) => {
                    return Err(EncodeError::EncodeFrameFailed(
                        "Render frame response disconnected".into(),
                    ));
                }
            }
        }
    }
}

pub(super) struct EncodeFrameRequest {
    generation: u64,
    frame_idx: i32,
    response_tx: Sender<Result<Frame, EncodeError>>,
}

impl EncodeFrameRequest {
    fn belongs_to(&self, source: &SquarebobEncodeSource) -> bool {
        self.generation == source.session_token.generation()
    }

    fn frame_idx(&self) -> i32 {
        self.frame_idx
    }

    fn complete(self, frame: Result<Frame, EncodeError>) {
        let _ = self.response_tx.send(frame);
    }
}

impl App {
    pub(super) fn fail_encode_render(&mut self, error: String) {
        let Some(session) = &mut self.encode_render_session else {
            return;
        };
        let cause = session.error.get_or_insert(error).clone();
        session.renderer_3d = None;
        session.renderer_2d = None;
        if let Some(request) = self.encode_active_frame.take() {
            request.complete(Err(EncodeError::EncodeFrameFailed(cause.clone())));
        }
        if let Some(source) = &self.encode_sequence_source {
            while let Some(request) = source.try_next_request() {
                request.complete(Err(EncodeError::EncodeFrameFailed(cause.clone())));
            }
        }
    }

    fn freeze_encode_render_session(
        &self,
        encoding: media_encoder::hdr::PngEncoding,
        white_nits: f32,
    ) -> Result<FrozenRenderSession, String> {
        let root = self
            .display_root()
            .ok_or("No display tree is available")?
            .clone();
        let mut options = self.render_3d_opts.clone();
        if self.render_mode != RenderMode::Mode3D || !options.path_tracing {
            // PBR and 2D produce their own display canvas; OCIO is a PT display stage.
            options.color_pipeline.mode = color_pipeline::ColorMode::BuiltIn;
        }
        let (pipeline, display_light) =
            freeze_output_pipeline(&mut options.color_pipeline, encoding, white_nits)?;
        let animate = options.animate;
        let env_animate = options.env_animate;
        options.animate = false;
        options.env_animate = false;
        options.pt_auto_spp = false;
        options.pt_camera_snap = false;
        let mut camera = self.orbit_camera.clone();
        camera.cancel_animation();
        camera.stop_inertia();
        let mut viewport = self.viewport.clone();
        viewport.width = self.last_render_size.0;
        viewport.height = self.last_render_size.1;
        Ok(FrozenRenderSession {
            root,
            mode: self.render_mode,
            backend: self.render_backend,
            options: self.opts.clone(),
            viewport,
            camera,
            selected_ids: self.selected_3d_ids.clone(),
            pipeline,
            display_light,
            extent: self.last_render_size,
            base_animation_time: options.animation_time,
            base_env_time: options.env_time,
            animate,
            env_animate,
            render_options: Arc::new(options),
            error: None,
            layout_dirty: true,
            render_dirty: true,
            renderer_3d: None,
            renderer_2d: None,
        })
    }

    pub(super) fn ui_encode_dialog_window(&mut self, ctx: &egui::Context) {
        if !self.show_encode_panel {
            return;
        }

        let active_comp = self.encode_source.clone();
        let response = self.encode_dialog.render(ctx, active_comp.as_ref());

        if response.close {
            self.cancel_encode_sequence_source();
            self.show_encode_panel = false;
            return;
        }
        if let Some(request) = response.launch {
            self.launch_encode(request);
        }
    }

    /// Bind the host source and worker to the same post-edit launch event.
    pub(super) fn launch_encode(&mut self, request: EncodeLaunchRequest) {
        let (width, height) = self.last_render_size;
        if width == 0 || height == 0 || self.encode_dialog.is_encoding() {
            return;
        }
        let mut session =
            match self.freeze_encode_render_session(request.output_encoding, request.white_nits) {
                Ok(session) => session,
                Err(error) => {
                    self.encode_dialog.progress = Some(media_encoder::EncodeProgress {
                        current_frame: 0,
                        total_frames: 0,
                        stage: media_encoder::EncodeStage::Error(error),
                    });
                    return;
                }
            };
        let mut source = SquarebobEncodeSource::new(
            width,
            height,
            request.frame_start,
            request.frame_end,
            request.fps,
            request.session_token(),
        );
        source.output_encoding = request.output_encoding;
        source.white_nits = request.white_nits;
        let source = Arc::new(source);
        let comp: Comp = source.clone();
        if self
            .encode_dialog
            .start_encoding(&comp, &media_encoder::Project, request)
        {
            session.renderer_3d = self.renderer_3d.take();
            session.renderer_2d = self.renderer_2d_gpu.take();
            self.encode_render_session = Some(session);
            self.invalidate_encode_preview();
            self.encode_sequence_source = Some(source);
            self.encode_source = Some(comp);
            self.encode_source_size = (width, height);
        }
    }

    /// Own lifecycle polling at app-frame scope, independent of encoder UI visibility.
    pub(super) fn poll_encode_lifecycle(&mut self, ctx: &egui::Context) {
        let was_encoding = self.encode_dialog.is_encoding();
        self.encode_dialog.poll_encoding_state(ctx);
        if was_encoding && !self.encode_dialog.is_encoding() {
            self.cancel_encode_sequence_source();
        }
        if !self.encode_dialog.is_encoding() {
            self.refresh_encode_source();
        }
    }

    pub(super) fn refresh_encode_source(&mut self) {
        if self.encode_dialog.is_encoding() {
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
        let session_token = self.encode_dialog.session_token();
        let generation = session_token.generation();

        if let Some(source) = &self.encode_sequence_source
            && source.matches(w, h, frame_start, frame_end, fps, generation)
        {
            self.encode_source_size = (w, h);
            return;
        }

        let source = Arc::new(SquarebobEncodeSource::new(
            w,
            h,
            frame_start,
            frame_end,
            fps,
            session_token,
        ));
        let comp: Comp = source.clone();
        self.encode_sequence_source = Some(source);
        self.encode_source = Some(comp);
        self.encode_source_size = (w, h);
    }

    pub(super) fn handle_image_sequence(&mut self, ctx: &egui::Context) {
        if !self.encode_dialog.is_encoding() {
            return;
        }

        let Some(source) = self.encode_sequence_source.clone() else {
            return;
        };

        ctx.request_repaint();

        if self.encode_active_frame.is_none() {
            if let Some(request) = source.try_next_request() {
                if !request.belongs_to(&source) {
                    request.complete(Err(EncodeError::Cancelled));
                    return;
                }
                let frame_idx = request.frame_idx();
                match self.start_encode_frame(&source, frame_idx) {
                    Ok(()) => self.encode_active_frame = Some(request),
                    Err(error) => request.complete(Err(error)),
                }
            }
            return;
        }

        if let Err(error) = self.render_encode_frame_if_needed() {
            self.fail_encode_render(error);
            return;
        }

        if let Some(error) = self
            .encode_render_session
            .as_ref()
            .and_then(|session| session.error.clone())
        {
            if let Some(request) = self.encode_active_frame.take() {
                request.complete(Err(EncodeError::EncodeFrameFailed(error)));
            }
            return;
        }
        if !self.encode_current_frame_ready() {
            return;
        }
        let Some(request) = self.encode_active_frame.take() else {
            return;
        };
        let (width, height) = self.encode_source_size;
        if width == 0 || height == 0 {
            request.complete(Err(EncodeError::EncodeFrameFailed(
                "Render size is empty".into(),
            )));
            return;
        }

        let captured =
            if self.render_mode_for_frame() == RenderMode::Mode3D || source.output_encoding.hdr() {
                self.capture_display_light(width, height, source.white_nits)
            } else {
                self.capture_viewport(width, height).and_then(|pixels| {
                    Frame::rgba8(width as usize, height as usize, pixels).map_err(|e| e.to_string())
                })
            };
        self.handle_gpu_errors();
        let captured = if let Some(error) = self
            .encode_render_session
            .as_ref()
            .and_then(|session| session.error.clone())
        {
            Err(error)
        } else {
            captured
        };
        match captured {
            Ok(frame) => request.complete(Ok(frame)),
            Err(error) => {
                log::error!("Failed to capture encode frame: {error}");
                request.complete(Err(EncodeError::EncodeFrameFailed(error.clone())));
                self.fail_encode_render(error);
            }
        }
    }

    /// Service export even when the viewport dock is hidden. The same renderer,
    /// canvas and display composite are used by the visible viewport callback.
    fn render_encode_frame_if_needed(&mut self) -> Result<(), String> {
        let Some(session) = &mut self.encode_render_session else {
            return Err("Export render session is unavailable".into());
        };
        if let Some(error) = &session.error {
            return Err(error.clone());
        }
        let (width, height) = session.extent;
        if session.mode == RenderMode::Mode2D {
            if session.backend == RenderBackend::Gpu && session.renderer_2d.is_none() {
                let gpu = self
                    .gpu_context
                    .clone()
                    .ok_or("GPU renderer is unavailable")?;
                session.renderer_2d = Some(treemap::GpuRenderer2D::new(gpu));
            }
            return Ok(());
        }
        if self.last_render_frame_3d == self.frame_count {
            return Ok(());
        }
        if session.renderer_3d.is_none() {
            let gpu = self
                .gpu_context
                .clone()
                .ok_or("GPU renderer is unavailable")?;
            let mut renderer = render_3d::Renderer3D::new(gpu);
            if session.render_options.env_map_enabled
                && let Some(path) = &session.render_options.env_map_path
            {
                renderer
                    .load_env_map(path)
                    .map_err(|error| error.to_string())?;
            }
            session.renderer_3d = Some(renderer);
        }
        let renderer = session
            .renderer_3d
            .as_mut()
            .ok_or("3D renderer is unavailable")?;
        if session.layout_dirty {
            renderer.invalidate_instances();
            renderer.mark_pt_scene_dirty();
        }
        session
            .pipeline
            .validate_for_export(&session.render_options.color_pipeline)?;
        renderer
            .sync_color_lut(&mut session.pipeline)
            .map_err(|error| error.to_string())?;
        renderer.set_selected_ids(&session.selected_ids);
        renderer
            .render_to_view(
                &session.root,
                width,
                height,
                &session.camera,
                &session.render_options,
                &session.options,
                Some(&mut session.selected_ids),
            )
            .map_err(|error| error.to_string())?;
        let path_tracing = session.render_options.path_tracing;
        self.maybe_run_oidn_denoise(width, height);
        if path_tracing {
            let source = if self.oidn_display_is_denoised {
                self.oidn_denoiser
                    .as_ref()
                    .map(|denoiser| denoiser.result_texture())
            } else {
                None
            };
            let session = self
                .encode_render_session
                .as_mut()
                .ok_or("Export session ended during render")?;
            if let Some(renderer) = &mut session.renderer_3d {
                renderer
                    .composite_overlay(source, &session.render_options, &session.pipeline)
                    .map_err(|error| error.to_string())?;
            }
        }
        self.last_render_frame_3d = self.frame_count;
        self.last_render_size = (width, height);
        self.finish_render_input_frame();
        Ok(())
    }

    /// Adapt the existing display canvas to WarpBro's display-light export contract.
    /// No scene-linear tags are attached to PBR, overlays, or 2D code values.
    fn capture_display_light(
        &mut self,
        width: u32,
        height: u32,
        white_nits: f32,
    ) -> Result<Frame, String> {
        let encoded = if self.render_mode_for_frame() == RenderMode::Mode3D {
            let renderer = self
                .renderer_3d_for_frame_mut()
                .ok_or("3D renderer is unavailable")?;
            let bytes = renderer
                .readback_render_texture(true)
                .map_err(|e| e.to_string())?;
            let expected = (width as usize)
                .checked_mul(height as usize)
                .and_then(|n| n.checked_mul(8))
                .ok_or("Capture dimensions overflow")?;
            if bytes.len() != expected {
                return Err(format!(
                    "Invalid HDR readback: {} bytes, expected {expected}",
                    bytes.len()
                ));
            }
            bytes
                .chunks_exact(2)
                .map(|b| half::f16::from_bits(u16::from_le_bytes([b[0], b[1]])).to_f32())
                .collect::<Vec<_>>()
        } else {
            self.capture_viewport(width, height)?
                .into_iter()
                .map(|v| f32::from(v) / 255.0)
                .collect()
        };
        let kind = self
            .encode_render_session
            .as_ref()
            .ok_or("Export render session is unavailable")?
            .display_light;
        display_canvas_frame(width, height, &encoded, kind, white_nits)
    }

    fn start_encode_frame(
        &mut self,
        source: &SquarebobEncodeSource,
        frame_idx: i32,
    ) -> Result<(), EncodeError> {
        if self.render_root().is_none() || self.encode_render_session.is_none() {
            return Err(EncodeError::EncodeFrameFailed(
                "Render scene or viewport is unavailable".into(),
            ));
        }
        if let Some(error) = self
            .encode_render_session
            .as_ref()
            .and_then(|session| session.error.clone())
        {
            return Err(EncodeError::EncodeFrameFailed(error));
        }

        let frame_seconds = source.frame_time_seconds(frame_idx);
        self.apply_encode_frame_time(frame_seconds);

        if self.render_mode_for_frame() == RenderMode::Mode3D {
            if let Some(session) = &mut self.encode_render_session {
                session.render_dirty = true;
                session.layout_dirty = true;
            }
            if let Some(renderer) = self.renderer_3d_for_frame_mut() {
                renderer.reset_pt_accumulation();
            }
        }

        Ok(())
    }

    fn encode_current_frame_ready(&self) -> bool {
        if self.render_mode_for_frame() != RenderMode::Mode3D {
            return true;
        }

        if self.last_render_frame_3d != self.frame_count {
            return false;
        }

        let options = self.render_options_for_frame();
        if !options.path_tracing {
            return true;
        }

        let target_samples = options.pt_samples.max(1);
        self.renderer_3d_for_frame()
            .map(|renderer| renderer.pt_frame_count() >= target_samples)
            .unwrap_or(false)
    }

    fn cancel_encode_sequence_source(&mut self) {
        if let Some(source) = &self.encode_sequence_source {
            source.cancel();
        }
        if let Some(request) = self.encode_active_frame.take() {
            request.complete(Err(EncodeError::Cancelled));
        }
        self.restore_encode_render_state();
        self.encode_source = None;
        self.encode_sequence_source = None;
        self.encode_source_size = (0, 0);
    }

    fn restore_encode_render_state(&mut self) {
        if let Some(session) = self.encode_render_session.take() {
            self.renderer_3d = session.renderer_3d;
            self.renderer_2d_gpu = session.renderer_2d;
            if session.error.is_some() {
                if let (Some(render_state), Some(texture)) =
                    (&self.wgpu_render_state, self.render_texture_id.take())
                {
                    render_state.renderer.write().free_texture(&texture);
                }
                self.render_texture_id = None;
                self.last_render_size = (0, 0);
            }
            self.color_pipeline =
                color_pipeline::ColorPipeline::new(&self.render_3d_opts.color_pipeline);
            if let Some(renderer) = &mut self.renderer_3d {
                if self.render_3d_opts.env_map_enabled
                    && let Some(path) = &self.render_3d_opts.env_map_path
                    && let Err(error) = renderer.load_env_map(path)
                {
                    log::error!("Cannot restore authored environment: {error}");
                }
                renderer.mark_pt_env_dirty();
            }
            self.on_render_mode_changed(session.mode);
            self.invalidate_encode_preview();
        }
    }

    fn apply_encode_frame_time(&mut self, frame_seconds: f32) {
        if let Some(session) = &mut self.encode_render_session {
            session.advance_to(frame_seconds);
        }
    }
}

fn display_canvas_frame(
    width: u32,
    height: u32,
    encoded: &[f32],
    kind: media_encoder::hdr::DisplayLight,
    white_nits: f32,
) -> Result<Frame, String> {
    let expected = (width as usize)
        .checked_mul(height as usize)
        .and_then(|n| n.checked_mul(4))
        .ok_or("Capture dimensions overflow")?;
    if encoded.len() != expected {
        return Err(format!(
            "Display canvas has {} elements, expected {expected}",
            encoded.len()
        ));
    }
    let mut light: Vec<f32> = encoded
        .chunks_exact(4)
        .flat_map(|pixel| {
            media_encoder::hdr::display_encoded_to_linear([pixel[0], pixel[1], pixel[2], pixel[3]])
        })
        .collect();
    if matches!(kind, media_encoder::hdr::DisplayLight::Absolute { .. }) {
        for pixel in light.chunks_exact_mut(4) {
            for channel in &mut pixel[..3] {
                *channel *= white_nits / 100.0;
            }
        }
    }
    Frame::display_light(width as usize, height as usize, light, kind, white_nits)
}

fn freeze_output_pipeline(
    settings: &mut color_pipeline::ColorPipelineSettings,
    encoding: media_encoder::hdr::PngEncoding,
    white_nits: f32,
) -> Result<
    (
        color_pipeline::ColorPipeline,
        media_encoder::hdr::DisplayLight,
    ),
    String,
> {
    use color_pipeline::vfx_ocio::Encoding;
    use color_pipeline::{ColorMode, ColorPipeline};
    use egui_display::export::{OutputKind, OutputView, OutputViewEncoding, resolve_output_view};
    if !white_nits.is_finite() || white_nits <= 0.0 {
        return Err("Output reference white must be finite and positive".into());
    }
    settings.output_hdr = encoding.hdr();
    settings.reference_white_nits = white_nits;
    let mut pipeline = ColorPipeline::new(settings);
    if settings.mode == ColorMode::Ocio {
        if pipeline.active_config_source() != &settings.ocio_config {
            return Err(format!(
                "Requested OCIO config {:?} could not be loaded",
                settings.ocio_config
            ));
        }
        let names: Vec<_> = pipeline
            .available_displays()
            .into_iter()
            .flat_map(|display| {
                pipeline
                    .available_views(&display)
                    .into_iter()
                    .map(move |view| (display.clone(), view))
            })
            .collect();
        let candidates: Vec<_> = names
            .iter()
            .map(|(display, view)| OutputView {
                display,
                view,
                encoding: match pipeline.output_encoding(display, view) {
                    Encoding::Hdr => OutputViewEncoding::Hdr,
                    Encoding::Data => OutputViewEncoding::Data,
                    _ => OutputViewEncoding::Picture,
                },
            })
            .collect();
        // Normalize empty OCIO selectors against the config's offered names,
        // matching WarpBro's Ocio::resolve before applying its shared output policy.
        let current_display = if settings.ocio_display.is_empty() {
            candidates
                .iter()
                .find(|candidate| {
                    candidate.encoding != OutputViewEncoding::Data
                        && (encoding.hdr() || candidate.encoding != OutputViewEncoding::Hdr)
                })
                .map(|candidate| candidate.display)
        } else {
            Some(settings.ocio_display.as_str())
        };
        let current = current_display.and_then(|display| {
            let view = if settings.ocio_view.is_empty() {
                candidates
                    .iter()
                    .find(|candidate| {
                        candidate.display == display
                            && (encoding.hdr() || candidate.encoding != OutputViewEncoding::Hdr)
                    })
                    .map(|candidate| candidate.view)
            } else {
                Some(settings.ocio_view.as_str())
            };
            view.map(|view| (display, view))
        });
        let selected = resolve_output_view(
            current,
            OutputKind::from(encoding),
            white_nits,
            &candidates,
            |display, view| {
                let mut probe_settings = settings.clone();
                probe_settings.ocio_display = display.to_owned();
                probe_settings.ocio_view = view.to_owned();
                let probe = ColorPipeline::new(&probe_settings);
                probe.validate_for_export(&probe_settings).ok()?;
                media_encoder::hdr::measure_display_peak(
                    |pixels| probe.apply_cpu_to_surface_linear(pixels),
                    white_nits,
                )
                .ok()
            },
        )?;
        settings.ocio_display = selected.display.to_owned();
        settings.ocio_view = selected.view.to_owned();
        pipeline = ColorPipeline::new(settings);
    }
    pipeline.validate_for_export(settings)?;
    let kind = if settings.mode == ColorMode::Ocio
        && pipeline.output_encoding(&settings.ocio_display, &settings.ocio_view) == Encoding::Hdr
    {
        media_encoder::hdr::DisplayLight::Absolute {
            peak_nits: media_encoder::hdr::measure_display_peak(
                |pixels| pipeline.apply_cpu_to_surface_linear(pixels),
                white_nits,
            )?,
        }
    } else {
        media_encoder::hdr::DisplayLight::Relative
    };
    Ok((pipeline, kind))
}

#[cfg(test)]
mod tests {
    use super::*;
    use media_encoder::hdr::{DisplayLight, PngEncoding};

    fn author_scene() -> App {
        let mut app = App::default();
        let mut root = squarebob_core::DirEntry::new_dir("launch scene".into(), "launch".into());
        root.children.push(squarebob_core::DirEntry::new_file(
            "a.rs".into(),
            "launch/a.rs".into(),
            1024,
            "rs".into(),
            None,
        ));
        root.size = 1024;
        app.tree = Some(root);
        app.last_render_size = (64, 48);
        app.render_mode = RenderMode::Mode3D;
        app.render_3d_opts.path_tracing = false;
        app.render_3d_opts.env_map_enabled = false;
        app.render_3d_opts.animate = true;
        app.render_3d_opts.env_animate = true;
        app.render_3d_opts.animation_time = 7.0;
        app.render_3d_opts.env_time = 11.0;
        app.render_3d_opts.animation_speed = 2.0;
        app.render_3d_opts.env_speed = 3.0;
        app.orbit_camera
            .set_front_view_for_viewport(64.0, 48.0, 64.0 / 48.0);
        app.selected_3d_ids.insert(3);
        app
    }

    #[test]
    fn export_scene_camera_quality_and_timeline_are_frozen_at_launch() {
        let mut app = author_scene();
        let distance = app.orbit_camera.distance;
        let samples = app.render_3d_opts.pt_samples;
        app.encode_render_session = Some(
            app.freeze_encode_render_session(PngEncoding::Sdr8, 203.0)
                .unwrap(),
        );
        app.tree.as_mut().unwrap().children.clear();
        app.render_mode = RenderMode::Mode2D;
        app.orbit_camera.distance = distance * 10.0;
        app.render_3d_opts.pt_samples = samples + 100;
        app.render_3d_opts.animation_speed = 99.0;
        app.viewport.zoom = 9.0;
        app.last_render_size = (2048, 2048);
        app.selected_3d_ids.clear();
        app.apply_encode_frame_time(0.5);
        let session = app.encode_render_session.as_ref().unwrap();
        assert_eq!(session.root.children.len(), 1);
        assert_eq!(session.extent, (64, 48));
        assert_eq!(session.mode, RenderMode::Mode3D);
        assert_eq!(session.camera.distance, distance);
        assert_eq!(session.render_options.pt_samples, samples);
        assert_eq!(session.render_options.animation_time, 8.0);
        assert_eq!(session.render_options.env_time, 14.0);
        assert_eq!(session.viewport.zoom, 1.0);
        assert!(session.selected_ids.contains(&3));
        assert_eq!(app.render_3d_opts.animation_time, 7.0);
        assert_eq!(app.render_3d_opts.env_time, 11.0);
    }

    #[test]
    fn cancel_and_failed_session_preserve_current_authored_state() {
        for failure in [false, true] {
            let mut app = author_scene();
            app.encode_render_session = Some(
                app.freeze_encode_render_session(PngEncoding::Sdr8, 203.0)
                    .unwrap(),
            );
            app.apply_encode_frame_time(12.0);
            app.render_3d_opts.animation_time = 300.0;
            app.render_3d_opts.env_time = 400.0;
            app.render_3d_opts.animate = false;
            app.render_3d_opts.env_animate = true;
            app.render_mode = RenderMode::Mode2D;
            if failure {
                app.encode_render_session.as_mut().unwrap().error =
                    Some("actual render failure".into());
                app.restore_encode_render_state();
            } else {
                let (sender, receiver) = crossbeam_channel::bounded(1);
                app.encode_active_frame = Some(EncodeFrameRequest {
                    generation: 1,
                    frame_idx: 1,
                    response_tx: sender,
                });
                app.cancel_encode_sequence_source();
                assert!(matches!(
                    receiver.recv().unwrap(),
                    Err(EncodeError::Cancelled)
                ));
            }
            assert!(app.encode_render_session.is_none());
            assert_eq!(app.render_3d_opts.animation_time, 300.0);
            assert_eq!(app.render_3d_opts.env_time, 400.0);
            assert!(!app.render_3d_opts.animate);
            assert!(app.render_3d_opts.env_animate);
            assert_eq!(app.render_mode, RenderMode::Mode2D);
            assert!(app.needs_layout && app.needs_render_3d);
        }
    }

    #[test]
    fn authored_dirty_flags_and_display_changes_do_not_reset_job_inputs() {
        let mut app = author_scene();
        app.encode_render_session = Some(
            app.freeze_encode_render_session(PngEncoding::Sdr8, 80.0)
                .unwrap(),
        );
        app.finish_render_input_frame();
        app.needs_layout = true;
        app.needs_render_3d = true;
        app.render_3d_opts.color_pipeline.output_hdr = true;
        app.render_3d_opts.color_pipeline.reference_white_nits = 1000.0;
        app.encode_dialog.output_encoding = PngEncoding::Hlg;
        app.encode_dialog.white_nits = 700.0;
        assert!(!app.layout_dirty_for_frame());
        assert!(!app.render_dirty_for_frame());
        let frozen = &app.render_options_for_frame().color_pipeline;
        assert!(!frozen.output_hdr);
        assert_eq!(frozen.reference_white_nits, 80.0);
        app.finish_render_input_frame();
        assert!(app.needs_layout && app.needs_render_3d);
    }

    #[test]
    fn paused_timelines_remain_paused_for_fractional_rate_export() {
        let mut app = author_scene();
        app.render_3d_opts.animate = false;
        app.encode_render_session = Some(
            app.freeze_encode_render_session(PngEncoding::Sdr8, 203.0)
                .unwrap(),
        );
        app.apply_encode_frame_time(3.0 / (24000.0 / 1001.0));
        let options = app.render_options_for_frame();
        assert_eq!(options.animation_time, 7.0);
        assert_eq!(options.env_time, 11.0);
    }

    #[test]
    fn sdr_output_resolves_an_hdr_preview_view_independently_of_monitor() {
        let mut settings = color_pipeline::ColorPipelineSettings::default();
        let config = color_pipeline::vfx_ocio::builtin::default_config();
        let (display, view) = config
            .displays()
            .displays()
            .iter()
            .find_map(|display| {
                config
                    .get_views(display.name())
                    .into_iter()
                    .find_map(|view| {
                        let colorspace =
                            config.colorspace(view.effective_colorspace(display.name()))?;
                        (colorspace.encoding() == color_pipeline::vfx_ocio::Encoding::Hdr)
                            .then(|| (display.name().to_owned(), view.name().to_owned()))
                    })
            })
            .expect("shipped ACES HDR view");
        settings.ocio_display = display;
        settings.ocio_view = view;
        let mut resolved = Vec::new();
        for monitor_hdr in [false, true] {
            let mut frozen = settings.clone();
            frozen.output_hdr = monitor_hdr;
            frozen.reference_white_nits = if monitor_hdr { 1000.0 } else { 80.0 };
            let (pipeline, kind) =
                freeze_output_pipeline(&mut frozen, PngEncoding::Sdr8, 203.0).unwrap();
            pipeline.validate_for_export(&frozen).unwrap();
            assert_eq!(kind, DisplayLight::Relative);
            assert!(!frozen.output_hdr);
            assert_eq!(frozen.reference_white_nits, 203.0);
            assert_ne!(
                pipeline.output_encoding(&frozen.ocio_display, &frozen.ocio_view),
                color_pipeline::vfx_ocio::Encoding::Hdr
            );
            resolved.push((frozen.ocio_display, frozen.ocio_view));
        }
        assert_eq!(resolved[0], resolved[1]);
    }

    #[test]
    fn export_rejects_requested_config_and_custom_lut_fallbacks() {
        let mut settings = color_pipeline::ColorPipelineSettings::default();
        settings.ocio_config = color_pipeline::ConfigSource::External(
            std::env::temp_dir().join(format!("squarebob-missing-{}.ocio", uuid::Uuid::new_v4())),
        );
        let error = match freeze_output_pipeline(&mut settings, PngEncoding::Sdr8, 203.0) {
            Err(error) => error,
            Ok(_) => panic!("missing requested config was replaced silently"),
        };
        assert!(error.contains("could not be loaded"));
        settings.ocio_config = color_pipeline::ConfigSource::BuiltIn;
        settings.ocio_custom_lut = Some(
            std::env::temp_dir().join(format!("squarebob-missing-{}.cube", uuid::Uuid::new_v4())),
        );
        let error = match freeze_output_pipeline(&mut settings, PngEncoding::Sdr8, 203.0) {
            Err(error) => error,
            Ok(_) => panic!("missing requested LUT was replaced silently"),
        };
        assert!(error.contains("Custom LUT"));
    }

    #[test]
    fn uncaptured_gpu_error_reaches_pending_source_request_and_blocks_further_render() {
        let mut app = author_scene();
        app.encode_render_session = Some(
            app.freeze_encode_render_session(PngEncoding::Sdr8, 203.0)
                .unwrap(),
        );
        let source = Arc::new(SquarebobEncodeSource::new(
            64,
            48,
            41,
            42,
            24000.0 / 1001.0,
            app.encode_dialog.session_token(),
        ));
        app.encode_sequence_source = Some(source.clone());
        let pending_source = source.clone();
        let pending = std::thread::spawn(move || pending_source.get_frame(41, true));
        app.encode_active_frame = Some(
            source
                .request_rx
                .recv_timeout(Duration::from_secs(2))
                .unwrap(),
        );
        let cause =
            "wgpu uncaptured error: Validation Error: texture format incompatible with pipeline";
        app.wgpu_error_tx.send(cause.into()).unwrap();
        app.handle_gpu_errors();
        assert!(
            matches!(pending.join().unwrap(), Err(EncodeError::EncodeFrameFailed(message)) if message == cause)
        );
        assert_eq!(
            app.encode_render_session.as_ref().unwrap().error.as_deref(),
            Some(cause)
        );
        assert!(app.encode_active_frame.is_none());
        assert!(
            matches!(app.start_encode_frame(&source, 42), Err(EncodeError::EncodeFrameFailed(message)) if message == cause)
        );
        assert_eq!(app.render_encode_frame_if_needed().unwrap_err(), cause);
        assert!(!source.session_token.is_cancelled());
        app.render_3d_opts.animation_time = 900.0;
        app.restore_encode_render_state();
        assert_eq!(app.render_3d_opts.animation_time, 900.0);
        assert!(app.encode_render_session.is_none());
    }

    #[test]
    fn float_sdr_canvas_preserves_sub_byte_precision_highlights_and_alpha() {
        let linear = [0.123456, 0.003123, 4.5, 0.375];
        let encoded = [
            egui_display::transfer::oetf(linear[0]),
            egui_display::transfer::oetf(linear[1]),
            egui_display::transfer::oetf(linear[2]),
            linear[3],
        ];
        let frame = display_canvas_frame(1, 1, &encoded, DisplayLight::Relative, 203.0).unwrap();
        let (pixels, kind, white) = frame.hdr_light().unwrap();
        assert_eq!(kind, DisplayLight::Relative);
        assert_eq!(white, 203.0);
        for (actual, expected) in pixels.iter().zip(linear) {
            assert!(
                (actual - expected).abs() < 0.000001,
                "{actual} != {expected}"
            );
        }
        assert!(display_canvas_frame(1, 1, &encoded[..3], kind, white).is_err());
        let absolute = display_canvas_frame(
            1,
            1,
            &encoded,
            DisplayLight::Absolute { peak_nits: 1000.0 },
            203.0,
        )
        .unwrap();
        let (pixels, _, _) = absolute.hdr_light().unwrap();
        assert!((pixels[0] - linear[0] * 2.03).abs() < 0.000001);
        assert_eq!(pixels[3], linear[3]);
    }

    #[test]
    #[ignore = "requires a GPU"]
    fn frozen_gpu_capture_is_unchanged_after_authored_scene_and_camera_edits() {
        fn render_job(app: &mut App) -> Frame {
            let session = app.encode_render_session.as_mut().unwrap();
            let renderer = session.renderer_3d.as_mut().unwrap();
            renderer
                .render_to_view(
                    &session.root,
                    session.extent.0,
                    session.extent.1,
                    &session.camera,
                    &session.render_options,
                    &session.options,
                    Some(&mut session.selected_ids),
                )
                .unwrap();
            app.capture_display_light(64, 48, 203.0).unwrap()
        }
        let gpu = std::sync::Arc::new(render_core::gpu::GpuContext::new().expect("GPU"));
        let mut app = author_scene();
        app.render_3d_opts.hash_effect = crate::renderer::HashTransformEffect::None;
        app.renderer_3d = Some(render_3d::Renderer3D::new(gpu));
        let mut session = app
            .freeze_encode_render_session(PngEncoding::Sdr8, 203.0)
            .unwrap();
        session.renderer_3d = app.renderer_3d.take();
        app.encode_render_session = Some(session);
        assert!(app.renderer_3d.is_none());
        let first = render_job(&mut app);
        app.tree.as_mut().unwrap().children.clear();
        app.orbit_camera.distance *= 10.0;
        app.render_3d_opts.background_color = [1.0, 0.0, 1.0];
        app.render_mode = RenderMode::Mode2D;
        app.on_render_mode_changed(RenderMode::Mode3D);
        let second = render_job(&mut app);
        assert_eq!(first.hdr_light().unwrap().0, second.hdr_light().unwrap().0);
        app.restore_encode_render_state();
        assert!(app.renderer_3d.is_some());
        assert_eq!(app.render_mode, RenderMode::Mode2D);
    }
}
