//! Reusable image sequence and render-export UI.
//!
//! The crate owns path-template normalization, frame path expansion, progress/ETA
//! accounting, encoder-facing settings, and an egui/egui-dock dialog. Host
//! applications provide rendered RGBA frames and drive their own render loop.

use egui_dock::{DockArea, DockState, NodeIndex, TabViewer};
use serde::{Deserialize, Serialize};
use std::collections::VecDeque;
use std::path::{Path, PathBuf};
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
    Png,
}

impl SequenceFormat {
    pub fn label(self) -> &'static str {
        match self {
            Self::Png => "PNG",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum VideoContainer {
    Mp4,
    Mov,
    Mkv,
}

impl VideoContainer {
    pub fn label(self) -> &'static str {
        match self {
            Self::Mp4 => "MP4",
            Self::Mov => "MOV",
            Self::Mkv => "MKV",
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
    pub fn label(self) -> &'static str {
        match self {
            Self::H264 => "H.264",
            Self::H265 => "H.265",
            Self::ProRes => "ProRes",
            Self::Av1 => "AV1",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum QualityMode {
    Crf,
    Bitrate,
}

impl QualityMode {
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
    pub start_frame: u32,
    pub frame_count: u32,
    pub fps: f32,
    pub max_samples: u32,
}

impl Default for SequenceSettings {
    fn default() -> Self {
        Self {
            format: SequenceFormat::Png,
            start_frame: 1,
            frame_count: 120,
            fps: 30.0,
            max_samples: 512,
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
        EncodeRequest {
            output_path: PathBuf::from(&self.output_path),
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
                        ui.selectable_value(
                            &mut state.sequence.format,
                            SequenceFormat::Png,
                            SequenceFormat::Png.label(),
                        );
                    });
                ui.end_row();
            });
    });
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
                        for value in [
                            VideoContainer::Mp4,
                            VideoContainer::Mov,
                            VideoContainer::Mkv,
                        ] {
                            ui.selectable_value(&mut state.video.container, value, value.label());
                        }
                    });
                ui.end_row();

                ui.label("Codec:");
                egui::ComboBox::from_id_salt("imageseq_video_codec")
                    .selected_text(state.video.codec.label())
                    .show_ui(ui, |ui| {
                        for value in [
                            VideoCodec::H264,
                            VideoCodec::H265,
                            VideoCodec::ProRes,
                            VideoCodec::Av1,
                        ] {
                            ui.selectable_value(&mut state.video.codec, value, value.label());
                        }
                    });
                ui.end_row();

                ui.label("Quality:");
                egui::ComboBox::from_id_salt("imageseq_video_quality")
                    .selected_text(state.video.quality_mode.label())
                    .show_ui(ui, |ui| {
                        for value in [QualityMode::Crf, QualityMode::Bitrate] {
                            ui.selectable_value(
                                &mut state.video.quality_mode,
                                value,
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
        ui.small(
            "Video encoding is configured here and implemented by the host via an FFmpeg sink.",
        );
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

pub fn save_rgba_png(
    path: impl AsRef<Path>,
    width: u32,
    height: u32,
    pixels: &[u8],
) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    let path = path.as_ref();
    if let Some(parent) = path.parent() {
        if !parent.as_os_str().is_empty() {
            std::fs::create_dir_all(parent)?;
        }
    }

    let img = image::RgbaImage::from_raw(width, height, pixels.to_vec())
        .ok_or("invalid RGBA buffer dimensions")?;
    img.save(path)?;
    Ok(())
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
