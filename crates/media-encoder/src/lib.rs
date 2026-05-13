//! Reusable image sequence and render-export UI.
//!
//! The crate owns path-template normalization, frame path expansion, progress/ETA
//! accounting, encoder-facing settings, and an egui/egui-dock dialog. Host
//! applications provide rendered RGBA frames and drive their own render loop.

use egui_dock::{DockArea, DockState, NodeIndex, TabViewer};
#[cfg(feature = "ffmpeg")]
pub use playa_ffmpeg as ffmpeg;
use serde::{Deserialize, Serialize};
use std::collections::VecDeque;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

const DEFAULT_MARKER: &str = "####";
const DEFAULT_EXTENSION: &str = "png";
const ETA_HISTORY_LIMIT: usize = 16;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SequenceTemplate {
    path: PathBuf,
}

impl SequenceTemplate {
    pub fn new(path: impl Into<PathBuf>) -> Self {
        Self { path: path.into() }
    }

    pub fn normalized(path: impl Into<PathBuf>) -> Self {
        Self {
            path: normalize_template_path(path.into()),
        }
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    pub fn frame_path(&self, frame_number: u32) -> PathBuf {
        expand_frame_path(&self.path, frame_number)
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SequenceJob {
    pub template: SequenceTemplate,
    pub start_frame: u32,
    pub frame_count: u32,
    pub max_samples: u32,
    pub fps: f32,
}

impl SequenceJob {
    pub fn new(
        template_path: impl Into<PathBuf>,
        start_frame: u32,
        frame_count: u32,
        max_samples: u32,
        fps: f32,
    ) -> Self {
        Self {
            template: SequenceTemplate::normalized(template_path),
            start_frame,
            frame_count: frame_count.max(1),
            max_samples: max_samples.max(1),
            fps: fps.max(0.001),
        }
    }

    pub fn frame_number(&self, frame_index: u32) -> u32 {
        self.start_frame.saturating_add(frame_index)
    }

    pub fn frame_time_seconds(&self, frame_index: u32) -> f32 {
        frame_index as f32 / self.fps
    }
}

#[derive(Debug, Clone)]
pub struct SequenceProgress {
    job: SequenceJob,
    current_index: u32,
    current_samples: u32,
    started_at: Instant,
    frame_started_at: Instant,
    recent_frame_durations: VecDeque<Duration>,
    finished: bool,
}

impl SequenceProgress {
    pub fn new(job: SequenceJob) -> Self {
        let now = Instant::now();
        Self {
            job,
            current_index: 0,
            current_samples: 0,
            started_at: now,
            frame_started_at: now,
            recent_frame_durations: VecDeque::new(),
            finished: false,
        }
    }

    pub fn job(&self) -> &SequenceJob {
        &self.job
    }

    pub fn current_index(&self) -> u32 {
        self.current_index
    }

    pub fn current_frame_number(&self) -> u32 {
        self.job.frame_number(self.current_index)
    }

    pub fn current_frame_path(&self) -> PathBuf {
        self.job.template.frame_path(self.current_frame_number())
    }

    pub fn current_frame_time_seconds(&self) -> f32 {
        self.job.frame_time_seconds(self.current_index)
    }

    pub fn observe_samples(&mut self, samples: u32) {
        self.current_samples = samples.min(self.job.max_samples);
    }

    pub fn current_samples(&self) -> u32 {
        self.current_samples
    }

    pub fn current_frame_ready(&self) -> bool {
        !self.finished && self.current_samples >= self.job.max_samples
    }

    pub fn complete_current_frame(&mut self) {
        if self.finished {
            return;
        }

        let now = Instant::now();
        self.recent_frame_durations
            .push_back(now.saturating_duration_since(self.frame_started_at));
        while self.recent_frame_durations.len() > ETA_HISTORY_LIMIT {
            self.recent_frame_durations.pop_front();
        }

        self.current_index = self.current_index.saturating_add(1);
        self.current_samples = 0;
        self.frame_started_at = now;
        self.finished = self.current_index >= self.job.frame_count;
    }

    pub fn snapshot(&self) -> ProgressSnapshot {
        let completed_frames = self.current_index.min(self.job.frame_count);
        let current_sample_fraction = if self.finished {
            0.0
        } else {
            self.current_samples as f32 / self.job.max_samples as f32
        }
        .clamp(0.0, 1.0);

        let total_units = self.job.frame_count as f32;
        let done_units = completed_frames as f32
            + if self.finished {
                0.0
            } else {
                current_sample_fraction
            };

        ProgressSnapshot {
            completed_frames,
            total_frames: self.job.frame_count,
            current_frame_number: self.current_frame_number(),
            current_samples: self.current_samples,
            max_samples: self.job.max_samples,
            elapsed: self.started_at.elapsed(),
            eta: self.estimate_eta(current_sample_fraction),
            fraction: if total_units > 0.0 {
                (done_units / total_units).clamp(0.0, 1.0)
            } else {
                1.0
            },
            finished: self.finished,
        }
    }

    pub fn is_finished(&self) -> bool {
        self.finished
    }

    fn estimate_eta(&self, current_sample_fraction: f32) -> Option<Duration> {
        if self.finished {
            return Some(Duration::ZERO);
        }

        let avg_frame = self.average_frame_duration().or_else(|| {
            if self.current_samples == 0 {
                None
            } else {
                let elapsed = self.frame_started_at.elapsed().as_secs_f32();
                Some(Duration::from_secs_f32(
                    elapsed / current_sample_fraction.max(0.001),
                ))
            }
        })?;

        let current_remaining = avg_frame.mul_f32((1.0 - current_sample_fraction).clamp(0.0, 1.0));
        let full_frames_remaining = self
            .job
            .frame_count
            .saturating_sub(self.current_index)
            .saturating_sub(1);
        Some(current_remaining + avg_frame.mul_f32(full_frames_remaining as f32))
    }

    fn average_frame_duration(&self) -> Option<Duration> {
        if self.recent_frame_durations.is_empty() {
            return None;
        }
        let total_secs: f32 = self
            .recent_frame_durations
            .iter()
            .map(Duration::as_secs_f32)
            .sum();
        Some(Duration::from_secs_f32(
            total_secs / self.recent_frame_durations.len() as f32,
        ))
    }
}

#[derive(Debug, Clone)]
pub struct ProgressSnapshot {
    pub completed_frames: u32,
    pub total_frames: u32,
    pub current_frame_number: u32,
    pub current_samples: u32,
    pub max_samples: u32,
    pub elapsed: Duration,
    pub eta: Option<Duration>,
    pub fraction: f32,
    pub finished: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum ExportMode {
    ImageSequence,
    Video,
}

impl ExportMode {
    pub fn label(self) -> &'static str {
        match self {
            Self::ImageSequence => "Image Sequence",
            Self::Video => "Video",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum SequenceFormat {
    Exr,
    Png,
    Jpeg,
    Tiff,
    Tga,
}

impl SequenceFormat {
    pub fn all() -> &'static [Self] {
        &[Self::Exr, Self::Png, Self::Jpeg, Self::Tiff, Self::Tga]
    }

    pub fn label(self) -> &'static str {
        match self {
            Self::Exr => "EXR",
            Self::Png => "PNG",
            Self::Jpeg => "JPEG",
            Self::Tiff => "TIFF",
            Self::Tga => "TGA",
        }
    }

    pub fn extension(self) -> &'static str {
        match self {
            Self::Exr => "exr",
            Self::Png => "png",
            Self::Jpeg => "jpg",
            Self::Tiff => "tiff",
            Self::Tga => "tga",
        }
    }

    pub fn supports_alpha(self) -> bool {
        !matches!(self, Self::Jpeg)
    }

    pub fn supported_depths(self) -> &'static [OutputBitDepth] {
        match self {
            Self::Exr => &[OutputBitDepth::F16, OutputBitDepth::F32],
            Self::Png => &[OutputBitDepth::U8, OutputBitDepth::U16],
            Self::Jpeg | Self::Tga => &[OutputBitDepth::U8],
            Self::Tiff => &[OutputBitDepth::U8, OutputBitDepth::U16],
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum ChannelMode {
    Rgba,
    Rgb,
}

impl Default for ChannelMode {
    fn default() -> Self {
        Self::Rgba
    }
}

impl ChannelMode {
    pub fn all() -> &'static [Self] {
        &[Self::Rgba, Self::Rgb]
    }

    pub fn label(self) -> &'static str {
        match self {
            Self::Rgba => "RGBA",
            Self::Rgb => "RGB",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum OutputBitDepth {
    U8,
    U16,
    F16,
    F32,
}

impl Default for OutputBitDepth {
    fn default() -> Self {
        Self::U8
    }
}

impl OutputBitDepth {
    pub fn all() -> &'static [Self] {
        &[Self::U8, Self::U16, Self::F16, Self::F32]
    }

    pub fn label(self) -> &'static str {
        match self {
            Self::U8 => "8-bit",
            Self::U16 => "16-bit",
            Self::F16 => "Half Float",
            Self::F32 => "Float",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum ExrCompression {
    None,
    Rle,
    Zips,
    Zip,
    Piz,
    Pxr24,
    B44,
    B44a,
    Dwaa,
    Dwab,
    HtJ2k32,
    HtJ2k256,
}

impl Default for ExrCompression {
    fn default() -> Self {
        Self::Zip
    }
}

impl ExrCompression {
    pub fn all() -> &'static [Self] {
        &[
            Self::None,
            Self::Rle,
            Self::Zips,
            Self::Zip,
            Self::Piz,
            Self::Pxr24,
            Self::B44,
            Self::B44a,
            Self::Dwaa,
            Self::Dwab,
            Self::HtJ2k32,
            Self::HtJ2k256,
        ]
    }

    pub fn label(self) -> &'static str {
        match self {
            Self::None => "None",
            Self::Rle => "RLE",
            Self::Zips => "ZIPS",
            Self::Zip => "ZIP",
            Self::Piz => "PIZ",
            Self::Pxr24 => "PXR24",
            Self::B44 => "B44",
            Self::B44a => "B44A",
            Self::Dwaa => "DWAA",
            Self::Dwab => "DWAB",
            Self::HtJ2k32 => "HTJ2K-32",
            Self::HtJ2k256 => "HTJ2K-256",
        }
    }

    pub fn has_quality_knob(self) -> bool {
        matches!(self, Self::Dwaa | Self::Dwab)
    }

    pub fn to_oiio_string(self, dwa_quality: f32) -> String {
        match self {
            Self::None => "none".to_string(),
            Self::Rle => "rle".to_string(),
            Self::Zips => "zips".to_string(),
            Self::Zip => "zip".to_string(),
            Self::Piz => "piz".to_string(),
            Self::Pxr24 => "pxr24".to_string(),
            Self::B44 => "b44".to_string(),
            Self::B44a => "b44a".to_string(),
            Self::Dwaa => format!("dwaa:{dwa_quality:.1}"),
            Self::Dwab => format!("dwab:{dwa_quality:.1}"),
            Self::HtJ2k32 => "htj2k:32".to_string(),
            Self::HtJ2k256 => "htj2k:256".to_string(),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum TiffCompression {
    None,
    Lzw,
    Zip,
    PackBits,
}

impl Default for TiffCompression {
    fn default() -> Self {
        Self::Lzw
    }
}

impl TiffCompression {
    pub fn all() -> &'static [Self] {
        &[Self::None, Self::Lzw, Self::Zip, Self::PackBits]
    }

    pub fn label(self) -> &'static str {
        match self {
            Self::None => "None",
            Self::Lzw => "LZW",
            Self::Zip => "ZIP",
            Self::PackBits => "PackBits",
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ExrSequenceSettings {
    pub compression: ExrCompression,
    pub dwa_quality: f32,
}

impl Default for ExrSequenceSettings {
    fn default() -> Self {
        Self {
            compression: ExrCompression::Zip,
            dwa_quality: 45.0,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PngSequenceSettings {
    pub compression: u8,
}

impl Default for PngSequenceSettings {
    fn default() -> Self {
        Self { compression: 6 }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct JpegSequenceSettings {
    pub quality: u8,
}

impl Default for JpegSequenceSettings {
    fn default() -> Self {
        Self { quality: 90 }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TiffSequenceSettings {
    pub compression: TiffCompression,
}

impl Default for TiffSequenceSettings {
    fn default() -> Self {
        Self {
            compression: TiffCompression::Lzw,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TgaSequenceSettings {
    pub rle_compression: bool,
}

impl Default for TgaSequenceSettings {
    fn default() -> Self {
        Self {
            rle_compression: true,
        }
    }
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct SequenceFormatSettings {
    pub exr: ExrSequenceSettings,
    pub png: PngSequenceSettings,
    pub jpeg: JpegSequenceSettings,
    pub tiff: TiffSequenceSettings,
    pub tga: TgaSequenceSettings,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum VideoContainer {
    Mp4,
    Mov,
    Mkv,
}

impl VideoContainer {
    pub fn all() -> &'static [Self] {
        &[Self::Mp4, Self::Mov, Self::Mkv]
    }

    pub fn label(self) -> &'static str {
        match self {
            Self::Mp4 => "MP4",
            Self::Mov => "MOV",
            Self::Mkv => "MKV",
        }
    }

    pub fn extension(self) -> &'static str {
        match self {
            Self::Mp4 => "mp4",
            Self::Mov => "mov",
            Self::Mkv => "mkv",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum VideoCodec {
    H264,
    H265,
    ProRes,
    Av1,
}

impl VideoCodec {
    pub fn all() -> &'static [Self] {
        &[Self::H264, Self::H265, Self::ProRes, Self::Av1]
    }

    pub fn label(self) -> &'static str {
        match self {
            Self::H264 => "H.264",
            Self::H265 => "H.265",
            Self::ProRes => "ProRes",
            Self::Av1 => "AV1",
        }
    }

    pub fn preferred_container(self) -> VideoContainer {
        match self {
            Self::ProRes => VideoContainer::Mov,
            Self::H264 | Self::H265 | Self::Av1 => VideoContainer::Mp4,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum QualityMode {
    Crf,
    Bitrate,
}

impl QualityMode {
    pub fn all() -> &'static [Self] {
        &[Self::Crf, Self::Bitrate]
    }

    pub fn label(self) -> &'static str {
        match self {
            Self::Crf => "CRF",
            Self::Bitrate => "Bitrate",
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SequenceSettings {
    pub format: SequenceFormat,
    #[serde(default)]
    pub channels: ChannelMode,
    #[serde(default)]
    pub bit_depth: OutputBitDepth,
    pub start_frame: u32,
    pub frame_count: u32,
    pub fps: f32,
    pub max_samples: u32,
    #[serde(default)]
    pub format_settings: SequenceFormatSettings,
}

impl Default for SequenceSettings {
    fn default() -> Self {
        Self {
            format: SequenceFormat::Png,
            channels: ChannelMode::Rgba,
            bit_depth: OutputBitDepth::U8,
            start_frame: 1,
            frame_count: 120,
            fps: 30.0,
            max_samples: 512,
            format_settings: SequenceFormatSettings::default(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct VideoSettings {
    pub container: VideoContainer,
    pub codec: VideoCodec,
    pub quality_mode: QualityMode,
    pub crf: u8,
    pub bitrate_mbps: f32,
}

impl Default for VideoSettings {
    fn default() -> Self {
        Self {
            container: VideoContainer::Mp4,
            codec: VideoCodec::H264,
            quality_mode: QualityMode::Crf,
            crf: 18,
            bitrate_mbps: 40.0,
        }
    }
}

#[derive(Debug, thiserror::Error)]
pub enum MediaEncodeError {
    #[error("cancelled")]
    Cancelled,
    #[error("invalid frame buffer: {0}")]
    InvalidFrame(String),
    #[error("image encode failed: {0}")]
    Image(String),
    #[error("EXR encode failed: {0}")]
    Exr(String),
    #[error("FFmpeg failed: {0}")]
    Ffmpeg(String),
    #[error("I/O failed: {0}")]
    Io(#[from] std::io::Error),
}

#[derive(Debug, Clone)]
pub enum MediaPixels {
    Rgba8(Vec<u8>),
    Rgba16(Vec<u16>),
    RgbaF16(Vec<half::f16>),
    RgbaF32(Vec<f32>),
}

impl MediaPixels {
    fn len(&self) -> usize {
        match self {
            Self::Rgba8(data) => data.len(),
            Self::Rgba16(data) => data.len(),
            Self::RgbaF16(data) => data.len(),
            Self::RgbaF32(data) => data.len(),
        }
    }
}

#[derive(Debug, Clone)]
pub struct MediaFrame {
    pub width: u32,
    pub height: u32,
    pub pixels: MediaPixels,
}

impl MediaFrame {
    pub fn rgba8(width: u32, height: u32, pixels: Vec<u8>) -> Result<Self, MediaEncodeError> {
        let frame = Self {
            width,
            height,
            pixels: MediaPixels::Rgba8(pixels),
        };
        frame.validate()?;
        Ok(frame)
    }

    pub fn validate(&self) -> Result<(), MediaEncodeError> {
        let expected = self.width as usize * self.height as usize * 4;
        if self.pixels.len() != expected {
            return Err(MediaEncodeError::InvalidFrame(format!(
                "expected {expected} RGBA samples, got {}",
                self.pixels.len()
            )));
        }
        Ok(())
    }

    pub fn to_rgba8(&self) -> Vec<u8> {
        match &self.pixels {
            MediaPixels::Rgba8(data) => data.clone(),
            MediaPixels::Rgba16(data) => data.iter().map(|&v| (v >> 8) as u8).collect(),
            MediaPixels::RgbaF16(data) => data
                .iter()
                .map(|v| (v.to_f32().clamp(0.0, 1.0) * 255.0) as u8)
                .collect(),
            MediaPixels::RgbaF32(data) => data
                .iter()
                .map(|&v| (v.clamp(0.0, 1.0) * 255.0) as u8)
                .collect(),
        }
    }

    pub fn to_rgba16(&self) -> Vec<u16> {
        match &self.pixels {
            MediaPixels::Rgba8(data) => data.iter().map(|&v| (v as u16) * 257).collect(),
            MediaPixels::Rgba16(data) => data.clone(),
            MediaPixels::RgbaF16(data) => data
                .iter()
                .map(|v| (v.to_f32().clamp(0.0, 1.0) * 65535.0) as u16)
                .collect(),
            MediaPixels::RgbaF32(data) => data
                .iter()
                .map(|&v| (v.clamp(0.0, 1.0) * 65535.0) as u16)
                .collect(),
        }
    }

    pub fn to_rgba_f32(&self) -> Vec<f32> {
        match &self.pixels {
            MediaPixels::Rgba8(data) => data.iter().map(|&v| v as f32 / 255.0).collect(),
            MediaPixels::Rgba16(data) => data.iter().map(|&v| v as f32 / 65535.0).collect(),
            MediaPixels::RgbaF16(data) => data.iter().map(|v| v.to_f32()).collect(),
            MediaPixels::RgbaF32(data) => data.clone(),
        }
    }

    pub fn to_rgb8(&self) -> Vec<u8> {
        strip_alpha(&self.to_rgba8())
    }

    pub fn to_rgb16(&self) -> Vec<u16> {
        strip_alpha(&self.to_rgba16())
    }
}

#[derive(Debug, Clone)]
pub struct FrameRequest {
    pub frame_index: u32,
    pub frame_number: u32,
    pub time_seconds: f32,
    pub max_samples: u32,
}

pub trait FrameSource {
    fn render_frame(
        &mut self,
        request: &FrameRequest,
        cancel: &AtomicBool,
    ) -> Result<MediaFrame, MediaEncodeError>;
}

#[cfg(feature = "ffmpeg")]
pub fn init_ffmpeg() -> Result<(), MediaEncodeError> {
    playa_ffmpeg::init().map_err(|err| MediaEncodeError::Ffmpeg(err.to_string()))
}

#[cfg(not(feature = "ffmpeg"))]
pub fn init_ffmpeg() -> Result<(), MediaEncodeError> {
    Err(MediaEncodeError::Ffmpeg(
        "media-encoder was built without the ffmpeg feature".to_string(),
    ))
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EncodeDialogState {
    pub output_path: String,
    pub export_mode: ExportMode,
    pub sequence: SequenceSettings,
    pub video: VideoSettings,
    #[serde(default = "default_encode_dock_state")]
    pub dock_state: DockState<EncodeDockTab>,
}

impl Default for EncodeDialogState {
    fn default() -> Self {
        Self {
            output_path: "renders/frame_####.png".to_string(),
            export_mode: ExportMode::ImageSequence,
            sequence: SequenceSettings::default(),
            video: VideoSettings::default(),
            dock_state: default_encode_dock_state(),
        }
    }
}

impl EncodeDialogState {
    pub fn set_output_path(&mut self, path: impl Into<String>) {
        self.output_path = path.into();
    }

    pub fn build_request(&self) -> EncodeRequest {
        let output_path = match self.export_mode {
            ExportMode::ImageSequence => {
                PathBuf::from(with_extension(&self.output_path, self.sequence.format.extension()))
            }
            ExportMode::Video => {
                PathBuf::from(with_extension(&self.output_path, self.video.container.extension()))
            }
        };

        EncodeRequest {
            output_path,
            export_mode: self.export_mode,
            sequence: self.sequence.clone(),
            video: self.video.clone(),
        }
    }

    pub fn ui(
        &mut self,
        ui: &mut egui::Ui,
        running: bool,
        progress: Option<&ProgressSnapshot>,
        error: Option<&str>,
    ) -> DialogResponse {
        let mut response = DialogResponse::default();

        ui.vertical(|ui| {
            ui.horizontal(|ui| {
                ui.heading(match self.export_mode {
                    ExportMode::ImageSequence => "Image Sequence Export",
                    ExportMode::Video => "Video Encoder",
                });
                ui.add_space(8.0);
                mode_button(
                    ui,
                    &mut self.export_mode,
                    ExportMode::ImageSequence,
                    running,
                );
                mode_button(ui, &mut self.export_mode, ExportMode::Video, running);
            });

            ui.add_space(4.0);
            ui.add_enabled_ui(!running, |ui| {
                ui.horizontal(|ui| {
                    ui.label("Output:");
                    ui.add(
                        egui::TextEdit::singleline(&mut self.output_path)
                            .desired_width((ui.available_width() - 36.0).max(120.0)),
                    );
                    if ui
                        .button("...")
                        .on_hover_text("Choose output path")
                        .clicked()
                    {
                        response.browse_output = true;
                    }
                });
            });

            ui.add_space(6.0);
            let mut dock_state =
                std::mem::replace(&mut self.dock_state, default_encode_dock_state());
            {
                let mut tabs = EncodeDialogTabs {
                    state: self,
                    running,
                    progress,
                    error,
                };
                DockArea::new(&mut dock_state)
                    .style(egui_dock::Style::from_egui(ui.global_style().as_ref()))
                    .show_inside(ui, &mut tabs);
            }
            self.dock_state = dock_state;

            ui.separator();
            ui.horizontal(|ui| {
                let can_start = !running && !self.output_path.trim().is_empty();
                if ui
                    .add_enabled(can_start, egui::Button::new("Start"))
                    .on_hover_text("Start render export")
                    .clicked()
                {
                    response.start = Some(self.build_request());
                }
                if ui
                    .add_enabled(running, egui::Button::new("Stop"))
                    .on_hover_text("Stop render export")
                    .clicked()
                {
                    response.stop = true;
                }
            });
        });

        response
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EncodeRequest {
    pub output_path: PathBuf,
    pub export_mode: ExportMode,
    pub sequence: SequenceSettings,
    pub video: VideoSettings,
}

impl EncodeRequest {
    pub fn sequence_job(&self) -> SequenceJob {
        SequenceJob::new(
            &self.output_path,
            self.sequence.start_frame,
            self.sequence.frame_count,
            self.sequence.max_samples,
            self.sequence.fps,
        )
    }
}

pub fn encode_image_sequence_from_source<S: FrameSource>(
    source: &mut S,
    request: &EncodeRequest,
    progress_tx: Option<std::sync::mpsc::Sender<ProgressSnapshot>>,
    cancel: Arc<AtomicBool>,
) -> Result<(), MediaEncodeError> {
    let mut progress = SequenceProgress::new(request.sequence_job());

    while !progress.is_finished() {
        if cancel.load(Ordering::Relaxed) {
            return Err(MediaEncodeError::Cancelled);
        }

        let frame_request = FrameRequest {
            frame_index: progress.current_index(),
            frame_number: progress.current_frame_number(),
            time_seconds: progress.current_frame_time_seconds(),
            max_samples: request.sequence.max_samples,
        };
        let frame = source.render_frame(&frame_request, &cancel)?;
        save_media_frame(
            progress.current_frame_path(),
            &frame,
            &request.sequence,
        )?;
        progress.observe_samples(request.sequence.max_samples);
        if let Some(tx) = &progress_tx {
            let _ = tx.send(progress.snapshot());
        }
        progress.complete_current_frame();
    }

    Ok(())
}

#[cfg(feature = "ffmpeg")]
pub fn encode_video_from_source<S: FrameSource>(
    source: &mut S,
    request: &EncodeRequest,
    progress_tx: Option<std::sync::mpsc::Sender<ProgressSnapshot>>,
    cancel: Arc<AtomicBool>,
) -> Result<(), MediaEncodeError> {
    init_ffmpeg()?;
    unsafe {
        playa_ffmpeg::ffi::av_log_set_level(playa_ffmpeg::ffi::AV_LOG_QUIET);
    }

    let mut progress = SequenceProgress::new(request.sequence_job());
    let first_request = FrameRequest {
        frame_index: progress.current_index(),
        frame_number: progress.current_frame_number(),
        time_seconds: progress.current_frame_time_seconds(),
        max_samples: request.sequence.max_samples,
    };
    let first_frame = source.render_frame(&first_request, &cancel)?;
    first_frame.validate()?;
    let width = first_frame.width;
    let height = first_frame.height;

    let mut output = ffmpeg::format::output(&request.output_path)
        .map_err(|err| MediaEncodeError::Ffmpeg(format!("create output: {err}")))?;
    let encoder_name = selected_encoder_name(request.video.codec)?;
    let codec = ffmpeg::encoder::find_by_name(encoder_name)
        .ok_or_else(|| MediaEncodeError::Ffmpeg(format!("encoder not found: {encoder_name}")))?;

    let mut encoder = ffmpeg::codec::context::Context::new_with_codec(codec)
        .encoder()
        .video()
        .map_err(|err| MediaEncodeError::Ffmpeg(format!("create video encoder: {err}")))?;
    encoder.set_width(width);
    encoder.set_height(height);

    let pixel_format = video_pixel_format(request.video.codec, encoder_name);
    encoder.set_format(pixel_format);
    let (fps_num, fps_den) = fps_to_rational(request.sequence.fps);
    encoder.set_frame_rate(Some(ffmpeg::util::rational::Rational::new(
        fps_num, fps_den,
    )));
    encoder.set_time_base(ffmpeg::util::rational::Rational::new(fps_den, fps_num));
    encoder.set_gop((request.sequence.fps.round() as u32 * 10).max(1));

    let mut options = ffmpeg::Dictionary::new();
    configure_video_quality(&mut encoder, &mut options, &request.video, encoder_name);

    let mut encoder = encoder
        .open_with(options)
        .map_err(|err| MediaEncodeError::Ffmpeg(format!("open {encoder_name}: {err}")))?;

    {
        let mut stream = output
            .add_stream(codec)
            .map_err(|err| MediaEncodeError::Ffmpeg(format!("add video stream: {err}")))?;
        stream.set_parameters(&encoder);
        stream.set_time_base(encoder.time_base());
        if request.video.codec == VideoCodec::H265
            && matches!(
                request.video.container,
                VideoContainer::Mp4 | VideoContainer::Mov
            )
        {
            unsafe {
                (*stream.parameters().as_mut_ptr()).codec_tag = u32::from_le_bytes(*b"hvc1");
            }
        }
    }

    let mut muxer_options = ffmpeg::Dictionary::new();
    if request.video.container == VideoContainer::Mp4 {
        muxer_options.set("movflags", "faststart");
    }
    output
        .write_header_with(muxer_options)
        .map_err(|err| MediaEncodeError::Ffmpeg(format!("write header: {err}")))?;

    let stream_time_base = output
        .stream(0)
        .ok_or_else(|| MediaEncodeError::Ffmpeg("missing output video stream".to_string()))?
        .time_base();
    let encoder_time_base = encoder.time_base();
    let needs_yuv = video_needs_yuv(encoder_name);
    let needs_10bit = matches!(
        pixel_format,
        ffmpeg::format::Pixel::YUV422P10LE | ffmpeg::format::Pixel::YUV420P10LE
    );
    let mut sws = if needs_yuv {
        let src_format = if needs_10bit {
            ffmpeg::format::Pixel::RGB48LE
        } else {
            ffmpeg::format::Pixel::RGB24
        };
        Some(SwsContext::new(src_format, pixel_format, width, height)?)
    } else {
        None
    };

    let mut pts = 0i64;
    encode_one_video_frame(
        &first_frame,
        &mut encoder,
        &mut output,
        sws.as_mut(),
        needs_yuv,
        needs_10bit,
        encoder_time_base,
        stream_time_base,
        pts,
    )?;
    pts += 1;
    progress.observe_samples(request.sequence.max_samples);
    if let Some(tx) = &progress_tx {
        let _ = tx.send(progress.snapshot());
    }
    progress.complete_current_frame();

    while !progress.is_finished() {
        if cancel.load(Ordering::Relaxed) {
            return Err(MediaEncodeError::Cancelled);
        }

        let frame_request = FrameRequest {
            frame_index: progress.current_index(),
            frame_number: progress.current_frame_number(),
            time_seconds: progress.current_frame_time_seconds(),
            max_samples: request.sequence.max_samples,
        };
        let frame = source.render_frame(&frame_request, &cancel)?;
        encode_one_video_frame(
            &frame,
            &mut encoder,
            &mut output,
            sws.as_mut(),
            needs_yuv,
            needs_10bit,
            encoder_time_base,
            stream_time_base,
            pts,
        )?;
        pts += 1;
        progress.observe_samples(request.sequence.max_samples);
        if let Some(tx) = &progress_tx {
            let _ = tx.send(progress.snapshot());
        }
        progress.complete_current_frame();
    }

    encoder
        .send_eof()
        .map_err(|err| MediaEncodeError::Ffmpeg(format!("flush encoder: {err}")))?;
    drain_video_packets(
        &mut encoder,
        &mut output,
        encoder_time_base,
        stream_time_base,
    )?;
    output
        .write_trailer()
        .map_err(|err| MediaEncodeError::Ffmpeg(format!("write trailer: {err}")))?;

    Ok(())
}

#[cfg(not(feature = "ffmpeg"))]
pub fn encode_video_from_source<S: FrameSource>(
    _source: &mut S,
    _request: &EncodeRequest,
    _progress_tx: Option<std::sync::mpsc::Sender<ProgressSnapshot>>,
    _cancel: Arc<AtomicBool>,
) -> Result<(), MediaEncodeError> {
    Err(MediaEncodeError::Ffmpeg(
        "media-encoder was built without the ffmpeg feature".to_string(),
    ))
}

#[derive(Debug, Default)]
pub struct DialogResponse {
    pub browse_output: bool,
    pub start: Option<EncodeRequest>,
    pub stop: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum EncodeDockTab {
    Frames,
    Image,
    Video,
    Progress,
}

pub fn default_encode_dock_state() -> DockState<EncodeDockTab> {
    let mut dock_state = DockState::new(vec![EncodeDockTab::Frames]);
    let [_frames, image] = dock_state.main_surface_mut().split_right(
        NodeIndex::root(),
        0.45,
        vec![EncodeDockTab::Image],
    );
    let [_image, video] =
        dock_state
            .main_surface_mut()
            .split_below(image, 0.50, vec![EncodeDockTab::Video]);
    let _ = dock_state
        .main_surface_mut()
        .split_below(video, 0.50, vec![EncodeDockTab::Progress]);
    dock_state
}

struct EncodeDialogTabs<'a> {
    state: &'a mut EncodeDialogState,
    running: bool,
    progress: Option<&'a ProgressSnapshot>,
    error: Option<&'a str>,
}

impl TabViewer for EncodeDialogTabs<'_> {
    type Tab = EncodeDockTab;

    fn title(&mut self, tab: &mut EncodeDockTab) -> egui::WidgetText {
        match tab {
            EncodeDockTab::Frames => "Frames".into(),
            EncodeDockTab::Image => "Image".into(),
            EncodeDockTab::Video => "Video".into(),
            EncodeDockTab::Progress => "Progress".into(),
        }
    }

    fn ui(&mut self, ui: &mut egui::Ui, tab: &mut EncodeDockTab) {
        match tab {
            EncodeDockTab::Frames => frame_settings_ui(ui, self.state, self.running),
            EncodeDockTab::Image => image_settings_ui(ui, self.state, self.running),
            EncodeDockTab::Video => video_settings_ui(ui, self.state, self.running),
            EncodeDockTab::Progress => progress_ui(ui, self.progress, self.error),
        }
    }
}

fn mode_button(ui: &mut egui::Ui, mode: &mut ExportMode, value: ExportMode, running: bool) {
    if ui
        .add_enabled(
            !running,
            egui::Button::new(value.label()).selected(*mode == value),
        )
        .clicked()
    {
        *mode = value;
    }
}

fn normalize_sequence_capabilities(sequence: &mut SequenceSettings) {
    if !sequence.format.supported_depths().contains(&sequence.bit_depth) {
        sequence.bit_depth = sequence.format.supported_depths()[0];
    }
    if !sequence.format.supports_alpha() && sequence.channels == ChannelMode::Rgba {
        sequence.channels = ChannelMode::Rgb;
    }
}

fn with_extension(path: &str, extension: &str) -> String {
    let mut path = PathBuf::from(path);
    path.set_extension(extension);
    path.to_string_lossy().to_string()
}

fn frame_settings_ui(ui: &mut egui::Ui, state: &mut EncodeDialogState, running: bool) {
    ui.add_enabled_ui(!running, |ui| {
        egui::Grid::new("imageseq_frames_grid")
            .num_columns(2)
            .spacing([8.0, 6.0])
            .show(ui, |ui| {
                ui.label("Start Frame:");
                ui.add(egui::DragValue::new(&mut state.sequence.start_frame).speed(1));
                ui.end_row();

                ui.label("Frame Count:");
                ui.add(
                    egui::DragValue::new(&mut state.sequence.frame_count)
                        .range(1..=100_000)
                        .speed(1),
                );
                ui.end_row();

                ui.label("FPS:");
                ui.add(
                    egui::DragValue::new(&mut state.sequence.fps)
                        .range(1.0..=240.0)
                        .speed(1.0),
                );
                ui.end_row();

                ui.label("Max Samples:");
                ui.add(
                    egui::DragValue::new(&mut state.sequence.max_samples)
                        .range(1..=1_000_000)
                        .speed(16),
                );
                ui.end_row();
            });
    });
}

fn image_settings_ui(ui: &mut egui::Ui, state: &mut EncodeDialogState, running: bool) {
    ui.add_enabled_ui(!running, |ui| {
        egui::Grid::new("imageseq_image_grid")
            .num_columns(2)
            .spacing([8.0, 6.0])
            .show(ui, |ui| {
                ui.label("Format:");
                egui::ComboBox::from_id_salt("imageseq_sequence_format")
                    .selected_text(state.sequence.format.label())
                    .show_ui(ui, |ui| {
                        for format in SequenceFormat::all() {
                            if ui
                                .selectable_value(
                                    &mut state.sequence.format,
                                    *format,
                                    format.label(),
                                )
                                .clicked()
                            {
                                state.output_path =
                                    with_extension(&state.output_path, format.extension());
                            }
                        }
                    });
                ui.end_row();

                ui.label("Channels:");
                egui::ComboBox::from_id_salt("imageseq_channels")
                    .selected_text(state.sequence.channels.label())
                    .show_ui(ui, |ui| {
                        for channels in ChannelMode::all() {
                            let enabled = *channels != ChannelMode::Rgba
                                || state.sequence.format.supports_alpha();
                            ui.add_enabled_ui(enabled, |ui| {
                                ui.selectable_value(
                                    &mut state.sequence.channels,
                                    *channels,
                                    channels.label(),
                                );
                            });
                        }
                    });
                ui.end_row();

                ui.label("Bit Depth:");
                egui::ComboBox::from_id_salt("imageseq_bit_depth")
                    .selected_text(state.sequence.bit_depth.label())
                    .show_ui(ui, |ui| {
                        for depth in OutputBitDepth::all() {
                            let enabled = state.sequence.format.supported_depths().contains(depth);
                            ui.add_enabled_ui(enabled, |ui| {
                                ui.selectable_value(
                                    &mut state.sequence.bit_depth,
                                    *depth,
                                    depth.label(),
                                );
                            });
                        }
                    });
                ui.end_row();
            });

        normalize_sequence_capabilities(&mut state.sequence);

        ui.add_space(6.0);
        match state.sequence.format {
            SequenceFormat::Exr => exr_settings_ui(ui, &mut state.sequence),
            SequenceFormat::Png => {
                ui.horizontal(|ui| {
                    ui.label("PNG Compression:");
                    ui.add(
                        egui::Slider::new(
                            &mut state.sequence.format_settings.png.compression,
                            0..=9,
                        )
                        .text("level"),
                    );
                });
            }
            SequenceFormat::Jpeg => {
                ui.horizontal(|ui| {
                    ui.label("JPEG Quality:");
                    ui.add(
                        egui::Slider::new(
                            &mut state.sequence.format_settings.jpeg.quality,
                            1..=100,
                        )
                        .text("%"),
                    );
                });
            }
            SequenceFormat::Tiff => {
                ui.horizontal(|ui| {
                    ui.label("TIFF Compression:");
                    egui::ComboBox::from_id_salt("imageseq_tiff_compression")
                        .selected_text(state.sequence.format_settings.tiff.compression.label())
                        .show_ui(ui, |ui| {
                            for compression in TiffCompression::all() {
                                ui.selectable_value(
                                    &mut state.sequence.format_settings.tiff.compression,
                                    *compression,
                                    compression.label(),
                                );
                            }
                        });
                });
            }
            SequenceFormat::Tga => {
                ui.checkbox(
                    &mut state.sequence.format_settings.tga.rle_compression,
                    "RLE Compression",
                );
            }
        }
    });
}

fn exr_settings_ui(ui: &mut egui::Ui, sequence: &mut SequenceSettings) {
    ui.horizontal(|ui| {
        ui.label("EXR Compression:");
        egui::ComboBox::from_id_salt("imageseq_exr_compression")
            .selected_text(sequence.format_settings.exr.compression.label())
            .show_ui(ui, |ui| {
                for compression in ExrCompression::all() {
                    ui.selectable_value(
                        &mut sequence.format_settings.exr.compression,
                        *compression,
                        compression.label(),
                    );
                }
            });
    });

    if sequence
        .format_settings
        .exr
        .compression
        .has_quality_knob()
    {
        ui.horizontal(|ui| {
            ui.label("DWA Loss:");
            ui.add(
                egui::Slider::new(&mut sequence.format_settings.exr.dwa_quality, 0.0..=200.0)
                    .text("45 default"),
            );
        });
    }
}

fn video_settings_ui(ui: &mut egui::Ui, state: &mut EncodeDialogState, running: bool) {
    ui.add_enabled_ui(!running, |ui| {
        egui::Grid::new("imageseq_video_grid")
            .num_columns(2)
            .spacing([8.0, 6.0])
            .show(ui, |ui| {
                ui.label("Container:");
                egui::ComboBox::from_id_salt("imageseq_video_container")
                    .selected_text(state.video.container.label())
                    .show_ui(ui, |ui| {
                        for value in VideoContainer::all() {
                            if ui
                                .selectable_value(
                                    &mut state.video.container,
                                    *value,
                                    value.label(),
                                )
                                .clicked()
                            {
                                state.output_path =
                                    with_extension(&state.output_path, value.extension());
                            }
                        }
                    });
                ui.end_row();

                ui.label("Codec:");
                egui::ComboBox::from_id_salt("imageseq_video_codec")
                    .selected_text(state.video.codec.label())
                    .show_ui(ui, |ui| {
                        for value in VideoCodec::all() {
                            if ui
                                .selectable_value(&mut state.video.codec, *value, value.label())
                                .clicked()
                            {
                                state.video.container = value.preferred_container();
                                state.output_path = with_extension(
                                    &state.output_path,
                                    state.video.container.extension(),
                                );
                            }
                        }
                    });
                ui.end_row();

                ui.label("Quality:");
                egui::ComboBox::from_id_salt("imageseq_video_quality")
                    .selected_text(state.video.quality_mode.label())
                    .show_ui(ui, |ui| {
                        for value in QualityMode::all() {
                            ui.selectable_value(
                                &mut state.video.quality_mode,
                                *value,
                                value.label(),
                            );
                        }
                    });
                ui.end_row();

                match state.video.quality_mode {
                    QualityMode::Crf => {
                        ui.label("CRF:");
                        ui.add(egui::Slider::new(&mut state.video.crf, 0..=51));
                    }
                    QualityMode::Bitrate => {
                        ui.label("Bitrate:");
                        ui.add(
                            egui::DragValue::new(&mut state.video.bitrate_mbps)
                                .range(1.0..=1000.0)
                                .speed(1.0)
                                .suffix(" Mbps"),
                        );
                    }
                }
                ui.end_row();
            });
    });
}

fn progress_ui(ui: &mut egui::Ui, progress: Option<&ProgressSnapshot>, error: Option<&str>) {
    if let Some(progress) = progress {
        ui.add(
            egui::ProgressBar::new(progress.fraction)
                .show_percentage()
                .text(format!(
                    "Frame {} of {}",
                    progress
                        .completed_frames
                        .saturating_add(1)
                        .min(progress.total_frames),
                    progress.total_frames
                )),
        );
        egui::Grid::new("imageseq_progress_grid")
            .num_columns(2)
            .spacing([8.0, 4.0])
            .show(ui, |ui| {
                ui.label("Current Frame:");
                ui.label(progress.current_frame_number.to_string());
                ui.end_row();

                ui.label("Samples:");
                ui.label(format!(
                    "{} / {}",
                    progress.current_samples, progress.max_samples
                ));
                ui.end_row();

                ui.label("Elapsed:");
                ui.label(format_duration_hms(progress.elapsed));
                ui.end_row();

                ui.label("ETA:");
                ui.label(
                    progress
                        .eta
                        .map(format_duration_hms)
                        .unwrap_or_else(|| "--:--".to_string()),
                );
                ui.end_row();
            });
    } else {
        ui.add(egui::ProgressBar::new(0.0).text("Idle"));
    }

    if let Some(error) = error {
        ui.separator();
        ui.colored_label(egui::Color32::from_rgb(220, 80, 70), error);
    }
}

pub fn normalize_template_path(path: PathBuf) -> PathBuf {
    let filename = path
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or_default();

    if contains_sequence_marker(filename) {
        return ensure_extension(path);
    }

    let parent = path.parent().map(Path::to_path_buf);
    let stem = path
        .file_stem()
        .and_then(|stem| stem.to_str())
        .filter(|stem| !stem.is_empty())
        .unwrap_or("frame");
    let ext = path
        .extension()
        .and_then(|ext| ext.to_str())
        .filter(|ext| !ext.is_empty())
        .unwrap_or(DEFAULT_EXTENSION);

    let filename = format!("{stem}_{DEFAULT_MARKER}.{ext}");
    match parent {
        Some(dir) => dir.join(filename),
        None => PathBuf::from(filename),
    }
}

pub fn expand_frame_path(template: &Path, frame_number: u32) -> PathBuf {
    let Some(filename) = template.file_name().and_then(|name| name.to_str()) else {
        return template.to_path_buf();
    };
    let expanded = expand_marker(filename, frame_number);
    match template.parent() {
        Some(dir) => dir.join(expanded),
        None => PathBuf::from(expanded),
    }
}

pub fn save_media_frame(
    path: impl AsRef<Path>,
    frame: &MediaFrame,
    settings: &SequenceSettings,
) -> Result<(), MediaEncodeError> {
    frame.validate()?;
    let path = path.as_ref();
    if let Some(parent) = path.parent() {
        if !parent.as_os_str().is_empty() {
            std::fs::create_dir_all(parent)?;
        }
    }

    match settings.format {
        SequenceFormat::Exr => write_exr_frame(path, frame, settings),
        SequenceFormat::Png => write_png_frame(path, frame, settings),
        SequenceFormat::Jpeg => write_jpeg_frame(path, frame, settings),
        SequenceFormat::Tiff => write_tiff_frame(path, frame, settings),
        SequenceFormat::Tga => write_tga_frame(path, frame, settings),
    }
}

pub fn save_rgba_png(
    path: impl AsRef<Path>,
    width: u32,
    height: u32,
    pixels: &[u8],
) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    let frame = MediaFrame::rgba8(width, height, pixels.to_vec())?;
    save_media_frame(path, &frame, &SequenceSettings::default())?;
    Ok(())
}

fn write_png_frame(
    path: &Path,
    frame: &MediaFrame,
    settings: &SequenceSettings,
) -> Result<(), MediaEncodeError> {
    use image::ImageEncoder;
    use image::codecs::png::{CompressionType, FilterType, PngEncoder};

    let file = std::fs::File::create(path)?;
    let writer = std::io::BufWriter::new(file);
    let compression = match settings.format_settings.png.compression {
        0..=3 => CompressionType::Fast,
        4..=6 => CompressionType::Default,
        _ => CompressionType::Best,
    };
    let encoder = PngEncoder::new_with_quality(writer, compression, FilterType::Adaptive);

    match settings.bit_depth {
        OutputBitDepth::U8 => {
            let rgba = frame.to_rgba8();
            match settings.channels {
                ChannelMode::Rgba => encoder.write_image(
                    &rgba,
                    frame.width,
                    frame.height,
                    image::ExtendedColorType::Rgba8,
                ),
                ChannelMode::Rgb => encoder.write_image(
                    &strip_alpha(&rgba),
                    frame.width,
                    frame.height,
                    image::ExtendedColorType::Rgb8,
                ),
            }
        }
        OutputBitDepth::U16 | OutputBitDepth::F16 | OutputBitDepth::F32 => {
            let rgba = frame.to_rgba16();
            match settings.channels {
                ChannelMode::Rgba => encoder.write_image(
                    bytemuck::cast_slice(&rgba),
                    frame.width,
                    frame.height,
                    image::ExtendedColorType::Rgba16,
                ),
                ChannelMode::Rgb => encoder.write_image(
                    bytemuck::cast_slice(&strip_alpha(&rgba)),
                    frame.width,
                    frame.height,
                    image::ExtendedColorType::Rgb16,
                ),
            }
        }
    }
    .map_err(|err| MediaEncodeError::Image(format!("PNG encode failed: {err}")))
}

fn write_jpeg_frame(
    path: &Path,
    frame: &MediaFrame,
    settings: &SequenceSettings,
) -> Result<(), MediaEncodeError> {
    use image::ImageEncoder;
    use image::codecs::jpeg::JpegEncoder;

    let file = std::fs::File::create(path)?;
    let writer = std::io::BufWriter::new(file);
    let encoder = JpegEncoder::new_with_quality(writer, settings.format_settings.jpeg.quality);
    encoder
        .write_image(
            &frame.to_rgb8(),
            frame.width,
            frame.height,
            image::ExtendedColorType::Rgb8,
        )
        .map_err(|err| MediaEncodeError::Image(format!("JPEG encode failed: {err}")))
}

fn write_tiff_frame(
    path: &Path,
    frame: &MediaFrame,
    settings: &SequenceSettings,
) -> Result<(), MediaEncodeError> {
    use image::{ImageBuffer, Rgb, Rgba};

    match settings.bit_depth {
        OutputBitDepth::U8 => match settings.channels {
            ChannelMode::Rgba => {
                let img: ImageBuffer<Rgba<u8>, Vec<u8>> =
                    ImageBuffer::from_raw(frame.width, frame.height, frame.to_rgba8())
                        .ok_or_else(|| MediaEncodeError::InvalidFrame("TIFF RGBA8".into()))?;
                img.save(path)
            }
            ChannelMode::Rgb => {
                let img: ImageBuffer<Rgb<u8>, Vec<u8>> =
                    ImageBuffer::from_raw(frame.width, frame.height, frame.to_rgb8())
                        .ok_or_else(|| MediaEncodeError::InvalidFrame("TIFF RGB8".into()))?;
                img.save(path)
            }
        },
        OutputBitDepth::U16 | OutputBitDepth::F16 | OutputBitDepth::F32 => match settings.channels {
            ChannelMode::Rgba => {
                let img: ImageBuffer<Rgba<u16>, Vec<u16>> =
                    ImageBuffer::from_raw(frame.width, frame.height, frame.to_rgba16())
                        .ok_or_else(|| MediaEncodeError::InvalidFrame("TIFF RGBA16".into()))?;
                img.save(path)
            }
            ChannelMode::Rgb => {
                let img: ImageBuffer<Rgb<u16>, Vec<u16>> =
                    ImageBuffer::from_raw(frame.width, frame.height, frame.to_rgb16())
                        .ok_or_else(|| MediaEncodeError::InvalidFrame("TIFF RGB16".into()))?;
                img.save(path)
            }
        },
    }
    .map_err(|err| MediaEncodeError::Image(format!("TIFF encode failed: {err}")))
}

fn write_tga_frame(
    path: &Path,
    frame: &MediaFrame,
    settings: &SequenceSettings,
) -> Result<(), MediaEncodeError> {
    use image::{ImageBuffer, Rgb, Rgba};

    match settings.channels {
        ChannelMode::Rgba => {
            let img: ImageBuffer<Rgba<u8>, Vec<u8>> =
                ImageBuffer::from_raw(frame.width, frame.height, frame.to_rgba8())
                    .ok_or_else(|| MediaEncodeError::InvalidFrame("TGA RGBA8".into()))?;
            img.save(path)
        }
        ChannelMode::Rgb => {
            let img: ImageBuffer<Rgb<u8>, Vec<u8>> =
                ImageBuffer::from_raw(frame.width, frame.height, frame.to_rgb8())
                    .ok_or_else(|| MediaEncodeError::InvalidFrame("TGA RGB8".into()))?;
            img.save(path)
        }
    }
    .map_err(|err| {
        let _ = settings.format_settings.tga.rle_compression;
        MediaEncodeError::Image(format!("TGA encode failed: {err}"))
    })
}

#[cfg(feature = "exr")]
fn write_exr_frame(
    path: &Path,
    frame: &MediaFrame,
    settings: &SequenceSettings,
) -> Result<(), MediaEncodeError> {
    use vfx_core::AttrValue;
    use vfx_io::{
        ChannelKind, ChannelSampleType, ChannelSamples, ImageChannel, ImageLayer, LayeredImage,
        Metadata,
    };

    let f32_data = frame.to_rgba_f32();
    let pixel_count = frame.width as usize * frame.height as usize;
    let n_out = match settings.channels {
        ChannelMode::Rgba => 4,
        ChannelMode::Rgb => 3,
    };
    let sample_type = match settings.bit_depth {
        OutputBitDepth::F32 => ChannelSampleType::F32,
        OutputBitDepth::U8 | OutputBitDepth::U16 | OutputBitDepth::F16 => ChannelSampleType::F16,
    };

    let mut planar: Vec<Vec<f32>> = (0..n_out)
        .map(|_| Vec::with_capacity(pixel_count))
        .collect();
    for px in 0..pixel_count {
        let base = px * 4;
        for channel in 0..n_out {
            planar[channel].push(f32_data[base + channel]);
        }
    }

    let names = ["R", "G", "B", "A"];
    let kinds = [
        ChannelKind::Color,
        ChannelKind::Color,
        ChannelKind::Color,
        ChannelKind::Alpha,
    ];
    let mut channels = Vec::with_capacity(n_out);
    for channel in 0..n_out {
        channels.push(ImageChannel {
            name: names[channel].to_string(),
            kind: kinds[channel],
            sample_type,
            samples: ChannelSamples::F32(std::mem::take(&mut planar[channel])),
            sampling: (1, 1),
            quantize_linearly: channel == 3,
        });
    }

    let mut layer = ImageLayer {
        name: String::new(),
        width: frame.width,
        height: frame.height,
        channels,
        ..Default::default()
    };
    layer.spec.attributes.insert(
        "compression".to_string(),
        AttrValue::String(
            settings
                .format_settings
                .exr
                .compression
                .to_oiio_string(settings.format_settings.exr.dwa_quality),
        ),
    );

    let layered = LayeredImage {
        layers: vec![layer],
        metadata: Metadata::default(),
    };

    vfx_io::exr::write_layers(path, &layered)
        .map_err(|err| MediaEncodeError::Exr(err.to_string()))
}

#[cfg(not(feature = "exr"))]
fn write_exr_frame(
    _path: &Path,
    _frame: &MediaFrame,
    _settings: &SequenceSettings,
) -> Result<(), MediaEncodeError> {
    Err(MediaEncodeError::Exr(
        "media-encoder was built without the exr feature".to_string(),
    ))
}

#[cfg(feature = "ffmpeg")]
fn selected_encoder_name(codec: VideoCodec) -> Result<&'static str, MediaEncodeError> {
    let preferred = match codec {
        VideoCodec::H264 => {
            #[cfg(target_os = "macos")]
            {
                if ffmpeg::encoder::find_by_name("h264_videotoolbox").is_some() {
                    return Ok("h264_videotoolbox");
                }
            }
            ["h264_nvenc", "h264_qsv", "h264_amf", "libx264"].as_slice()
        }
        VideoCodec::H265 => {
            #[cfg(target_os = "macos")]
            {
                if ffmpeg::encoder::find_by_name("hevc_videotoolbox").is_some() {
                    return Ok("hevc_videotoolbox");
                }
            }
            ["hevc_nvenc", "hevc_qsv", "hevc_amf", "libx265"].as_slice()
        }
        VideoCodec::ProRes => ["prores_ks"].as_slice(),
        VideoCodec::Av1 => ["av1_nvenc", "av1_qsv", "av1_amf", "libsvtav1", "libaom-av1"].as_slice(),
    };

    preferred
        .iter()
        .copied()
        .find(|name| ffmpeg::encoder::find_by_name(name).is_some())
        .ok_or_else(|| MediaEncodeError::Ffmpeg(format!("no encoder available for {}", codec.label())))
}

#[cfg(feature = "ffmpeg")]
fn video_needs_yuv(encoder_name: &str) -> bool {
    matches!(
        encoder_name,
        "h264_nvenc"
            | "hevc_nvenc"
            | "av1_nvenc"
            | "h264_qsv"
            | "hevc_qsv"
            | "av1_qsv"
            | "h264_amf"
            | "hevc_amf"
            | "av1_amf"
            | "h264_videotoolbox"
            | "hevc_videotoolbox"
            | "libsvtav1"
            | "libaom-av1"
            | "prores_ks"
    )
}

#[cfg(feature = "ffmpeg")]
fn video_pixel_format(codec: VideoCodec, encoder_name: &str) -> ffmpeg::format::Pixel {
    if codec == VideoCodec::ProRes || encoder_name == "prores_ks" {
        ffmpeg::format::Pixel::YUV422P10LE
    } else if codec == VideoCodec::H265 {
        ffmpeg::format::Pixel::YUV420P
    } else if video_needs_yuv(encoder_name) {
        ffmpeg::format::Pixel::YUV420P
    } else {
        ffmpeg::format::Pixel::RGB24
    }
}

#[cfg(feature = "ffmpeg")]
fn configure_video_quality(
    encoder: &mut ffmpeg::codec::encoder::video::Video,
    options: &mut ffmpeg::Dictionary,
    settings: &VideoSettings,
    encoder_name: &str,
) {
    match settings.quality_mode {
        QualityMode::Crf => match encoder_name {
            "h264_nvenc" | "hevc_nvenc" => {
                options.set("rc", "constqp");
                options.set("cq", &settings.crf.to_string());
                options.set("preset", "p4");
                options.set("forced-idr", "1");
                options.set("no-scenecut", "1");
            }
            "libx264" => {
                options.set("crf", &settings.crf.to_string());
                options.set("preset", "medium");
                options.set("profile", "high");
            }
            "libx265" => {
                options.set("crf", &settings.crf.to_string());
                options.set("preset", "medium");
                options.set("profile", "main");
            }
            "h264_qsv" | "hevc_qsv" | "av1_qsv" => {
                options.set("global_quality", &settings.crf.to_string());
            }
            "h264_amf" | "hevc_amf" | "av1_amf" => {
                options.set("rc", "cqp");
                options.set("qp", &settings.crf.to_string());
            }
            "h264_videotoolbox" | "hevc_videotoolbox" => {
                let bitrate_kbps = if settings.crf <= 18 {
                    10_000
                } else if settings.crf <= 23 {
                    5_000
                } else {
                    2_500
                };
                encoder.set_bit_rate(bitrate_kbps * 1000);
            }
            "av1_nvenc" => {
                options.set("rc", "constqp");
                options.set("qp", &settings.crf.to_string());
                options.set("preset", "p4");
            }
            "libsvtav1" => {
                options.set("crf", &settings.crf.to_string());
                options.set("preset", "6");
            }
            "libaom-av1" => {
                options.set("crf", &settings.crf.to_string());
                options.set("cpu-used", "6");
            }
            "prores_ks" => {
                options.set("profile", "2");
                options.set("vendor", "apl0");
            }
            _ => {
                options.set("crf", &settings.crf.to_string());
            }
        },
        QualityMode::Bitrate => {
            encoder.set_bit_rate((settings.bitrate_mbps * 1_000_000.0) as usize);
        }
    }
}

#[cfg(feature = "ffmpeg")]
fn encode_one_video_frame(
    frame: &MediaFrame,
    encoder: &mut ffmpeg::codec::encoder::video::Encoder,
    output: &mut ffmpeg::format::context::Output,
    sws: Option<&mut SwsContext>,
    needs_yuv: bool,
    needs_10bit: bool,
    encoder_time_base: ffmpeg::Rational,
    stream_time_base: ffmpeg::Rational,
    pts: i64,
) -> Result<(), MediaEncodeError> {
    let width = frame.width;
    let height = frame.height;
    let mut ffmpeg_frame = if needs_10bit {
        let sws = sws.ok_or_else(|| MediaEncodeError::Ffmpeg("missing swscale context".to_string()))?;
        sws.convert_rgb48(&frame.to_rgb16(), width, height)?
    } else if needs_yuv {
        let sws = sws.ok_or_else(|| MediaEncodeError::Ffmpeg("missing swscale context".to_string()))?;
        sws.convert(&frame.to_rgb8(), width, height)?
    } else {
        rgb24_video_frame(&frame.to_rgb8(), width, height)?
    };

    ffmpeg_frame.set_pts(Some(pts));
    encoder
        .send_frame(&ffmpeg_frame)
        .map_err(|err| MediaEncodeError::Ffmpeg(format!("send frame: {err}")))?;
    drain_video_packets(encoder, output, encoder_time_base, stream_time_base)
}

#[cfg(feature = "ffmpeg")]
fn drain_video_packets(
    encoder: &mut ffmpeg::codec::encoder::video::Encoder,
    output: &mut ffmpeg::format::context::Output,
    encoder_time_base: ffmpeg::Rational,
    stream_time_base: ffmpeg::Rational,
) -> Result<(), MediaEncodeError> {
    let mut encoded = ffmpeg::Packet::empty();
    while encoder.receive_packet(&mut encoded).is_ok() {
        encoded.set_stream(0);
        encoded.rescale_ts(encoder_time_base, stream_time_base);
        encoded.set_duration(1);
        if encoded.dts().is_none() {
            if let Some(pts) = encoded.pts() {
                encoded.set_dts(Some(pts));
            }
        }
        encoded
            .write_interleaved(output)
            .map_err(|err| MediaEncodeError::Ffmpeg(format!("write packet: {err}")))?;
    }
    Ok(())
}

#[cfg(feature = "ffmpeg")]
fn rgb24_video_frame(
    rgb24: &[u8],
    width: u32,
    height: u32,
) -> Result<ffmpeg::util::frame::video::Video, MediaEncodeError> {
    let expected = width as usize * height as usize * 3;
    if rgb24.len() != expected {
        return Err(MediaEncodeError::InvalidFrame(format!(
            "expected {expected} RGB samples, got {}",
            rgb24.len()
        )));
    }

    let mut frame = ffmpeg::util::frame::video::Video::new(
        ffmpeg::format::Pixel::RGB24,
        width,
        height,
    );
    let dst_stride = frame.stride(0);
    let src_stride = width as usize * 3;
    {
        let data = frame.data_mut(0);
        for y in 0..height as usize {
            let src_offset = y * src_stride;
            let dst_offset = y * dst_stride;
            data[dst_offset..dst_offset + src_stride]
                .copy_from_slice(&rgb24[src_offset..src_offset + src_stride]);
        }
    }
    Ok(frame)
}

#[cfg(feature = "ffmpeg")]
fn fps_to_rational(fps: f32) -> (i32, i32) {
    for &(target, num, den) in &[
        (23.976, 24000, 1001),
        (29.97, 30000, 1001),
        (47.952, 48000, 1001),
        (59.94, 60000, 1001),
        (119.88, 120000, 1001),
    ] {
        if (fps - target).abs() < 0.01 {
            return (num, den);
        }
    }
    let rounded = fps.round() as i32;
    if (fps - rounded as f32).abs() < 0.001 {
        (rounded.max(1), 1)
    } else {
        (((fps * 1000.0).round() as i32).max(1), 1000)
    }
}

#[cfg(feature = "ffmpeg")]
pub struct SwsContext {
    ctx: Option<ffmpeg::software::scaling::Context>,
    src_format: ffmpeg::format::Pixel,
    dst_format: ffmpeg::format::Pixel,
    width: u32,
    height: u32,
}

#[cfg(feature = "ffmpeg")]
impl SwsContext {
    pub fn new(
        src_format: ffmpeg::format::Pixel,
        dst_format: ffmpeg::format::Pixel,
        width: u32,
        height: u32,
    ) -> Result<Self, MediaEncodeError> {
        let ctx = ffmpeg::software::scaling::Context::get(
            src_format,
            width,
            height,
            dst_format,
            width,
            height,
            ffmpeg::software::scaling::Flags::BILINEAR,
        )
        .map_err(|err| MediaEncodeError::Ffmpeg(format!("create swscale context: {err}")))?;

        Ok(Self {
            ctx: Some(ctx),
            src_format,
            dst_format,
            width,
            height,
        })
    }

    pub fn convert(
        &mut self,
        rgb24: &[u8],
        width: u32,
        height: u32,
    ) -> Result<ffmpeg::util::frame::video::Video, MediaEncodeError> {
        if self.width != width || self.height != height {
            self.recreate(width, height)?;
        }
        let src = rgb24_video_frame(rgb24, width, height)?;
        let mut dst = ffmpeg::util::frame::video::Video::new(self.dst_format, width, height);
        self.ctx
            .as_mut()
            .ok_or_else(|| MediaEncodeError::Ffmpeg("swscale context is not initialized".to_string()))?
            .run(&src, &mut dst)
            .map_err(|err| MediaEncodeError::Ffmpeg(format!("RGB24 to YUV conversion: {err}")))?;
        Ok(dst)
    }

    pub fn convert_rgb48(
        &mut self,
        rgb48: &[u16],
        width: u32,
        height: u32,
    ) -> Result<ffmpeg::util::frame::video::Video, MediaEncodeError> {
        let expected = width as usize * height as usize * 3;
        if rgb48.len() != expected {
            return Err(MediaEncodeError::InvalidFrame(format!(
                "expected {expected} RGB48 samples, got {}",
                rgb48.len()
            )));
        }
        if self.width != width || self.height != height {
            self.recreate(width, height)?;
        }

        let mut src =
            ffmpeg::util::frame::video::Video::new(ffmpeg::format::Pixel::RGB48LE, width, height);
        let stride = src.stride(0);
        let row_pixels = width as usize;
        {
            let data = src.data_mut(0);
            for y in 0..height as usize {
                for x in 0..row_pixels {
                    let src_idx = (y * row_pixels + x) * 3;
                    let dst_idx = y * stride + x * 6;
                    data[dst_idx..dst_idx + 2].copy_from_slice(&rgb48[src_idx].to_le_bytes());
                    data[dst_idx + 2..dst_idx + 4]
                        .copy_from_slice(&rgb48[src_idx + 1].to_le_bytes());
                    data[dst_idx + 4..dst_idx + 6]
                        .copy_from_slice(&rgb48[src_idx + 2].to_le_bytes());
                }
            }
        }

        let mut dst = ffmpeg::util::frame::video::Video::new(self.dst_format, width, height);
        self.ctx
            .as_mut()
            .ok_or_else(|| MediaEncodeError::Ffmpeg("swscale context is not initialized".to_string()))?
            .run(&src, &mut dst)
            .map_err(|err| MediaEncodeError::Ffmpeg(format!("RGB48 to YUV conversion: {err}")))?;
        Ok(dst)
    }

    fn recreate(&mut self, width: u32, height: u32) -> Result<(), MediaEncodeError> {
        self.ctx = Some(
            ffmpeg::software::scaling::Context::get(
                self.src_format,
                width,
                height,
                self.dst_format,
                width,
                height,
                ffmpeg::software::scaling::Flags::BILINEAR,
            )
            .map_err(|err| MediaEncodeError::Ffmpeg(format!("recreate swscale context: {err}")))?,
        );
        self.width = width;
        self.height = height;
        Ok(())
    }
}

fn strip_alpha<T: Copy>(rgba: &[T]) -> Vec<T> {
    let mut rgb = Vec::with_capacity(rgba.len() / 4 * 3);
    for chunk in rgba.chunks_exact(4) {
        rgb.push(chunk[0]);
        rgb.push(chunk[1]);
        rgb.push(chunk[2]);
    }
    rgb
}

pub fn format_duration_hms(duration: Duration) -> String {
    let secs = duration.as_secs();
    let hours = secs / 3600;
    let minutes = (secs % 3600) / 60;
    let seconds = secs % 60;
    if hours > 0 {
        format!("{hours:02}:{minutes:02}:{seconds:02}")
    } else {
        format!("{minutes:02}:{seconds:02}")
    }
}

fn contains_sequence_marker(filename: &str) -> bool {
    filename.contains('#') || filename.contains('@')
}

fn ensure_extension(path: PathBuf) -> PathBuf {
    if path.extension().is_some() {
        path
    } else {
        path.with_extension(DEFAULT_EXTENSION)
    }
}

fn expand_marker(filename: &str, frame_number: u32) -> String {
    let mut chars = filename.char_indices().peekable();
    while let Some((start, ch)) = chars.next() {
        if ch != '#' && ch != '@' {
            continue;
        }

        let marker = ch;
        let mut end = start + ch.len_utf8();
        let mut width = 1usize;
        while let Some(&(idx, next)) = chars.peek() {
            if next != marker {
                break;
            }
            chars.next();
            end = idx + next.len_utf8();
            width += 1;
        }

        let mut out = String::with_capacity(filename.len() + 8);
        out.push_str(&filename[..start]);
        out.push_str(&format!("{frame_number:0width$}"));
        out.push_str(&filename[end..]);
        return out;
    }

    filename.to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn normalize_adds_after_effects_style_marker_before_extension() {
        assert_eq!(
            normalize_template_path(PathBuf::from("renders/frame.png")),
            PathBuf::from("renders/frame_####.png")
        );
    }

    #[test]
    fn normalize_keeps_existing_marker() {
        assert_eq!(
            normalize_template_path(PathBuf::from("renders/frame_##.png")),
            PathBuf::from("renders/frame_##.png")
        );
    }

    #[test]
    fn expand_hash_marker_with_padding() {
        assert_eq!(
            expand_frame_path(Path::new("renders/frame_####.png"), 42),
            PathBuf::from("renders/frame_0042.png")
        );
    }

    #[test]
    fn expand_at_marker_with_padding() {
        assert_eq!(
            expand_frame_path(Path::new("renders/frame_@@@.png"), 7),
            PathBuf::from("renders/frame_007.png")
        );
    }
}
