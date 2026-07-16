//! Encoding dialog UI
//!
//! Provides dialog for configuring and running video encoding.

use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{Receiver, channel};
use std::thread::JoinHandle;

use eframe::egui;
use egui_phosphor::regular as icons;
use log::info;

use crate::dialogs::encode::{
    ChannelMode, CodecSettings, Container, EncodeError, EncodeProgress, EncodeStage,
    EncoderSettings, ExportMode, ExrCompression, OutputBitDepth, ProResProfile, SequenceFormat,
    SequenceSettings, TiffCompression, VideoCodec, encode_comp, encode_image_sequence,
};
use crate::progress::ProgressBar;
use crate::source::{Comp, Project};

/// Cancellation identity shared by one encoder worker and its frame source.
#[derive(Clone)]
pub struct EncodeSessionToken {
    generation: u64,
    cancelled: Arc<AtomicBool>,
}

impl EncodeSessionToken {
    fn new(generation: u64) -> Self {
        Self {
            generation,
            cancelled: Arc::new(AtomicBool::new(false)),
        }
    }

    pub fn generation(&self) -> u64 {
        self.generation
    }

    pub fn is_cancelled(&self) -> bool {
        self.cancelled.load(Ordering::Acquire)
    }

    pub fn cancel(&self) {
        self.cancelled.store(true, Ordering::Release);
    }

    fn flag(&self) -> Arc<AtomicBool> {
        Arc::clone(&self.cancelled)
    }
}

struct EncodeSession {
    token: EncodeSessionToken,
    progress_rx: Receiver<EncodeProgress>,
    worker: JoinHandle<Result<(), EncodeError>>,
}

enum EncodeLifecycle {
    Idle(EncodeSessionToken),
    Running(EncodeSession),
    Cancelling(EncodeSession),
    Finishing(EncodeSession),
}

impl EncodeLifecycle {
    fn session(&self) -> Option<&EncodeSession> {
        match self {
            Self::Idle(_) => None,
            Self::Running(session) | Self::Cancelling(session) | Self::Finishing(session) => {
                Some(session)
            }
        }
    }

    fn is_cancelling(&self) -> bool {
        matches!(self, Self::Cancelling(_))
    }
}

/// Encoding dialog state
pub struct EncodeDialog {
    /// Output path and container settings
    pub output_path: PathBuf,
    pub container: Container,
    pub fps: f32,
    pub frame_start: i32,
    pub frame_end: i32,

    /// Currently selected codec tab
    pub selected_codec: VideoCodec,

    /// Per-codec settings
    pub codec_settings: CodecSettings,

    /// Current encoding progress.
    pub progress: Option<EncodeProgress>,

    /// Single owner for the worker, progress receiver, generation, and cancellation token.
    lifecycle: EncodeLifecycle,

    /// Progress bar widget
    progress_bar: ProgressBar,

    /// Tonemapping mode for HDRâ†’LDR conversion
    pub tonemap_mode: crate::frame::TonemapMode,

    /// Export mode (Video or Sequence)
    pub export_mode: ExportMode,

    /// Image sequence settings
    pub sequence_settings: SequenceSettings,
}

impl EncodeDialog {
    /// Increment the last number in filename
    /// Examples: aaa001.mp4 -> aaa002.mp4, test999.mp4 -> test1000.mp4
    fn increment_filename(&mut self) {
        let file_stem = self
            .output_path
            .file_stem()
            .and_then(|s| s.to_str())
            .unwrap_or("output");

        let extension = self
            .output_path
            .extension()
            .and_then(|s| s.to_str())
            .unwrap_or("mp4");

        // Find last number in filename using regex-like approach
        let mut last_num_start = None;
        let mut last_num_end = None;
        let mut in_number = false;

        for (i, c) in file_stem.chars().enumerate() {
            if c.is_ascii_digit() {
                if !in_number {
                    last_num_start = Some(i);
                    in_number = true;
                }
                last_num_end = Some(i + 1);
            } else {
                in_number = false;
            }
        }

        let new_stem = if let (Some(start), Some(end)) = (last_num_start, last_num_end) {
            let prefix = &file_stem[..start];
            let num_str = &file_stem[start..end];
            let suffix = &file_stem[end..];

            // Parse number and increment
            if let Ok(num) = num_str.parse::<u32>() {
                let new_num = num + 1;
                let old_width = num_str.len();

                // Calculate how many digits the new number has (integer-based for precision)
                let new_num_digits = match new_num {
                    0 => 1,
                    n => {
                        let mut count = 0;
                        let mut val = n;
                        while val > 0 {
                            count += 1;
                            val /= 10;
                        }
                        count
                    }
                };

                // Use original width if new number fits, otherwise use natural width
                let width = old_width.max(new_num_digits);

                format!("{}{:0width$}{}", prefix, new_num, suffix, width = width)
            } else {
                // If parse fails, just append 001
                format!("{}001", file_stem)
            }
        } else {
            // No number found, append 001
            format!("{}001", file_stem)
        };

        // Update path with new filename
        if let Some(parent) = self.output_path.parent() {
            self.output_path = parent.join(format!("{}.{}", new_stem, extension));
        } else {
            self.output_path = PathBuf::from(format!("{}.{}", new_stem, extension));
        }
    }

    /// Load dialog state from AppSettings (called when opening dialog)
    pub fn load_from_settings(settings: &crate::dialogs::encode::EncodeDialogSettings) -> Self {
        log::trace!("========== LOADING ENCODE DIALOG SETTINGS ==========");
        log::trace!("  Output: {}", settings.output_path.display());
        log::trace!(
            "  Container: {:?}, FPS: {}, Codec: {:?}",
            settings.container,
            settings.fps,
            settings.selected_codec
        );
        log::trace!(
            "  H.264: mode={:?}, value={}, preset={}, profile={}",
            settings.codec_settings.h264.quality_mode,
            settings.codec_settings.h264.quality_value,
            settings.codec_settings.h264.preset,
            settings.codec_settings.h264.profile
        );
        log::trace!(
            "  H.265: mode={:?}, value={}, preset={}, profile={}",
            settings.codec_settings.h265.quality_mode,
            settings.codec_settings.h265.quality_value,
            settings.codec_settings.h265.preset,
            settings.codec_settings.h265.profile
        );
        log::trace!(
            "  ProRes: profile={:?}",
            settings.codec_settings.prores.profile
        );
        log::trace!(
            "  AV1: mode={:?}, value={}",
            settings.codec_settings.av1.quality_mode,
            settings.codec_settings.av1.quality_value
        );
        log::trace!("  Tonemap: {:?}", settings.tonemap_mode);
        log::trace!("  ExportMode: {:?}", settings.export_mode);
        log::trace!(
            "  Sequence: format={:?}, channels={:?}, depth={:?}",
            settings.sequence_settings.format,
            settings.sequence_settings.channels,
            settings.sequence_settings.bit_depth
        );

        Self {
            output_path: settings.output_path.clone(),
            container: settings.container,
            fps: settings.fps,
            frame_start: settings.frame_start,
            frame_end: settings.frame_end.max(settings.frame_start),
            selected_codec: settings.selected_codec,
            codec_settings: settings.codec_settings.clone(),
            progress: None,
            lifecycle: EncodeLifecycle::Idle(EncodeSessionToken::new(1)),
            progress_bar: ProgressBar::new(400.0, 20.0),
            tonemap_mode: settings.tonemap_mode,
            export_mode: settings.export_mode,
            sequence_settings: settings.sequence_settings.clone(),
        }
    }

    /// Save current dialog state to AppSettings (called when closing dialog or starting encode)
    pub fn save_to_settings(&self) -> crate::dialogs::encode::EncodeDialogSettings {
        log::trace!("========== SAVING ENCODE DIALOG SETTINGS ==========");
        log::trace!("  Output: {}", self.output_path.display());
        log::trace!(
            "  Container: {:?}, FPS: {}, Codec: {:?}",
            self.container,
            self.fps,
            self.selected_codec
        );
        log::trace!(
            "  H.264: mode={:?}, value={}, preset={}, profile={}",
            self.codec_settings.h264.quality_mode,
            self.codec_settings.h264.quality_value,
            self.codec_settings.h264.preset,
            self.codec_settings.h264.profile
        );
        log::trace!(
            "  H.265: mode={:?}, value={}, preset={}, profile={}",
            self.codec_settings.h265.quality_mode,
            self.codec_settings.h265.quality_value,
            self.codec_settings.h265.preset,
            self.codec_settings.h265.profile
        );
        log::trace!("  ProRes: profile={:?}", self.codec_settings.prores.profile);
        log::trace!(
            "  AV1: mode={:?}, value={}",
            self.codec_settings.av1.quality_mode,
            self.codec_settings.av1.quality_value
        );
        log::trace!("  Tonemap: {:?}", self.tonemap_mode);
        log::trace!("  ExportMode: {:?}", self.export_mode);
        log::trace!(
            "  Sequence: format={:?}, channels={:?}, depth={:?}",
            self.sequence_settings.format,
            self.sequence_settings.channels,
            self.sequence_settings.bit_depth
        );

        crate::dialogs::encode::EncodeDialogSettings {
            output_path: self.output_path.clone(),
            container: self.container,
            fps: self.fps,
            frame_start: self.frame_start,
            frame_end: self.frame_end.max(self.frame_start),
            selected_codec: self.selected_codec,
            tonemap_mode: self.tonemap_mode,
            codec_settings: self.codec_settings.clone(),
            export_mode: self.export_mode,
            sequence_settings: self.sequence_settings.clone(),
        }
    }

    /// Build EncoderSettings from current UI state
    pub fn build_encoder_settings(&self) -> EncoderSettings {
        // self.output_path is already normalized (kept in sync with container changes)
        let (quality_mode, quality_value, preset, profile, prores_profile) =
            match self.selected_codec {
                VideoCodec::H264 => (
                    self.codec_settings.h264.quality_mode,
                    self.codec_settings.h264.quality_value,
                    Some(self.codec_settings.h264.preset.clone()),
                    Some(self.codec_settings.h264.profile.clone()),
                    None,
                ),
                VideoCodec::H265 => (
                    self.codec_settings.h265.quality_mode,
                    self.codec_settings.h265.quality_value,
                    Some(self.codec_settings.h265.preset.clone()),
                    Some(self.codec_settings.h265.profile.clone()),
                    None,
                ),
                VideoCodec::AV1 => (
                    self.codec_settings.av1.quality_mode,
                    self.codec_settings.av1.quality_value,
                    None,
                    None,
                    None,
                ),
                VideoCodec::ProRes => (
                    crate::dialogs::encode::QualityMode::CRF,
                    0,
                    None,
                    None,
                    Some(self.codec_settings.prores.profile),
                ),
            };

        EncoderSettings {
            output_path: self.output_path.clone(),
            container: self.container,
            codec: self.selected_codec,
            quality_mode,
            quality_value,
            fps: self.fps,
            preset,
            profile,
            prores_profile,
            tonemap_mode: self.tonemap_mode,
        }
    }

    /// True until the active generation has terminated and been joined.
    pub fn is_encoding(&self) -> bool {
        !matches!(self.lifecycle, EncodeLifecycle::Idle(_))
    }

    /// Token for the next/active generation. Frame sources must share it.
    pub fn session_token(&self) -> EncodeSessionToken {
        match &self.lifecycle {
            EncodeLifecycle::Idle(token) => token.clone(),
            EncodeLifecycle::Running(session)
            | EncodeLifecycle::Cancelling(session)
            | EncodeLifecycle::Finishing(session) => session.token.clone(),
        }
    }

    /// Stop encoding (public interface for ESC key handling).
    pub fn stop_encoding(&mut self) {
        self.stop_encoding_keep_window();
    }

    /// Render the encode dialog
    ///
    /// Returns: true if dialog should remain open, false if closed
    pub fn render(
        &mut self,
        ctx: &egui::Context,
        project: &Project,
        active_comp: Option<&Comp>,
    ) -> bool {
        let window_title = match self.export_mode {
            ExportMode::Video => "Video Encoder",
            ExportMode::Sequence => "Image Sequence Export",
        };
        let mut should_close = false;
        egui::Window::new(window_title)
            .id(egui::Id::new("encode_dialog"))
            .resizable(false)
            .collapsible(false)
            .show(ctx, |ui| {
                ui.set_width(600.0);
                if self.render_inline(ui, project, active_comp, true) {
                    should_close = true;
                }
            });

        // Return true if window should stay open
        !should_close
    }

    /// Drain progress and reap a finished worker without blocking the UI.
    pub fn poll_encoding_state(&mut self, ctx: &egui::Context) {
        let updates: Vec<_> = self
            .lifecycle
            .session()
            .map(|session| session.progress_rx.try_iter().collect())
            .unwrap_or_default();
        for progress in updates {
            self.progress = Some(progress);
        }

        let terminal_progress = matches!(
            self.progress.as_ref().map(|progress| &progress.stage),
            Some(EncodeStage::Complete | EncodeStage::Error(_))
        );
        if terminal_progress {
            let state = self.take_lifecycle();
            self.lifecycle = match state {
                EncodeLifecycle::Running(session) => EncodeLifecycle::Finishing(session),
                other => other,
            };
        }

        let worker_finished = self
            .lifecycle
            .session()
            .is_some_and(|session| session.worker.is_finished());
        if worker_finished {
            self.reap_finished_session();
        }

        if self.is_encoding() {
            ctx.request_repaint();
        }
    }

    fn take_lifecycle(&mut self) -> EncodeLifecycle {
        let token = self.session_token();
        std::mem::replace(&mut self.lifecycle, EncodeLifecycle::Idle(token))
    }

    fn reap_finished_session(&mut self) {
        let state = self.take_lifecycle();
        let (session, was_cancelling) = match state {
            EncodeLifecycle::Running(session) | EncodeLifecycle::Finishing(session) => {
                (session, false)
            }
            EncodeLifecycle::Cancelling(session) => (session, true),
            idle @ EncodeLifecycle::Idle(_) => {
                self.lifecycle = idle;
                return;
            }
        };

        let generation = session.token.generation();
        let result = session.worker.join();
        let next_generation = generation.saturating_add(1);
        self.lifecycle = EncodeLifecycle::Idle(EncodeSessionToken::new(next_generation));

        match result {
            Ok(Ok(())) => {
                if !was_cancelling {
                    info!("Encoding generation {} completed successfully", generation);
                }
            }
            Ok(Err(EncodeError::Cancelled)) if was_cancelling => {
                info!("Encoding generation {} cancelled", generation);
            }
            Ok(Err(error)) => {
                info!("Encoding generation {} failed: {}", generation, error);
                self.set_terminal_error(error.to_string());
            }
            Err(payload) => {
                let message = format!("Encoder generation {} panicked: {:?}", generation, payload);
                info!("{}", message);
                self.set_terminal_error(message);
            }
        }
    }

    fn set_terminal_error(&mut self, message: String) {
        let (current_frame, total_frames) = self
            .progress
            .as_ref()
            .map(|progress| (progress.current_frame, progress.total_frames))
            .unwrap_or((0, 0));
        self.progress = Some(EncodeProgress {
            current_frame,
            total_frames,
            stage: EncodeStage::Error(message),
        });
    }

    /// Render the encoder UI body directly into `ui`. Use this when
    /// embedding the encoder inline inside another panel (e.g., the
    /// Settings → Output section). The window-presented
    /// [`Self::render`] is a thin wrapper around this; both share
    /// behaviour.
    ///
    /// `with_close_button` controls the bottom button row:
    /// * `true` (window mode): renders the [Close] [Encode/Stop] pair
    ///   side-by-side. The Close button signals "close window" via the
    ///   `bool` return.
    /// * `false` (inline mode): suppresses Close (the section has its
    ///   own collapse) and stretches Encode/Stop to fill the row width.
    ///
    /// Returns `true` if the user requested a close (only meaningful
    /// when `with_close_button` is `true`). Width is not forced here —
    /// the inline section uses whatever width the parent `Ui` provides.
    pub fn render_inline(
        &mut self,
        ui: &mut egui::Ui,
        project: &Project,
        active_comp: Option<&Comp>,
        with_close_button: bool,
    ) -> bool {
        let mut should_close = false;
        {
            // === Output Path ===
            ui.horizontal(|ui| {
                ui.label("Output:");
                ui.add_enabled_ui(!self.is_encoding(), |ui| {
                    let path_str = self.output_path.display().to_string();
                    let mut edit_path = path_str.clone();
                    if ui.text_edit_singleline(&mut edit_path).changed() {
                        self.output_path = PathBuf::from(edit_path);
                    }

                    // Increment filename button
                    if ui
                        .button("+")
                        .on_hover_text(
                            "Increment number in filename (e.g., file001.mp4 -> file002.mp4)",
                        )
                        .clicked()
                    {
                        self.increment_filename();
                    }

                    if ui.button("Browse").clicked()
                        && let Some(path) = rfd::FileDialog::new()
                            .set_file_name("output.mp4")
                            .save_file()
                    {
                        self.output_path = path;
                    }
                });
            });

            // === Framerate ===
            ui.horizontal(|ui| {
                ui.label("Framerate:");
                ui.add_enabled_ui(!self.is_encoding(), |ui| {
                    ui.add(egui::Slider::new(&mut self.fps, 1.0..=960.0).text("fps"));
                });
            });

            ui.separator();

            // === Export Mode Tabs (Video / Sequence) ===
            //
            // Smart-rename rules — keep the user from manually
            // adding / removing the `####` frame token every
            // time they flip the mode:
            //   Video → strip any padding token from the stem
            //           (`####`, `%04d`, `@@@@`, with or without
            //           a leading `.`), then set the container
            //           extension.
            //   Sequence → append `.####` to the stem (skipped
            //              if any padding token is already
            //              there), then set the image-format
            //              extension.
            ui.horizontal(|ui| {
                ui.add_enabled_ui(!self.is_encoding(), |ui| {
                    let video_btn = egui::Button::new("Video")
                        .selected(self.export_mode == ExportMode::Video)
                        .min_size(egui::vec2(80.0, 0.0));
                    if ui.add(video_btn).clicked() {
                        self.export_mode = ExportMode::Video;
                        let stem = self
                            .output_path
                            .file_stem()
                            .and_then(|s| s.to_str())
                            .unwrap_or("output");
                        let clean = strip_frame_token(stem);
                        let ext = self.container.extension();
                        self.output_path = rebuild_path(self.output_path.parent(), &clean, ext);
                    }

                    let seq_btn = egui::Button::new("Sequence")
                        .selected(self.export_mode == ExportMode::Sequence)
                        .min_size(egui::vec2(80.0, 0.0));
                    if ui.add(seq_btn).clicked() {
                        self.export_mode = ExportMode::Sequence;
                        let stem = self
                            .output_path
                            .file_stem()
                            .and_then(|s| s.to_str())
                            .unwrap_or("frame");
                        let with_token = ensure_frame_token(stem);
                        let ext = self.sequence_settings.format.extension();
                        self.output_path =
                            rebuild_path(self.output_path.parent(), &with_token, ext);
                    }
                });
            });

            ui.add_space(4.0);

            // === Codec/Format Tabs based on mode ===
            match self.export_mode {
                ExportMode::Video => {
                    // Video codec tabs
                    ui.horizontal(|ui| {
                        ui.add_enabled_ui(!self.is_encoding(), |ui| {
                            for codec in VideoCodec::all() {
                                let is_available = codec.is_available();
                                let is_selected = self.selected_codec == *codec;

                                ui.add_enabled_ui(is_available, |ui| {
                                    let button = egui::Button::new(codec.to_string())
                                        .selected(is_selected)
                                        .min_size(egui::vec2(90.0, 0.0));

                                    if ui.add(button).clicked() {
                                        self.selected_codec = *codec;
                                        let preferred_container = codec.preferred_container();
                                        self.container = preferred_container;
                                        self.output_path
                                            .set_extension(preferred_container.extension());
                                    }
                                });

                                if !is_available {
                                    ui.label(icons::X)
                                        .on_hover_text(format!("{} encoder not available", codec));
                                }
                            }
                        });
                    });

                    ui.separator();
                    ui.add_space(8.0);

                    // Per-Codec Settings
                    ui.add_enabled_ui(!self.is_encoding(), |ui| match self.selected_codec {
                        VideoCodec::H264 => self.render_h264_settings(ui),
                        VideoCodec::H265 => self.render_h265_settings(ui),
                        VideoCodec::AV1 => self.render_av1_settings(ui),
                        VideoCodec::ProRes => self.render_prores_settings(ui),
                    });
                }
                ExportMode::Sequence => {
                    let caps = self.sequence_settings.format.capabilities();

                    // === Common settings (above format buttons) ===
                    ui.add_enabled_ui(!self.is_encoding(), |ui| {
                            // Channels (RGB/RGBA)
                            ui.horizontal(|ui| {
                                ui.label("Channels:");
                                for mode in ChannelMode::all() {
                                    let enabled = caps.supports_alpha || *mode == ChannelMode::Rgb;
                                    ui.add_enabled_ui(enabled, |ui| {
                                        if ui.radio_value(
                                            &mut self.sequence_settings.channels,
                                            *mode,
                                            mode.to_string(),
                                        ).changed() {
                                            self.sequence_settings.validate();
                                        }
                                    });
                                }
                                if !caps.supports_alpha {
                                    ui.label("(no alpha)").on_hover_text("This format doesn't support alpha channel");
                                }
                            });

                            // Bit Depth
                            ui.horizontal(|ui| {
                                ui.label("Bit Depth:");
                                for depth in OutputBitDepth::all() {
                                    let supported = self.sequence_settings.format.supports_depth(*depth);
                                    ui.add_enabled_ui(supported, |ui| {
                                        if ui.radio_value(
                                            &mut self.sequence_settings.bit_depth,
                                            *depth,
                                            depth.to_string(),
                                        ).changed() {
                                            self.sequence_settings.validate();
                                        }
                                    });
                                }
                            });

                            // Tonemapping
                            ui.horizontal(|ui| {
                                let needs_tonemap_hint = !caps.is_hdr;
                                ui.checkbox(&mut self.sequence_settings.apply_tonemap, "Tonemapping");
                                if self.sequence_settings.apply_tonemap {
                                    egui::ComboBox::from_id_salt("seq_tonemap")
                                        .selected_text(format!("{:?}", self.sequence_settings.tonemap_mode))
                                        .show_ui(ui, |ui| {
                                            ui.selectable_value(
                                                &mut self.sequence_settings.tonemap_mode,
                                                crate::frame::TonemapMode::ACES,
                                                "ACES",
                                            );
                                            ui.selectable_value(
                                                &mut self.sequence_settings.tonemap_mode,
                                                crate::frame::TonemapMode::Reinhard,
                                                "Reinhard",
                                            );
                                            ui.selectable_value(
                                                &mut self.sequence_settings.tonemap_mode,
                                                crate::frame::TonemapMode::Clamp,
                                                "Clamp",
                                            );
                                        });
                                }
                                if needs_tonemap_hint && !self.sequence_settings.apply_tonemap {
                                    ui.label("(auto for HDR input)").on_hover_text(
                                        "HDR frames will be automatically tonemapped for this LDR format"
                                    );
                                }
                            });
                        });

                    ui.add_space(8.0);

                    // === Format buttons ===
                    ui.horizontal(|ui| {
                        ui.add_enabled_ui(!self.is_encoding(), |ui| {
                            for format in SequenceFormat::all() {
                                let is_selected = self.sequence_settings.format == *format;
                                let button = egui::Button::new(format.to_string())
                                    .selected(is_selected)
                                    .min_size(egui::vec2(70.0, 0.0));

                                if ui.add(button).clicked() {
                                    self.sequence_settings.format = *format;
                                    // Update file extension
                                    self.output_path.set_extension(format.extension());
                                    // Validate settings for new format
                                    self.sequence_settings.validate();
                                }
                            }
                        });
                    });

                    ui.separator();
                    ui.add_space(4.0);

                    // === Format-specific settings ===
                    ui.add_enabled_ui(!self.is_encoding(), |ui| {
                        self.render_sequence_format_settings(ui, active_comp);
                    });
                }
            }

            // === Frame Range ===
            // Per-field labels (`Start`, `End`) sit OUTSIDE the
            // DragValue widgets instead of being baked into the
            // numeric prefix — matches the rest of the settings
            // panel's label conventions and avoids the "Start 0"
            // typed-into-the-field look.
            ui.horizontal(|ui| {
                ui.label("Frame Range:");
                ui.add_enabled_ui(!self.is_encoding(), |ui| {
                    ui.label("Start");
                    ui.add(egui::DragValue::new(&mut self.frame_start).speed(1.0));
                    ui.label("End");
                    ui.add(egui::DragValue::new(&mut self.frame_end).speed(1.0));
                });
            });
            if self.frame_end < self.frame_start {
                self.frame_end = self.frame_start;
            }

            ui.separator();

            // === Progress (always visible to prevent dialog size jumping) ===
            if self.is_encoding() {
                if let Some(ref progress) = self.progress {
                    let stage_text = match &progress.stage {
                        EncodeStage::Validating => "Validating frame sizes...",
                        EncodeStage::Opening => "Opening encoder...",
                        EncodeStage::Encoding => "Encoding frames...",
                        EncodeStage::Flushing => "Flushing encoder...",
                        EncodeStage::Complete => "Complete!",
                        EncodeStage::Error(msg) => msg.as_str(),
                    };
                    ui.label(stage_text);
                    self.progress_bar.set_progress(
                        progress.current_frame.max(0) as usize,
                        progress.total_frames.max(0) as usize,
                    );
                    self.progress_bar.render(ui);
                }
            } else {
                // Idle: keep the slot occupied (label + bar) so the
                // section height doesn't jump when encoding starts.
                ui.label("Ready to encode");
                let planned_total = active_comp
                    .map(|c| {
                        let (s, e) = c.play_range(true);
                        (e - s + 1).max(0) as usize
                    })
                    .unwrap_or(0);
                self.progress_bar.set_progress(0, planned_total);
                self.progress_bar.render(ui);
            }

            ui.separator();

            // === Readiness check ===
            let ready_to_encode = active_comp.is_some();

            if !ready_to_encode {
                ui.colored_label(
                    egui::Color32::from_rgb(200, 150, 0),
                    "No active comp to encode",
                );
            }

            // === Buttons ===
            // Window mode (`with_close_button`): [Close] [Encode/Stop]
            // side-by-side. Inline mode: single full-width Encode/Stop
            // toggle — the host panel owns visibility, Close is moot.
            if with_close_button {
                ui.horizontal(|ui| {
                    if ui.button("Close").clicked() {
                        if self.is_encoding() {
                            self.stop_encoding_and_close();
                        }
                        should_close = true;
                    }

                    if self.is_encoding() {
                        if self.lifecycle.is_cancelling() {
                            ui.add_enabled(false, egui::Button::new("Stopping..."));
                        } else if ui.button("Stop").clicked() {
                            self.stop_encoding_keep_window();
                        }
                    } else {
                        ui.add_enabled_ui(ready_to_encode, |ui| {
                            let mut button = ui.button("Encode");
                            if !ready_to_encode {
                                button = button.on_disabled_hover_text("No active comp");
                            }
                            if button.clicked()
                                && let Some(comp) = active_comp
                            {
                                self.start_encoding(comp, project);
                            }
                        });
                    }
                });
            } else {
                // Inline: full-width action button. `min_size` with
                // `ui.available_width()` stretches it across the
                // section without forcing a layout dance.
                let row_w = ui.available_width();
                if self.is_encoding() {
                    let label = if self.lifecycle.is_cancelling() {
                        "Stopping..."
                    } else {
                        "Stop"
                    };
                    let stop_btn = egui::Button::new(label).min_size(egui::vec2(row_w, 0.0));
                    if ui
                        .add_enabled(!self.lifecycle.is_cancelling(), stop_btn)
                        .clicked()
                    {
                        self.stop_encoding_keep_window();
                    }
                } else {
                    ui.add_enabled_ui(ready_to_encode, |ui| {
                        let encode_btn =
                            egui::Button::new("Encode").min_size(egui::vec2(row_w, 0.0));
                        let mut resp = ui.add(encode_btn);
                        if !ready_to_encode {
                            resp = resp.on_disabled_hover_text("No active comp");
                        }
                        if resp.clicked()
                            && let Some(comp) = active_comp
                        {
                            self.start_encoding(comp, project);
                        }
                    });
                }
            }
        }
        should_close
    }

    /// Start one generation. A second generation cannot start until this worker is joined.
    fn start_encoding(&mut self, comp: &Comp, project: &Project) {
        let token = match &self.lifecycle {
            EncodeLifecycle::Idle(token) if token.generation() < u64::MAX => token.clone(),
            EncodeLifecycle::Idle(_) => {
                self.set_terminal_error("Encoder generation counter exhausted".into());
                return;
            }
            _ => return,
        };

        info!(
            "Starting encoding generation {} ({:?})",
            token.generation(),
            self.export_mode
        );
        self.progress = None;

        let (progress_tx, progress_rx) = channel();
        let cancel_flag = token.flag();
        let comp = comp.clone();
        let project = project.clone();

        let worker = match self.export_mode {
            ExportMode::Video => {
                let settings = self.build_encoder_settings();
                std::thread::Builder::new()
                    .name(format!("encode-video-{}", token.generation()))
                    .spawn(move || {
                        encode_comp(&comp, &project, &settings, progress_tx, cancel_flag)
                    })
            }
            ExportMode::Sequence => {
                let settings = self.sequence_settings.clone();
                let output_path = self.output_path.clone();
                std::thread::Builder::new()
                    .name(format!("encode-sequence-{}", token.generation()))
                    .spawn(move || {
                        encode_image_sequence(
                            &comp,
                            &project,
                            &output_path,
                            &settings,
                            progress_tx,
                            cancel_flag,
                        )
                    })
            }
        };

        match worker {
            Ok(worker) => {
                self.lifecycle = EncodeLifecycle::Running(EncodeSession {
                    token,
                    progress_rx,
                    worker,
                });
            }
            Err(error) => {
                self.set_terminal_error(format!("Failed to start encoder worker: {}", error));
            }
        }
    }

    /// Stop encoding and close window.
    fn stop_encoding_and_close(&mut self) {
        info!("Stopping encoding (closing window)");
        self.stop_encoding_internal();
    }

    /// Stop encoding but keep window open.
    fn stop_encoding_keep_window(&mut self) {
        info!("Stopping encoding (keeping window open)");
        self.stop_encoding_internal();
    }

    /// Request cancellation. Worker ownership stays in the lifecycle until a later poll joins it.
    fn stop_encoding_internal(&mut self) {
        let state = self.take_lifecycle();
        self.lifecycle = match state {
            EncodeLifecycle::Running(session) | EncodeLifecycle::Finishing(session) => {
                session.token.cancel();
                EncodeLifecycle::Cancelling(session)
            }
            EncodeLifecycle::Cancelling(session) => EncodeLifecycle::Cancelling(session),
            idle @ EncodeLifecycle::Idle(_) => idle,
        };
    }

    fn render_h264_settings(&mut self, ui: &mut egui::Ui) {
        let profiles: &[&str] = &["baseline", "main", "high"];
        render_h26x_settings(
            ui,
            &mut self.codec_settings.h264,
            "h264",
            "18=best, 23=default, 28=fast",
            &[
                "ultrafast",
                "superfast",
                "veryfast",
                "faster",
                "fast",
                "medium",
                "slow",
                "slower",
                "veryslow",
            ],
            profiles,
        );
    }

    fn render_h265_settings(&mut self, ui: &mut egui::Ui) {
        let profiles: &[&str] = &["main", "main10"];
        render_h26x_settings(
            ui,
            &mut self.codec_settings.h265,
            "h265",
            "28=default (higher than H.264)",
            &[
                "ultrafast",
                "superfast",
                "veryfast",
                "faster",
                "fast",
                "medium",
                "slow",
                "slower",
                "veryslow",
                "placebo",
            ],
            profiles,
        );
    }

    /// Render ProRes settings
    fn render_prores_settings(&mut self, ui: &mut egui::Ui) {
        ui.label("Profile:");
        ui.horizontal(|ui| {
            for profile in ProResProfile::all() {
                ui.radio_value(
                    &mut self.codec_settings.prores.profile,
                    *profile,
                    profile.to_string(),
                );
            }
        });

        ui.add_space(4.0);
        ui.label("ProRes CPU encoder (prores_aw)");

        // Empty lines for vertical alignment with H264 tab
        ui.add_space(4.0);
        ui.label("");
        ui.add_space(4.0);
        ui.label("");
        ui.add_space(4.0);
        ui.label("");
        ui.add_space(4.0);
        ui.label("");
        ui.add_space(4.0);
        ui.label("");
    }

    /// Render AV1 settings
    fn render_av1_settings(&mut self, ui: &mut egui::Ui) {
        use crate::dialogs::encode::QualityMode;

        ui.label("Quality Mode:");
        ui.horizontal(|ui| {
            for mode in QualityMode::all() {
                ui.radio_value(
                    &mut self.codec_settings.av1.quality_mode,
                    *mode,
                    mode.to_string(),
                );
            }
        });

        ui.horizontal(|ui| {
            ui.label("Value:");
            let hint = match self.codec_settings.av1.quality_mode {
                QualityMode::CRF => "CRF (0-63, lower=better)",
                QualityMode::Bitrate => "kbps",
            };
            ui.add(
                egui::Slider::new(&mut self.codec_settings.av1.quality_value, 0..=10000).text(hint),
            );
        });

        ui.add_space(4.0);
        ui.label("AV1 CPU encoder (rav1e, fixed speed preset)");
    }

    /// Render format-specific settings for image sequence export
    fn render_sequence_format_settings(&mut self, ui: &mut egui::Ui, active_comp: Option<&Comp>) {
        // Per-format settings (compression, quality, etc.)
        match self.sequence_settings.format {
            SequenceFormat::Exr => {
                // Encode mode (Display only vs Pass-through). Pass-through reads
                // each source EXR via vfx-io and writes back preserving every
                // layer + per-layer compression â€” the OIIO-aligned transcode path.
                ui.horizontal(|ui| {
                    ui.label("Mode:");
                    egui::ComboBox::from_id_salt("exr_mode")
                        .selected_text(
                            self.sequence_settings
                                .format_settings
                                .exr
                                .mode
                                .to_string(),
                        )
                        .show_ui(ui, |ui| {
                            for m in crate::dialogs::encode::ExrEncodeMode::all() {
                                ui.selectable_value(
                                    &mut self.sequence_settings.format_settings.exr.mode,
                                    *m,
                                    m.to_string(),
                                )
                                .on_hover_text(match m {
                                    crate::dialogs::encode::ExrEncodeMode::DisplayOnly =>
                                        "Single RGBA layer from compositor output. Standard EXR write.",
                                    crate::dialogs::encode::ExrEncodeMode::PassThrough =>
                                        "Read source EXR via vfx-io and preserve every layer + per-layer compression. Falls back to display-only if source isn't EXR.",
                                });
                            }
                        });
                });
                // Compression / DWA controls only relevant for Display-only mode
                // (Pass-through preserves source per-layer compression).
                let pass_through = matches!(
                    self.sequence_settings.format_settings.exr.mode,
                    crate::dialogs::encode::ExrEncodeMode::PassThrough,
                );
                ui.add_enabled_ui(!pass_through, |ui| {
                    ui.horizontal(|ui| {
                        ui.label("Compression:");
                        egui::ComboBox::from_id_salt("exr_compression")
                            .selected_text(
                                self.sequence_settings
                                    .format_settings
                                    .exr
                                    .compression
                                    .to_string(),
                            )
                            .show_ui(ui, |ui| {
                                for comp in ExrCompression::all() {
                                    ui.selectable_value(
                                        &mut self
                                            .sequence_settings
                                            .format_settings
                                            .exr
                                            .compression,
                                        *comp,
                                        comp.to_string(),
                                    );
                                }
                            });
                    });
                    // DWA loss level â€” only meaningful for DWAA/DWAB.
                    // OpenEXR semantics: lower = less loss, 45 = visually lossless,
                    // higher = smaller files / more loss. NOT the usual "quality 0-100".
                    if self
                        .sequence_settings
                        .format_settings
                        .exr
                        .compression
                        .has_quality_knob()
                    {
                        ui.horizontal(|ui| {
                            ui.label("DWA loss level:").on_hover_text(
                                "Lower = less loss / larger files. 45 = visually lossless (OpenEXR default).",
                            );
                            ui.add(
                                egui::Slider::new(
                                    &mut self
                                        .sequence_settings
                                        .format_settings
                                        .exr
                                        .dwa_quality,
                                    0.0..=200.0,
                                )
                                .text("(45 default)"),
                            );
                        });
                    }
                });
                ui.add_space(4.0);
                if pass_through {
                    self.render_exr_source_layer_info(ui, active_comp);
                } else {
                    ui.label("EXR: HDR format, preserves full dynamic range");
                }
            }
            SequenceFormat::Png => {
                ui.horizontal(|ui| {
                    ui.label("Compression:");
                    ui.add(
                        egui::Slider::new(
                            &mut self.sequence_settings.format_settings.png.compression,
                            0..=9,
                        )
                        .text("level"),
                    );
                });
                ui.add_space(4.0);
                ui.label("PNG: Lossless, good for compositing");
            }
            SequenceFormat::Jpeg => {
                ui.horizontal(|ui| {
                    ui.label("Quality:");
                    ui.add(
                        egui::Slider::new(
                            &mut self.sequence_settings.format_settings.jpeg.quality,
                            1..=100,
                        )
                        .text("%"),
                    );
                });
                ui.add_space(4.0);
                ui.label("JPEG: Lossy, small files, no alpha");
            }
            SequenceFormat::Tiff => {
                ui.horizontal(|ui| {
                    ui.label("Compression:");
                    egui::ComboBox::from_id_salt("tiff_compression")
                        .selected_text(
                            self.sequence_settings
                                .format_settings
                                .tiff
                                .compression
                                .to_string(),
                        )
                        .show_ui(ui, |ui| {
                            for comp in TiffCompression::all() {
                                ui.selectable_value(
                                    &mut self.sequence_settings.format_settings.tiff.compression,
                                    *comp,
                                    comp.to_string(),
                                );
                            }
                        });
                });
                ui.add_space(4.0);
                ui.label("TIFF: Industry standard, lossless");
            }
            SequenceFormat::Tga => {
                ui.horizontal(|ui| {
                    ui.checkbox(
                        &mut self.sequence_settings.format_settings.tga.rle_compression,
                        "RLE Compression",
                    );
                });
                ui.add_space(4.0);
                ui.label("TGA: Legacy format, game industry");
            }
        }

        // Padding pattern hint
        ui.add_space(8.0);
        ui.separator();
        ui.add_space(4.0);
        ui.label("Padding patterns: #### (4 digits), %04d (printf), @ (no padding)");
    }

    /// Renders optional source-layer info supplied by the host FrameSource.
    fn render_exr_source_layer_info(&mut self, ui: &mut egui::Ui, active_comp: Option<&Comp>) {
        let Some(info) = active_comp.and_then(|comp| comp.exr_source_info()) else {
            ui.colored_label(
                egui::Color32::from_rgb(220, 180, 80),
                "Pass-through: no EXR source in project - will fall back to display-only",
            );
            return;
        };

        ui.label(format!(
            "Pass-through source: {}  ({} layer{})",
            info.path.display(),
            info.layer_count,
            if info.layer_count == 1 { "" } else { "s" },
        ));
        ui.indent("exr_source_layers", |ui| {
            for layer in &info.layers {
                ui.label(format!(
                    "{} {}  -  {}",
                    layer.marker, layer.name, layer.compression
                ));
            }
        });
        ui.label("Pass-through preserves every layer + per-layer compression.");
    }
}
impl Drop for EncodeDialog {
    fn drop(&mut self) {
        let state = self.take_lifecycle();
        let session = match state {
            EncodeLifecycle::Running(session)
            | EncodeLifecycle::Cancelling(session)
            | EncodeLifecycle::Finishing(session) => session,
            EncodeLifecycle::Idle(token) => {
                self.lifecycle = EncodeLifecycle::Idle(token);
                return;
            }
        };

        session.token.cancel();
        if let Err(payload) = session.worker.join() {
            info!("Encoder worker panicked during shutdown: {:?}", payload);
        }
    }
}

/// Render H.264/H.265 settings. Codec-specific differences are passed as parameters:
/// - `id_prefix`: "h264" or "h265" â€” used as egui ComboBox id_salt to avoid conflicts
/// - `crf_hint`: the CRF quality hint string shown next to the slider
/// - `presets`: codec-supported CPU preset strings
/// - `profiles`: available profile strings for the profile ComboBox
fn render_h26x_settings(
    ui: &mut egui::Ui,
    settings: &mut dyn crate::dialogs::encode::H26xSettingsMut,
    id_prefix: &str,
    crf_hint: &str,
    presets: &[&str],
    profiles: &[&str],
) {
    use crate::dialogs::encode::QualityMode;

    ui.add_space(4.0);

    ui.label("Quality Mode:");
    ui.horizontal(|ui| {
        for mode in QualityMode::all() {
            ui.radio_value(settings.quality_mode_mut(), *mode, mode.to_string());
        }
    });

    ui.horizontal(|ui| {
        ui.label("Value:");
        let hint = match settings.quality_mode() {
            QualityMode::CRF => crf_hint,
            QualityMode::Bitrate => "kbps",
        };
        ui.add(egui::Slider::new(settings.quality_value_mut(), 1..=10000).text(hint));
    });

    ui.add_space(4.0);

    ui.horizontal(|ui| {
        ui.label("Preset:");
        let preset_id = format!("{}_preset", id_prefix);
        egui::ComboBox::from_id_salt(preset_id)
            .selected_text(settings.preset())
            .show_ui(ui, |ui| {
                for &preset in presets {
                    ui.selectable_value(settings.preset_mut(), preset.to_string(), preset);
                }
            });
    });

    ui.horizontal(|ui| {
        ui.label("Profile:");
        let profile_id = format!("{}_profile", id_prefix);
        egui::ComboBox::from_id_salt(profile_id)
            .selected_text(settings.profile())
            .show_ui(ui, |ui| {
                for &profile in profiles {
                    ui.selectable_value(settings.profile_mut(), profile.to_string(), profile);
                }
            });
    });

    ui.add_space(4.0);
    ui.label(""); // Spacer for visual alignment with other codec tabs
}

/// Remove any frame-padding token from a filename stem so video
/// output names don't carry the sequence-mode `####` after the
/// user flips back to Video. Recognises the three formats
/// `media-encoder` accepts as inputs:
///
/// * `####`  — hash padding (any run of `#`)
/// * `@@@@`  — at-sign padding (Houdini convention)
/// * `%04d` / `%4d` / `%08d` — printf-style padding
///
/// Tokens are dropped along with the `.` that typically separates
/// them from the rest of the stem (e.g. `frame.####` → `frame`).
/// A token at the very start of the stem is also stripped. If
/// stripping leaves an empty string the literal `output` is
/// returned so the user never ends up with `.mp4` as a filename.
fn strip_frame_token(stem: &str) -> String {
    let mut s = stem.to_string();
    // Walk every padding family; loop until no change so a stem
    // like `out.####.%04d` still ends up clean.
    loop {
        let before = s.clone();
        s = strip_hash_token(&s);
        s = strip_at_token(&s);
        s = strip_printf_token(&s);
        if s == before {
            break;
        }
    }
    // Trim any leftover `.` boundary the token leaves behind.
    let trimmed = s.trim_matches('.').to_string();
    if trimmed.is_empty() {
        "output".to_string()
    } else {
        trimmed
    }
}

/// Inverse of [`strip_frame_token`]: append `.####` if no padding
/// token is already present. Picks 4-hash as a sensible default —
/// users editing the field by hand can change padding width
/// directly.
fn ensure_frame_token(stem: &str) -> String {
    let has_token = stem.contains('#') || stem.contains('@') || has_printf_token(stem);
    if has_token {
        stem.to_string()
    } else {
        format!("{stem}.####")
    }
}

fn strip_hash_token(s: &str) -> String {
    if let Some(start) = s.find('#') {
        let end = s[start..]
            .find(|c: char| c != '#')
            .map(|idx| start + idx)
            .unwrap_or(s.len());
        let mut head = s[..start].to_string();
        if head.ends_with('.') {
            head.pop();
        }
        head.push_str(&s[end..]);
        head
    } else {
        s.to_string()
    }
}

fn strip_at_token(s: &str) -> String {
    if let Some(start) = s.find('@') {
        let end = s[start..]
            .find(|c: char| c != '@')
            .map(|idx| start + idx)
            .unwrap_or(s.len());
        let mut head = s[..start].to_string();
        if head.ends_with('.') {
            head.pop();
        }
        head.push_str(&s[end..]);
        head
    } else {
        s.to_string()
    }
}

fn strip_printf_token(s: &str) -> String {
    // `%04d`, `%4d`, `%08d`, etc. Scan for `%`, then skip an
    // optional width, then expect `d` to commit the strip.
    let bytes = s.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%' {
            let start = i;
            let mut j = i + 1;
            while j < bytes.len() && bytes[j].is_ascii_digit() {
                j += 1;
            }
            if j < bytes.len() && bytes[j] == b'd' && j > i + 1 {
                let mut head = s[..start].to_string();
                if head.ends_with('.') {
                    head.pop();
                }
                head.push_str(&s[j + 1..]);
                return head;
            }
        }
        i += 1;
    }
    s.to_string()
}

fn has_printf_token(s: &str) -> bool {
    strip_printf_token(s) != s
}

fn rebuild_path(parent: Option<&std::path::Path>, stem: &str, ext: &str) -> PathBuf {
    let filename = format!("{stem}.{ext}");
    match parent {
        Some(p) if !p.as_os_str().is_empty() => p.join(filename),
        _ => PathBuf::from(filename),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn strip_hash() {
        assert_eq!(strip_frame_token("frame.####"), "frame");
        assert_eq!(strip_frame_token("frame.###"), "frame");
        assert_eq!(strip_frame_token("####"), "output");
    }

    #[test]
    fn strip_at() {
        assert_eq!(strip_frame_token("frame.@@@@"), "frame");
    }

    #[test]
    fn strip_printf() {
        assert_eq!(strip_frame_token("frame.%04d"), "frame");
        assert_eq!(strip_frame_token("frame.%4d"), "frame");
        assert_eq!(strip_frame_token("frame%08d"), "frame");
    }

    #[test]
    fn strip_no_token_is_noop() {
        assert_eq!(strip_frame_token("plain"), "plain");
    }

    #[test]
    fn ensure_adds_when_missing() {
        assert_eq!(ensure_frame_token("frame"), "frame.####");
    }

    #[test]
    fn ensure_keeps_existing() {
        assert_eq!(ensure_frame_token("frame.###"), "frame.###");
        assert_eq!(ensure_frame_token("frame.%04d"), "frame.%04d");
    }
}
