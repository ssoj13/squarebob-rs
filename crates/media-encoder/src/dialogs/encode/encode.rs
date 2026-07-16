//! Video encoding module
//!
//! Handles encoding sequences to video files using FFmpeg encoders.
//! Uses explicit CPU codecs with transactional ISO-BMFF output.
//! Entry points are called from encoding dialogs and tests; the code pulls frames
//! from `entities::Comp` (via `Project`) and streams them through FFmpeg. Data flow:
//! timeline/project -> `encode_comp` -> frame retrieval (Comp.get_frame) -> pixel
//! format conversion -> muxed video on disk.

#![allow(clippy::items_after_test_module)] // SwsContext etc. placed after tests for readability

use log::info;
use serde::{Deserialize, Serialize};
use std::fs::File;
use std::io::BufWriter;
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::Sender;

#[cfg(feature = "video")]
use crate::frame::CropAlign;
use crate::frame::{FrameConversion, PixelBuffer, PixelFormat, TonemapMode};
use crate::source::Comp;

/// Export mode - video or image sequence
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default, Serialize, Deserialize)]
pub enum ExportMode {
    #[default]
    Video,
    Sequence,
}

/// Encode dialog settings (persistent via AppSettings)
/// Contains all codec settings + dialog state
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct EncodeDialogSettings {
    // Dialog state
    pub output_path: PathBuf,
    pub container: Container,
    pub fps: f32,
    #[serde(default)]
    pub frame_start: i32,
    #[serde(default = "default_frame_end")]
    pub frame_end: i32,
    pub selected_codec: VideoCodec,

    // HDR → LDR conversion settings
    #[serde(default)]
    pub tonemap_mode: TonemapMode,

    // Per-codec settings (all preserved when switching codecs)
    #[serde(default)]
    pub codec_settings: CodecSettings,

    // Export mode (Video or Sequence)
    #[serde(default)]
    pub export_mode: ExportMode,

    // Image sequence settings
    #[serde(default)]
    pub sequence_settings: SequenceSettings,
}

impl Default for EncodeDialogSettings {
    fn default() -> Self {
        Self {
            output_path: PathBuf::from("output.mp4"),
            container: Container::MP4,
            fps: 24.0,
            frame_start: 0,
            frame_end: default_frame_end(),
            selected_codec: VideoCodec::H264,
            tonemap_mode: TonemapMode::default(),
            codec_settings: CodecSettings::default(),
            export_mode: ExportMode::Video,
            sequence_settings: SequenceSettings::default(),
        }
    }
}

fn default_frame_end() -> i32 {
    119
}

/// Encoder input settings (transport DTO for encode_sequence)
///
/// This is a simple flat structure containing settings for ONE selected codec.
/// The UI uses EncodeDialogSettings which stores settings for ALL codecs (H.264/H.265/ProRes/AV1).
/// When starting encoding, build_encoder_settings() converts EncodeDialogSettings → EncoderSettings
/// by extracting only the settings for the currently selected codec.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct EncoderSettings {
    pub output_path: PathBuf,
    pub container: Container,
    pub codec: VideoCodec,
    pub quality_mode: QualityMode,
    pub quality_value: u32, // CRF 18-28 or bitrate in kbps
    pub fps: f32,           // Output framerate (frames per second)

    // Per-codec optional settings
    #[serde(default)]
    pub preset: Option<String>, // H.264/H.265 CPU preset (e.g. "medium")
    #[serde(default)]
    pub profile: Option<String>, // H.264/H.265 profile (e.g. "high", "main", "main10")
    #[serde(default)]
    pub prores_profile: Option<ProResProfile>, // ProRes profile

    // HDR → LDR conversion settings
    #[serde(default)]
    pub tonemap_mode: TonemapMode, // Tonemapping mode for HDR sources (when encoding 8-bit)
}

impl Default for EncoderSettings {
    fn default() -> Self {
        Self {
            output_path: PathBuf::from("output.mp4"),
            container: Container::MP4,
            codec: VideoCodec::H264,
            quality_mode: QualityMode::CRF,
            quality_value: 23, // Default CRF for H.264
            fps: 24.0,         // Default framerate
            preset: Some("medium".to_string()),
            profile: Some("high".to_string()), // H.264: "high", H.265: "main" or "main10"
            prores_profile: Some(ProResProfile::Standard),
            tonemap_mode: TonemapMode::default(), // ACES by default
        }
    }
}

/// H.264 specific settings
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct H264Settings {
    pub quality_mode: QualityMode,
    pub quality_value: u32, // CRF 0-51 or bitrate kbps
    pub preset: String,     // OpenH264 complexity preset
    pub profile: String,    // baseline/main/high
}

impl Default for H264Settings {
    fn default() -> Self {
        Self {
            quality_mode: QualityMode::CRF,
            quality_value: 23,
            preset: "medium".to_string(),
            profile: "high".to_string(),
        }
    }
}

/// H.265/HEVC specific settings
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct H265Settings {
    pub quality_mode: QualityMode,
    pub quality_value: u32, // CRF 0-51 or bitrate kbps
    pub preset: String,     // Kvazaar speed/quality preset
    #[serde(default)]
    pub profile: String, // "main" (8-bit) or "main10" (10-bit)
}

impl Default for H265Settings {
    fn default() -> Self {
        Self {
            quality_mode: QualityMode::CRF,
            quality_value: 28, // H.265 default is higher than H.264
            preset: "medium".to_string(),
            profile: "main".to_string(), // 8-bit by default
        }
    }
}

/// Shared mutable access to H.264/H.265 settings fields for the unified UI renderer.
pub trait H26xSettingsMut {
    fn quality_mode_mut(&mut self) -> &mut QualityMode;
    fn quality_value_mut(&mut self) -> &mut u32;
    fn preset_mut(&mut self) -> &mut String;
    fn profile_mut(&mut self) -> &mut String;
    fn quality_mode(&self) -> QualityMode;
    fn preset(&self) -> &str;
    fn profile(&self) -> &str;
}

impl H26xSettingsMut for H264Settings {
    fn quality_mode_mut(&mut self) -> &mut QualityMode {
        &mut self.quality_mode
    }
    fn quality_value_mut(&mut self) -> &mut u32 {
        &mut self.quality_value
    }
    fn preset_mut(&mut self) -> &mut String {
        &mut self.preset
    }
    fn profile_mut(&mut self) -> &mut String {
        &mut self.profile
    }
    fn quality_mode(&self) -> QualityMode {
        self.quality_mode
    }
    fn preset(&self) -> &str {
        &self.preset
    }
    fn profile(&self) -> &str {
        &self.profile
    }
}

impl H26xSettingsMut for H265Settings {
    fn quality_mode_mut(&mut self) -> &mut QualityMode {
        &mut self.quality_mode
    }
    fn quality_value_mut(&mut self) -> &mut u32 {
        &mut self.quality_value
    }
    fn preset_mut(&mut self) -> &mut String {
        &mut self.preset
    }
    fn profile_mut(&mut self) -> &mut String {
        &mut self.profile
    }
    fn quality_mode(&self) -> QualityMode {
        self.quality_mode
    }
    fn preset(&self) -> &str {
        &self.preset
    }
    fn profile(&self) -> &str {
        &self.profile
    }
}

/// ProRes profile variants
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[allow(clippy::upper_case_acronyms)]
pub enum ProResProfile {
    Proxy,              // 0
    LT,                 // 1
    Standard,           // 2 (422)
    HQ,                 // 3
    FourFourFourFour,   // 4 (4444)
    FourFourFourFourXQ, // 5 (4444XQ)
}

impl ProResProfile {
    pub fn all() -> &'static [ProResProfile] {
        &[
            ProResProfile::Proxy,
            ProResProfile::LT,
            ProResProfile::Standard,
            ProResProfile::HQ,
            ProResProfile::FourFourFourFour,
            ProResProfile::FourFourFourFourXQ,
        ]
    }

    pub fn to_ffmpeg_value(self) -> &'static str {
        match self {
            ProResProfile::Proxy => "0",
            ProResProfile::LT => "1",
            ProResProfile::Standard => "2",
            ProResProfile::HQ => "3",
            ProResProfile::FourFourFourFour => "4",
            ProResProfile::FourFourFourFourXQ => "5",
        }
    }
}

impl std::fmt::Display for ProResProfile {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ProResProfile::Proxy => write!(f, "Proxy"),
            ProResProfile::LT => write!(f, "LT"),
            ProResProfile::Standard => write!(f, "422 (Standard)"),
            ProResProfile::HQ => write!(f, "422 HQ"),
            ProResProfile::FourFourFourFour => write!(f, "4444"),
            ProResProfile::FourFourFourFourXQ => write!(f, "4444 XQ"),
        }
    }
}

/// ProRes specific settings
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ProResSettings {
    pub profile: ProResProfile,
}

impl Default for ProResSettings {
    fn default() -> Self {
        Self {
            profile: ProResProfile::Standard,
        }
    }
}

/// AV1 specific settings
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct AV1Settings {
    pub quality_mode: QualityMode,
    pub quality_value: u32, // CRF 0-63 or bitrate kbps
}

impl Default for AV1Settings {
    fn default() -> Self {
        Self {
            quality_mode: QualityMode::CRF,
            quality_value: 30, // AV1 default (roughly equivalent to H.264 CRF 23)
        }
    }
}

/// All codec-specific settings
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct CodecSettings {
    pub h264: H264Settings,
    pub h265: H265Settings,
    pub prores: ProResSettings,
    pub av1: AV1Settings,
}

/// Container format
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[allow(clippy::upper_case_acronyms)]
pub enum Container {
    MP4,
    MOV,
}

impl Container {
    pub fn extension(&self) -> &'static str {
        match self {
            Container::MP4 => "mp4",
            Container::MOV => "mov",
        }
    }
}

impl std::fmt::Display for Container {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Container::MP4 => write!(f, "MP4"),
            Container::MOV => write!(f, "MOV"),
        }
    }
}

/// Video codec
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[allow(clippy::upper_case_acronyms)]
pub enum VideoCodec {
    H264,
    H265,
    ProRes,
    AV1,
}

impl VideoCodec {
    pub fn all() -> &'static [VideoCodec] {
        &[
            VideoCodec::H264,
            VideoCodec::H265,
            VideoCodec::AV1,
            VideoCodec::ProRes,
        ]
    }

    /// Get preferred container for this codec
    pub fn preferred_container(&self) -> Container {
        match self {
            VideoCodec::H264 => Container::MP4,
            VideoCodec::H265 => Container::MP4,
            VideoCodec::AV1 => Container::MP4,
            VideoCodec::ProRes => Container::MOV, // ProRes typically uses MOV
        }
    }

    /// Every codec variant has a compiled CPU implementation in the `video` feature.
    pub const fn is_available(&self) -> bool {
        cfg!(feature = "video")
    }
}

impl std::fmt::Display for VideoCodec {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            VideoCodec::H264 => write!(f, "H.264"),
            VideoCodec::H265 => write!(f, "H.265 (HEVC)"),
            VideoCodec::AV1 => write!(f, "AV1"),
            VideoCodec::ProRes => write!(f, "ProRes"),
        }
    }
}

/// Quality mode for encoding
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[allow(clippy::upper_case_acronyms)]
pub enum QualityMode {
    CRF,     // Constant Rate Factor (quality-based)
    Bitrate, // Target bitrate in kbps
}

impl QualityMode {
    pub fn all() -> &'static [QualityMode] {
        &[QualityMode::CRF, QualityMode::Bitrate]
    }
}

impl std::fmt::Display for QualityMode {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            QualityMode::CRF => write!(f, "CRF (Quality)"),
            QualityMode::Bitrate => write!(f, "Bitrate (kbps)"),
        }
    }
}

// ============================================================================
// IMAGE SEQUENCE EXPORT
// ============================================================================

/// Image sequence format
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum SequenceFormat {
    #[cfg_attr(feature = "exr", default)]
    Exr,
    #[cfg_attr(not(feature = "exr"), default)]
    Png,
    Jpeg,
    Tiff,
    Tga,
}

impl SequenceFormat {
    pub fn all() -> &'static [SequenceFormat] {
        #[cfg(feature = "exr")]
        {
            &[
                SequenceFormat::Exr,
                SequenceFormat::Png,
                SequenceFormat::Jpeg,
                SequenceFormat::Tiff,
                SequenceFormat::Tga,
            ]
        }
        #[cfg(not(feature = "exr"))]
        {
            &[
                SequenceFormat::Png,
                SequenceFormat::Jpeg,
                SequenceFormat::Tiff,
                SequenceFormat::Tga,
            ]
        }
    }

    pub fn extension(&self) -> &'static str {
        match self {
            SequenceFormat::Exr => "exr",
            SequenceFormat::Png => "png",
            SequenceFormat::Jpeg => "jpg",
            SequenceFormat::Tiff => "tiff",
            SequenceFormat::Tga => "tga",
        }
    }

    /// Whether format supports alpha channel
    pub fn supports_alpha(&self) -> bool {
        match self {
            SequenceFormat::Exr => true,
            SequenceFormat::Png => true,
            SequenceFormat::Jpeg => false,
            SequenceFormat::Tiff => true,
            SequenceFormat::Tga => true,
        }
    }

    /// Whether format supports HDR (no tonemapping needed)
    pub fn is_hdr(&self) -> bool {
        matches!(self, SequenceFormat::Exr)
    }
}

impl std::fmt::Display for SequenceFormat {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            SequenceFormat::Exr => write!(f, "EXR"),
            SequenceFormat::Png => write!(f, "PNG"),
            SequenceFormat::Jpeg => write!(f, "JPEG"),
            SequenceFormat::Tiff => write!(f, "TIFF"),
            SequenceFormat::Tga => write!(f, "TGA"),
        }
    }
}

/// Channel mode for export
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, Default)]
pub enum ChannelMode {
    Rgb,
    #[default]
    Rgba,
}

impl ChannelMode {
    pub fn all() -> &'static [ChannelMode] {
        &[ChannelMode::Rgb, ChannelMode::Rgba]
    }
}

impl std::fmt::Display for ChannelMode {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ChannelMode::Rgb => write!(f, "RGB"),
            ChannelMode::Rgba => write!(f, "RGBA"),
        }
    }
}

/// Unified output bit depth for all formats
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, Default)]
pub enum OutputBitDepth {
    #[default]
    U8, // 8-bit unsigned
    U16, // 16-bit unsigned
    F16, // 16-bit float (half)
    F32, // 32-bit float
}

impl OutputBitDepth {
    pub fn all() -> &'static [OutputBitDepth] {
        &[
            OutputBitDepth::U8,
            OutputBitDepth::U16,
            OutputBitDepth::F16,
            OutputBitDepth::F32,
        ]
    }
}

impl std::fmt::Display for OutputBitDepth {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            OutputBitDepth::U8 => write!(f, "8-bit"),
            OutputBitDepth::U16 => write!(f, "16-bit"),
            OutputBitDepth::F16 => write!(f, "Half (F16)"),
            OutputBitDepth::F32 => write!(f, "Float (F32)"),
        }
    }
}

/// Format capabilities - what each format supports
#[derive(Clone, Debug)]
pub struct FormatCapabilities {
    pub supported_depths: &'static [OutputBitDepth],
    pub supports_alpha: bool,
    pub is_hdr: bool, // Can store values > 1.0 without tonemapping
}

impl SequenceFormat {
    /// Get capabilities for this format
    pub fn capabilities(&self) -> FormatCapabilities {
        match self {
            SequenceFormat::Exr => FormatCapabilities {
                supported_depths: &[OutputBitDepth::F16, OutputBitDepth::F32],
                supports_alpha: true,
                is_hdr: true,
            },
            SequenceFormat::Png => FormatCapabilities {
                supported_depths: &[OutputBitDepth::U8, OutputBitDepth::U16],
                supports_alpha: true,
                is_hdr: false,
            },
            SequenceFormat::Jpeg => FormatCapabilities {
                supported_depths: &[OutputBitDepth::U8],
                supports_alpha: false,
                is_hdr: false,
            },
            SequenceFormat::Tiff => FormatCapabilities {
                supported_depths: &[OutputBitDepth::U8, OutputBitDepth::U16],
                supports_alpha: true,
                is_hdr: false,
            },
            SequenceFormat::Tga => FormatCapabilities {
                supported_depths: &[OutputBitDepth::U8],
                supports_alpha: true,
                is_hdr: false,
            },
        }
    }

    /// Check if bit depth is supported by this format
    pub fn supports_depth(&self, depth: OutputBitDepth) -> bool {
        self.capabilities().supported_depths.contains(&depth)
    }

    /// Get default bit depth for this format
    pub fn default_depth(&self) -> OutputBitDepth {
        self.capabilities().supported_depths[0]
    }

    /// Validate and fix settings for this format
    pub fn validate_settings(&self, channels: &mut ChannelMode, depth: &mut OutputBitDepth) {
        let caps = self.capabilities();

        // Fix channels if alpha not supported
        if !caps.supports_alpha && *channels == ChannelMode::Rgba {
            *channels = ChannelMode::Rgb;
        }

        // Fix bit depth if not supported
        if !caps.supported_depths.contains(depth) {
            *depth = caps.supported_depths[0];
        }
    }
}

/// EXR compression mode. Quality knob for DWA lives separately in
/// [`ExrSequenceSettings::dwa_quality`] so the variants stay `Eq + Hash`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, Default)]
pub enum ExrCompression {
    None,
    Rle,
    /// Per-scanline ZIP (smaller blocks, slightly faster random access)
    Zips,
    #[default]
    /// 16-scanline ZIP (the OpenEXR default)
    Zip,
    Piz,
    /// Lossless for f16/u32, lossy for f32 (drops 8 bits of mantissa)
    Pxr24,
    /// Lossy for f16; f32/u32 stored uncompressed
    B44,
    /// B44 with uniform-area optimization
    B44a,
    /// 32-scanline DCT, lossy. Quality controlled by `dwa_quality`
    Dwaa,
    /// 256-scanline DCT, lossy. Faster full-frame decode than DWAA
    Dwab,
    /// 32-scanline HTJ2K (enabled through the `vfx-io/htj2k` feature)
    HtJ2k32,
    /// 256-scanline HTJ2K
    HtJ2k256,
}

impl ExrCompression {
    pub fn all() -> &'static [ExrCompression] {
        &[
            ExrCompression::None,
            ExrCompression::Rle,
            ExrCompression::Zips,
            ExrCompression::Zip,
            ExrCompression::Piz,
            ExrCompression::Pxr24,
            ExrCompression::B44,
            ExrCompression::B44a,
            ExrCompression::Dwaa,
            ExrCompression::Dwab,
            ExrCompression::HtJ2k32,
            ExrCompression::HtJ2k256,
        ]
    }

    /// True if this compression discards data (DWA, B44 family, PXR24 for f32).
    pub fn is_lossy(self) -> bool {
        matches!(
            self,
            ExrCompression::B44
                | ExrCompression::B44a
                | ExrCompression::Dwaa
                | ExrCompression::Dwab
                | ExrCompression::HtJ2k32
                | ExrCompression::HtJ2k256
                | ExrCompression::Pxr24
        )
    }

    /// True if this compression has a configurable quality knob.
    pub fn has_quality_knob(self) -> bool {
        matches!(self, ExrCompression::Dwaa | ExrCompression::Dwab)
    }

    /// OIIO-style compression string ready to drop into
    /// `ImageSpec.attributes["compression"]`. DWA variants embed the quality
    /// level after a colon: `"dwaa:45"`. HTJ2K variants always carry an
    /// explicit `:32` / `:256` suffix to avoid ambiguity. Format matches
    /// `vfx_io::exr::compression_str::format` so vfx-io reads it back to
    /// the right `exr-core` compression enum.
    pub fn to_oiio_string(self, dwa_quality: f32) -> String {
        match self {
            ExrCompression::None => "none".to_string(),
            ExrCompression::Rle => "rle".to_string(),
            ExrCompression::Zips => "zips".to_string(),
            ExrCompression::Zip => "zip".to_string(),
            ExrCompression::Piz => "piz".to_string(),
            ExrCompression::Pxr24 => "pxr24".to_string(),
            ExrCompression::B44 => "b44".to_string(),
            ExrCompression::B44a => "b44a".to_string(),
            ExrCompression::Dwaa => {
                if dwa_quality.fract() == 0.0 && dwa_quality.is_finite() {
                    format!("dwaa:{}", dwa_quality as i64)
                } else {
                    format!("dwaa:{}", dwa_quality)
                }
            }
            ExrCompression::Dwab => {
                if dwa_quality.fract() == 0.0 && dwa_quality.is_finite() {
                    format!("dwab:{}", dwa_quality as i64)
                } else {
                    format!("dwab:{}", dwa_quality)
                }
            }
            ExrCompression::HtJ2k32 => "htj2k:32".to_string(),
            ExrCompression::HtJ2k256 => "htj2k:256".to_string(),
        }
    }
}

impl std::fmt::Display for ExrCompression {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ExrCompression::None => write!(f, "None"),
            ExrCompression::Rle => write!(f, "RLE"),
            ExrCompression::Zips => write!(f, "ZIPS"),
            ExrCompression::Zip => write!(f, "ZIP"),
            ExrCompression::Piz => write!(f, "PIZ"),
            ExrCompression::Pxr24 => write!(f, "PXR24"),
            ExrCompression::B44 => write!(f, "B44 (lossy)"),
            ExrCompression::B44a => write!(f, "B44A (lossy)"),
            ExrCompression::Dwaa => write!(f, "DWAA (lossy)"),
            ExrCompression::Dwab => write!(f, "DWAB (lossy)"),
            ExrCompression::HtJ2k32 => write!(f, "HTJ2K-32 (lossy)"),
            ExrCompression::HtJ2k256 => write!(f, "HTJ2K-256 (lossy)"),
        }
    }
}

/// Default DWA quality per OpenEXR convention (lower = smaller, more loss).
pub const DWA_QUALITY_DEFAULT: f32 = 45.0;

/// EXR encode mode — what the writer pulls from for each output frame.
///
/// `DisplayOnly` is the historical playa behavior: take whatever the
/// compositor produced (single RGBA layer) and write it out with the chosen
/// compression. `PassThrough` reads the source EXR via vfx-io and writes
/// it back preserving every layer + per-layer compression / metadata via
/// `vfx_io::exr::write_layers` — only available when the source frames are
/// EXR files coming from a `FileNode`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, Default)]
pub enum ExrEncodeMode {
    /// Write the compositor output as a single RGBA EXR layer (default).
    #[default]
    DisplayOnly,
    /// Read each source EXR via vfx-io and write back preserving all layers,
    /// per-layer compression, channelformats and custom attrs. Falls back to
    /// `DisplayOnly` if the source file isn't an EXR or can't be opened.
    PassThrough,
}

impl ExrEncodeMode {
    pub fn all() -> &'static [ExrEncodeMode] {
        &[ExrEncodeMode::DisplayOnly, ExrEncodeMode::PassThrough]
    }
}

impl std::fmt::Display for ExrEncodeMode {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ExrEncodeMode::DisplayOnly => write!(f, "Display only (single RGBA)"),
            ExrEncodeMode::PassThrough => write!(f, "Pass-through (preserve all layers)"),
        }
    }
}

/// EXR sequence settings. Bit depth comes from the global [`OutputBitDepth`]
/// (filtered to F16/F32 by [`FormatCapabilities`]); per-channel mixed types
/// flow through `mode = PassThrough` which preserves the source layout.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ExrSequenceSettings {
    pub compression: ExrCompression,
    /// Quality 0..=100 for DWAA/DWAB. Ignored for other compressions.
    #[serde(default = "default_dwa_quality")]
    pub dwa_quality: f32,
    /// Encode mode — DisplayOnly (default, current behavior) or PassThrough
    /// (read source EXR via vfx-io, preserve all layers / per-layer compression).
    #[serde(default)]
    pub mode: ExrEncodeMode,
}

fn default_dwa_quality() -> f32 {
    DWA_QUALITY_DEFAULT
}

impl Default for ExrSequenceSettings {
    fn default() -> Self {
        Self {
            compression: ExrCompression::Zip,
            dwa_quality: DWA_QUALITY_DEFAULT,
            mode: ExrEncodeMode::DisplayOnly,
        }
    }
}

/// PNG compression level (0-9)
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct PngSequenceSettings {
    pub compression: u8, // 0 (none) to 9 (max)
}

impl Default for PngSequenceSettings {
    fn default() -> Self {
        Self { compression: 6 }
    }
}

/// JPEG quality settings
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct JpegSequenceSettings {
    pub quality: u8, // 1-100
}

impl Default for JpegSequenceSettings {
    fn default() -> Self {
        Self { quality: 90 }
    }
}

/// TIFF compression
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, Default)]
pub enum TiffCompression {
    None,
    #[default]
    Lzw,
    Zip,
    PackBits,
}

impl TiffCompression {
    pub fn all() -> &'static [TiffCompression] {
        &[
            TiffCompression::None,
            TiffCompression::Lzw,
            TiffCompression::Zip,
            TiffCompression::PackBits,
        ]
    }
}

impl std::fmt::Display for TiffCompression {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            TiffCompression::None => write!(f, "None"),
            TiffCompression::Lzw => write!(f, "LZW"),
            TiffCompression::Zip => write!(f, "ZIP"),
            TiffCompression::PackBits => write!(f, "PackBits"),
        }
    }
}

/// TIFF bit depth
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, Default)]
pub enum TiffBitDepth {
    #[default]
    Eight, // 8-bit
    Sixteen, // 16-bit
}

impl TiffBitDepth {
    pub fn all() -> &'static [TiffBitDepth] {
        &[TiffBitDepth::Eight, TiffBitDepth::Sixteen]
    }
}

impl std::fmt::Display for TiffBitDepth {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            TiffBitDepth::Eight => write!(f, "8-bit"),
            TiffBitDepth::Sixteen => write!(f, "16-bit"),
        }
    }
}

/// TIFF sequence settings
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct TiffSequenceSettings {
    pub bit_depth: TiffBitDepth,
    pub compression: TiffCompression,
}

impl Default for TiffSequenceSettings {
    fn default() -> Self {
        Self {
            bit_depth: TiffBitDepth::Eight,
            compression: TiffCompression::Lzw,
        }
    }
}

/// TGA settings
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
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

/// All sequence format settings
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct SequenceFormatSettings {
    pub exr: ExrSequenceSettings,
    pub png: PngSequenceSettings,
    pub jpeg: JpegSequenceSettings,
    pub tiff: TiffSequenceSettings,
    pub tga: TgaSequenceSettings,
}

/// Sequence export settings
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct SequenceSettings {
    pub format: SequenceFormat,
    pub channels: ChannelMode,
    pub bit_depth: OutputBitDepth,
    pub apply_tonemap: bool,
    pub tonemap_mode: TonemapMode,
    pub format_settings: SequenceFormatSettings,
}

impl Default for SequenceSettings {
    fn default() -> Self {
        let format = SequenceFormat::default();
        let bit_depth = if matches!(format, SequenceFormat::Exr) {
            OutputBitDepth::F16
        } else {
            OutputBitDepth::U8
        };
        Self {
            format,
            channels: ChannelMode::Rgba,
            bit_depth,
            apply_tonemap: false,
            tonemap_mode: TonemapMode::default(),
            format_settings: SequenceFormatSettings::default(),
        }
    }
}

impl SequenceSettings {
    /// Validate settings against format capabilities and fix if needed
    pub fn validate(&mut self) {
        self.format
            .validate_settings(&mut self.channels, &mut self.bit_depth);
    }
}

/// Padding pattern for frame numbering
#[derive(Clone, Debug, PartialEq)]
pub enum PaddingPattern {
    /// Printf-style: %04d -> 4 digits
    Printf { width: usize },
    /// Hash-style: #### -> 4 digits
    Hashes { count: usize },
    /// At-sign: @ -> no padding
    At,
    /// No pattern found
    None,
}

impl PaddingPattern {
    /// Format frame number according to pattern
    pub fn format(&self, frame: i32) -> String {
        match self {
            PaddingPattern::Printf { width } | PaddingPattern::Hashes { count: width } => {
                format!("{:0width$}", frame, width = *width)
            }
            PaddingPattern::At | PaddingPattern::None => {
                format!("{}", frame)
            }
        }
    }
}

/// Parse filename pattern and extract padding info
/// Returns (prefix, pattern, suffix)
/// Example: "render.####.exr" -> ("render.", Hashes{4}, ".exr")
pub fn parse_padding_pattern(filename: &str) -> (String, PaddingPattern, String) {
    // Try printf-style first: %0Nd or %Nd
    if let Some(pos) = filename.find('%') {
        let rest = &filename[pos + 1..];
        let mut chars = rest.chars().peekable();

        // Skip leading zero
        let has_zero = chars.peek() == Some(&'0');
        if has_zero {
            chars.next();
        }

        // Parse width
        let mut width_str = String::new();
        while let Some(&c) = chars.peek() {
            if c.is_ascii_digit() {
                width_str.push(c);
                chars.next();
            } else {
                break;
            }
        }

        // Check for 'd'
        if chars.next() == Some('d') {
            let width = width_str.parse::<usize>().unwrap_or(1);
            let prefix = filename[..pos].to_string();
            let consumed = 1 + if has_zero { 1 } else { 0 } + width_str.len() + 1; // % + 0? + digits + d
            let suffix = filename[pos + consumed..].to_string();
            return (prefix, PaddingPattern::Printf { width }, suffix);
        }
    }

    // Try hash-style: ####
    if let Some(start) = filename.find('#') {
        let mut count = 0;
        for c in filename[start..].chars() {
            if c == '#' {
                count += 1;
            } else {
                break;
            }
        }
        if count > 0 {
            let prefix = filename[..start].to_string();
            let suffix = filename[start + count..].to_string();
            return (prefix, PaddingPattern::Hashes { count }, suffix);
        }
    }

    // Try @-style
    if let Some(pos) = filename.find('@') {
        let prefix = filename[..pos].to_string();
        let suffix = filename[pos + 1..].to_string();
        return (prefix, PaddingPattern::At, suffix);
    }

    // No pattern - insert before extension
    if let Some(dot_pos) = filename.rfind('.') {
        let prefix = format!("{}.", &filename[..dot_pos]);
        let suffix = filename[dot_pos..].to_string();
        (prefix, PaddingPattern::None, suffix)
    } else {
        (
            format!("{}.", filename),
            PaddingPattern::None,
            String::new(),
        )
    }
}

/// Build frame path from pattern
pub fn build_frame_path(
    base_dir: &std::path::Path,
    prefix: &str,
    pattern: &PaddingPattern,
    suffix: &str,
    frame: i32,
) -> PathBuf {
    let filename = format!("{}{}{}", prefix, pattern.format(frame), suffix);
    base_dir.join(filename)
}

/// Update filename extension based on format
pub fn update_extension(path: &std::path::Path, format: SequenceFormat) -> PathBuf {
    let mut new_path = path.to_path_buf();
    new_path.set_extension(format.extension());
    new_path
}

// ============================================================================
// VIDEO ENCODING (existing code)
// ============================================================================

/// Progress updates during encoding
#[derive(Clone, Debug)]
pub struct EncodeProgress {
    pub current_frame: i32,
    pub total_frames: i32,
    pub stage: EncodeStage,
}

/// Encoding stages
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum EncodeStage {
    Validating, // Checking frame sizes
    Opening,    // Creating encoder
    Encoding,   // Encoding frames
    Flushing,   // Flushing encoder
    Complete,   // Successfully finished
    #[allow(dead_code)] // Used in ui_encode.rs pattern matching
    Error(String), // Failed with error
}

/// Encoding errors
#[derive(Debug)]
pub enum EncodeError {
    BackendUnavailable(&'static str),
    EncoderNotFound,
    OutputCreateFailed(String),
    EncodeFrameFailed(String),
    Cancelled,
}

impl std::fmt::Display for EncodeError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            EncodeError::BackendUnavailable(backend) => {
                write!(f, "Encoding backend not available: {}", backend)
            }
            EncodeError::EncoderNotFound => write!(f, "Encoder not found"),
            EncodeError::OutputCreateFailed(msg) => {
                write!(f, "Failed to create output file: {}", msg)
            }
            EncodeError::EncodeFrameFailed(msg) => {
                write!(f, "Frame encoding failed: {}", msg)
            }
            EncodeError::Cancelled => write!(f, "Encoding cancelled by user"),
        }
    }
}

impl std::error::Error for EncodeError {}

/// Convert f32 fps to a stable rational (numerator, denominator).
#[cfg(feature = "video")]
fn fps_to_rational(fps: f32) -> Result<(i32, i32), EncodeError> {
    if !fps.is_finite() || fps <= 0.0 {
        return Err(EncodeError::OutputCreateFailed(
            "frame rate must be finite and positive".to_string(),
        ));
    }
    const NTSC_RATES: &[(f32, i32, i32)] = &[
        (23.976, 24000, 1001),
        (29.97, 30000, 1001),
        (47.952, 48000, 1001),
        (59.94, 60000, 1001),
        (119.88, 120000, 1001),
    ];
    for &(target, numerator, denominator) in NTSC_RATES {
        if (fps - target).abs() < 0.01 {
            return Ok((numerator, denominator));
        }
    }
    let rounded = fps.round();
    if (fps - rounded).abs() < 0.001 {
        return Ok((rounded as i32, 1));
    }

    let numerator = (fps * 1000.0).round() as i32;
    let divisor = gcd_i32(numerator, 1000);
    Ok((numerator / divisor, 1000 / divisor))
}

#[cfg(feature = "video")]
fn gcd_i32(mut a: i32, mut b: i32) -> i32 {
    while b != 0 {
        let remainder = a % b;
        a = b;
        b = remainder;
    }
    a.abs().max(1)
}

/// Encode the composition play range through the unified pure-Rust video backend.
#[cfg(feature = "video")]
pub fn encode_sequence_from_comp(
    comp: &Comp,
    _project: &crate::source::Project,
    settings: &EncoderSettings,
    progress_tx: Sender<EncodeProgress>,
    cancel_flag: Arc<AtomicBool>,
) -> Result<(), EncodeError> {
    let started = std::time::Instant::now();
    let play_range = comp.play_range(true);
    if play_range.1 < play_range.0 {
        return Err(EncodeError::EncodeFrameFailed(format!(
            "invalid play range {}..{}",
            play_range.0, play_range.1
        )));
    }
    let total_frames = play_range
        .1
        .checked_sub(play_range.0)
        .and_then(|count| count.checked_add(1))
        .ok_or_else(|| EncodeError::EncodeFrameFailed("play range overflow".to_string()))?;

    progress_tx
        .send(EncodeProgress {
            current_frame: 0,
            total_frames,
            stage: EncodeStage::Validating,
        })
        .map_err(|_| EncodeError::Cancelled)?;

    let first_frame = comp.get_frame(play_range.0, true).ok_or_else(|| {
        EncodeError::EncodeFrameFailed(format!("first frame {} is unavailable", play_range.0))
    })?;
    let (width, height) = first_frame.resolution();
    let width = u32::try_from(width)
        .map_err(|_| EncodeError::EncodeFrameFailed("frame width exceeds u32".to_string()))?;
    let height = u32::try_from(height)
        .map_err(|_| EncodeError::EncodeFrameFailed("frame height exceeds u32".to_string()))?;

    if cancel_flag.load(Ordering::Relaxed) {
        return Err(EncodeError::Cancelled);
    }
    progress_tx
        .send(EncodeProgress {
            current_frame: 0,
            total_frames,
            stage: EncodeStage::Opening,
        })
        .map_err(|_| EncodeError::Cancelled)?;

    let rate = fps_to_rational(settings.fps)?;
    let mut encoder = super::video::VideoEncoder::open(settings, width, height, rate)?;

    progress_tx
        .send(EncodeProgress {
            current_frame: 0,
            total_frames,
            stage: EncodeStage::Encoding,
        })
        .map_err(|_| EncodeError::Cancelled)?;

    for frame_idx in play_range.0..=play_range.1 {
        if cancel_flag.load(Ordering::Relaxed) {
            return Err(EncodeError::Cancelled);
        }

        let frame = comp.get_frame(frame_idx, true).ok_or_else(|| {
            EncodeError::EncodeFrameFailed(format!("frame {frame_idx} is unavailable"))
        })?;
        let (frame_width, frame_height) = frame.resolution();
        let cropped = if frame_width != width as usize || frame_height != height as usize {
            frame
                .crop_copy(width as usize, height as usize, CropAlign::Center)
                .map_err(|error| {
                    EncodeError::EncodeFrameFailed(format!(
                        "frame {frame_idx} crop {frame_width}x{frame_height} -> {width}x{height} failed: {error}"
                    ))
                })?
        } else {
            frame.clone()
        };

        if cancel_flag.load(Ordering::Relaxed) {
            return Err(EncodeError::Cancelled);
        }
        let source_is_hdr = matches!(
            cropped.pixel_format(),
            PixelFormat::RgbaF16 | PixelFormat::RgbaF32
        );
        let frame_for_encode = if encoder.requires_ldr() && source_is_hdr {
            cropped.tonemap(settings.tonemap_mode).map_err(|error| {
                EncodeError::EncodeFrameFailed(format!(
                    "frame {frame_idx} tonemapping failed: {error}"
                ))
            })?
        } else {
            cropped
        };

        if cancel_flag.load(Ordering::Relaxed) {
            return Err(EncodeError::Cancelled);
        }
        encoder.push(&frame_for_encode)?;

        let current_frame = frame_idx - play_range.0 + 1;
        progress_tx
            .send(EncodeProgress {
                current_frame,
                total_frames,
                stage: EncodeStage::Encoding,
            })
            .map_err(|_| EncodeError::Cancelled)?;
        if current_frame % 10 == 0 || current_frame == total_frames {
            info!("Encoded frame {current_frame}/{total_frames}");
        }
    }

    progress_tx
        .send(EncodeProgress {
            current_frame: total_frames,
            total_frames,
            stage: EncodeStage::Flushing,
        })
        .map_err(|_| EncodeError::Cancelled)?;
    encoder.finish(&cancel_flag)?;

    let _ = progress_tx.send(EncodeProgress {
        current_frame: total_frames,
        total_frames,
        stage: EncodeStage::Complete,
    });
    info!(
        "Encoding complete: {total_frames} frames to {} in {:.2}s",
        settings.output_path.display(),
        started.elapsed().as_secs_f64()
    );
    Ok(())
}

/// High-level encoding entry point: encodes a Comp.
///
/// Comp is the single source of truth for play range and fps.
pub fn encode_comp(
    comp: &Comp,
    project: &crate::source::Project,
    settings: &EncoderSettings,
    progress_tx: Sender<EncodeProgress>,
    cancel_flag: Arc<AtomicBool>,
) -> Result<(), EncodeError> {
    #[cfg(feature = "video")]
    {
        encode_sequence_from_comp(comp, project, settings, progress_tx, cancel_flag)
    }

    #[cfg(not(feature = "video"))]
    {
        let _ = (comp, project, settings, progress_tx, cancel_flag);
        Err(EncodeError::BackendUnavailable("video"))
    }
}

/// Strip alpha channel from RGBA interleaved data.
fn strip_alpha<T: Copy>(rgba: &[T]) -> Vec<T> {
    let mut rgb = Vec::with_capacity(rgba.len() / 4 * 3);
    for chunk in rgba.chunks_exact(4) {
        rgb.push(chunk[0]);
        rgb.push(chunk[1]);
        rgb.push(chunk[2]);
    }
    rgb
}

#[cfg(feature = "exr")]
fn f16_to_f32_buf(data: &[half::f16]) -> Vec<f32> {
    data.iter().map(|v| v.to_f32()).collect()
}

/// Convert any PixelBuffer variant to packed RGBA u8 (clamped, LDR).
fn pixel_buf_to_rgba8(buffer: &PixelBuffer) -> Vec<u8> {
    match buffer {
        PixelBuffer::U8(data) => data.clone(),
        PixelBuffer::F16(data) => data
            .iter()
            .map(|v| (v.to_f32().clamp(0.0, 1.0) * 255.0) as u8)
            .collect(),
        PixelBuffer::F32(data) => data
            .iter()
            .map(|&v| (v.clamp(0.0, 1.0) * 255.0) as u8)
            .collect(),
    }
}

/// Write a frame through `vfx-io` and its pure-Rust `exr-core` backend.
#[cfg(feature = "exr")]
fn write_exr_frame(
    frame: &crate::frame::Frame,
    path: &std::path::Path,
    settings: &ExrSequenceSettings,
    channels: ChannelMode,
    bit_depth: OutputBitDepth,
) -> Result<(), EncodeError> {
    use crate::io::exr_layered::{
        AttrValue, ChannelKind, ChannelSampleType, ChannelSamples, ImageChannel, ImageLayer,
        LayeredImage, Metadata, write_exr_layers,
    };

    let buffer = frame.buffer();
    let (width, height) = frame.resolution();
    let pixel_count = width * height;

    // Convert any pixel buffer to interleaved RGBA f32 once.
    let f32_data: Vec<f32> = match buffer.as_ref() {
        PixelBuffer::F32(data) => data.clone(),
        PixelBuffer::F16(data) => f16_to_f32_buf(data),
        PixelBuffer::U8(data) => data.iter().map(|&v| v as f32 / 255.0).collect(),
    };

    // EXR supports F16 / F32 only; the global filter rejects U8/U16 for EXR.
    let use_half = matches!(
        bit_depth,
        OutputBitDepth::F16 | OutputBitDepth::U8 | OutputBitDepth::U16
    );
    let sample_type = if use_half {
        ChannelSampleType::F16
    } else {
        ChannelSampleType::F32
    };

    let n_out = match channels {
        ChannelMode::Rgba => 4,
        ChannelMode::Rgb => 3,
    };

    // De-interleave into per-channel planar buffers (vfx-io stores F16 as F32
    // in memory; the writer down-converts at write time).
    let mut planar: Vec<Vec<f32>> = (0..n_out)
        .map(|_| Vec::with_capacity(pixel_count))
        .collect();
    for px in 0..pixel_count {
        let base = px * 4;
        for c in 0..n_out {
            planar[c].push(f32_data[base + c]);
        }
    }

    let names: &[&str] = &["R", "G", "B", "A"];
    let kinds: &[ChannelKind] = &[
        ChannelKind::Color,
        ChannelKind::Color,
        ChannelKind::Color,
        ChannelKind::Alpha,
    ];

    let mut exr_channels = Vec::with_capacity(n_out);
    for c in 0..n_out {
        exr_channels.push(ImageChannel {
            name: names[c].to_string(),
            kind: kinds[c],
            sample_type,
            samples: ChannelSamples::F32(std::mem::take(&mut planar[c])),
            sampling: (1, 1),
            // OpenEXR convention: alpha quantized linearly; chroma channels exponentially.
            quantize_linearly: c == 3,
        });
    }

    // Per-layer compression goes into spec.attributes — vfx-io's writer reads it
    // back per layer. Future multi-layer encode reuses
    // this same path with more layers.
    let mut layer = ImageLayer {
        name: String::new(),
        width: width as u32,
        height: height as u32,
        channels: exr_channels,
        ..Default::default()
    };
    layer.spec.attributes.insert(
        "compression".to_string(),
        AttrValue::String(settings.compression.to_oiio_string(settings.dwa_quality)),
    );

    let layered = LayeredImage {
        layers: vec![layer],
        metadata: Metadata::default(),
    };

    // Free convenience function — internally builds an ExrWriter with default
    // options. Per-layer compression comes from layer.spec.attributes (above).
    write_exr_layers(path, &layered).map_err(|e| {
        EncodeError::EncodeFrameFailed(match e {
            crate::io::IoError::Exr(s) => format!("EXR write failed: {s}"),
            crate::io::IoError::Image(s) => format!("EXR write failed (image): {s}"),
            crate::io::IoError::LoadError(s) => format!("EXR write failed (load): {s}"),
            crate::io::IoError::UnsupportedFormat(s) => format!("EXR write failed (format): {s}"),
        })
    })
}

/// Pass-through EXR transcode: read the source EXR for `frame_idx` via vfx-io
/// and write it back preserving every layer + per-layer compression. The
/// source path comes from the first EXR `FileNode` in the project.
///
/// Returns `Ok(true)` if the pass-through succeeded, `Ok(false)` if no EXR
/// source was found (caller should fall back to display-only encode).
#[cfg(feature = "exr")]
fn write_exr_pass_through(
    comp: &Comp,
    frame_idx: i32,
    dest_path: &std::path::Path,
) -> Result<bool, EncodeError> {
    let Some(src) = comp.exr_source_path(frame_idx) else {
        return Ok(false);
    };
    if !src.exists() {
        return Ok(false);
    }

    // Byte-exact pass-through: `vfx-io` preserves every raw `exr-core` chunk
    // payload. No decompress + recompress, so DWAA / DWAB /
    // B44 / HTJ2K survive transcode without quality loss. Custom header
    // attrs (chromaticities, timecode, owner, …) preserved automatically
    // because the source Header is reused verbatim.
    let layered = crate::io::exr_layered::read_exr_layers_passthrough(&src).map_err(|e| {
        EncodeError::EncodeFrameFailed(format!(
            "EXR pass-through read failed for {:?}: {}",
            src,
            match e {
                crate::io::IoError::Exr(s) => s,
                crate::io::IoError::Image(s) => s,
                crate::io::IoError::LoadError(s) => s,
                crate::io::IoError::UnsupportedFormat(s) => s,
            }
        ))
    })?;
    crate::io::exr_layered::write_exr_layers_passthrough(dest_path, &layered).map_err(|e| {
        EncodeError::EncodeFrameFailed(format!(
            "EXR pass-through write failed: {}",
            match e {
                crate::io::IoError::Exr(s) => s,
                crate::io::IoError::Image(s) => s,
                crate::io::IoError::LoadError(s) => s,
                crate::io::IoError::UnsupportedFormat(s) => s,
            }
        ))
    })?;

    Ok(true)
}

/// Write frame to PNG file
fn write_png_frame(
    frame: &crate::frame::Frame,
    path: &std::path::Path,
    settings: &PngSequenceSettings,
    channels: ChannelMode,
    bit_depth: OutputBitDepth,
) -> Result<(), EncodeError> {
    use image::ImageEncoder;
    use image::codecs::png::{CompressionType, FilterType, PngEncoder};

    let buffer = frame.buffer();
    let (width, height) = frame.resolution();

    let file = File::create(path).map_err(|e| {
        EncodeError::OutputCreateFailed(format!("Failed to create PNG file: {}", e))
    })?;
    let writer = BufWriter::new(file);

    let compression = match settings.compression {
        0 => CompressionType::Fast,
        1..=3 => CompressionType::Fast,
        4..=6 => CompressionType::Default,
        _ => CompressionType::Best,
    };

    let encoder = PngEncoder::new_with_quality(writer, compression, FilterType::Adaptive);

    // PNG supports U8 and U16
    match bit_depth {
        OutputBitDepth::U8 => {
            let rgba_data = pixel_buf_to_rgba8(buffer.as_ref());
            match channels {
                ChannelMode::Rgba => {
                    encoder
                        .write_image(
                            &rgba_data,
                            width as u32,
                            height as u32,
                            image::ExtendedColorType::Rgba8,
                        )
                        .map_err(|e| {
                            EncodeError::EncodeFrameFailed(format!("PNG encode failed: {}", e))
                        })?;
                }
                ChannelMode::Rgb => {
                    encoder
                        .write_image(
                            &strip_alpha(&rgba_data),
                            width as u32,
                            height as u32,
                            image::ExtendedColorType::Rgb8,
                        )
                        .map_err(|e| {
                            EncodeError::EncodeFrameFailed(format!("PNG encode failed: {}", e))
                        })?;
                }
            }
        }
        OutputBitDepth::U16 | OutputBitDepth::F16 | OutputBitDepth::F32 => {
            // Convert to U16 for PNG16
            let rgba16_data: Vec<u16> = match buffer.as_ref() {
                PixelBuffer::U8(data) => data.iter().map(|&v| (v as u16) * 257).collect(),
                PixelBuffer::F16(data) => data
                    .iter()
                    .map(|v| (v.to_f32().clamp(0.0, 1.0) * 65535.0) as u16)
                    .collect(),
                PixelBuffer::F32(data) => data
                    .iter()
                    .map(|&v| (v.clamp(0.0, 1.0) * 65535.0) as u16)
                    .collect(),
            };

            match channels {
                ChannelMode::Rgba => {
                    encoder
                        .write_image(
                            bytemuck::cast_slice(&rgba16_data),
                            width as u32,
                            height as u32,
                            image::ExtendedColorType::Rgba16,
                        )
                        .map_err(|e| {
                            EncodeError::EncodeFrameFailed(format!("PNG16 encode failed: {}", e))
                        })?;
                }
                ChannelMode::Rgb => {
                    encoder
                        .write_image(
                            bytemuck::cast_slice(&strip_alpha(&rgba16_data)),
                            width as u32,
                            height as u32,
                            image::ExtendedColorType::Rgb16,
                        )
                        .map_err(|e| {
                            EncodeError::EncodeFrameFailed(format!("PNG16 encode failed: {}", e))
                        })?;
                }
            }
        }
    }

    Ok(())
}

/// Write frame to JPEG file
fn write_jpeg_frame(
    frame: &crate::frame::Frame,
    path: &std::path::Path,
    settings: &JpegSequenceSettings,
) -> Result<(), EncodeError> {
    use image::ImageEncoder;
    use image::codecs::jpeg::JpegEncoder;

    let buffer = frame.buffer();
    let (width, height) = frame.resolution();

    // Get U8 data
    let rgba_data = match buffer.as_ref() {
        PixelBuffer::U8(data) => data.clone(),
        _ => {
            return Err(EncodeError::EncodeFrameFailed(
                "JPEG requires U8 data. Apply tonemapping for HDR sources.".into(),
            ));
        }
    };

    // Convert RGBA to RGB (JPEG doesn't support alpha)
    let rgb_data = strip_alpha(&rgba_data);

    let file = File::create(path).map_err(|e| {
        EncodeError::OutputCreateFailed(format!("Failed to create JPEG file: {}", e))
    })?;
    let writer = BufWriter::new(file);

    let encoder = JpegEncoder::new_with_quality(writer, settings.quality);
    encoder
        .write_image(
            &rgb_data,
            width as u32,
            height as u32,
            image::ExtendedColorType::Rgb8,
        )
        .map_err(|e| EncodeError::EncodeFrameFailed(format!("JPEG encode failed: {}", e)))?;

    Ok(())
}

/// Write frame to TIFF file
fn write_tiff_frame(
    frame: &crate::frame::Frame,
    path: &std::path::Path,
    settings: &TiffSequenceSettings,
    channels: ChannelMode,
    bit_depth: OutputBitDepth,
) -> Result<(), EncodeError> {
    use image::{ImageBuffer, Rgb, Rgba};

    let buffer = frame.buffer();
    let (width, height) = frame.resolution();

    // TIFF supports U8 and U16
    match bit_depth {
        OutputBitDepth::U8 => {
            let rgba_data = pixel_buf_to_rgba8(buffer.as_ref());
            match channels {
                ChannelMode::Rgba => {
                    let img: ImageBuffer<Rgba<u8>, Vec<u8>> = ImageBuffer::from_raw(
                        width as u32,
                        height as u32,
                        rgba_data,
                    )
                    .ok_or_else(|| {
                        EncodeError::EncodeFrameFailed("Failed to create TIFF buffer".into())
                    })?;
                    img.save(path).map_err(|e| {
                        EncodeError::EncodeFrameFailed(format!("TIFF save failed: {}", e))
                    })?;
                }
                ChannelMode::Rgb => {
                    let img: ImageBuffer<Rgb<u8>, Vec<u8>> =
                        ImageBuffer::from_raw(width as u32, height as u32, strip_alpha(&rgba_data))
                            .ok_or_else(|| {
                                EncodeError::EncodeFrameFailed(
                                    "Failed to create TIFF buffer".into(),
                                )
                            })?;
                    img.save(path).map_err(|e| {
                        EncodeError::EncodeFrameFailed(format!("TIFF save failed: {}", e))
                    })?;
                }
            }
        }
        OutputBitDepth::U16 | OutputBitDepth::F16 | OutputBitDepth::F32 => {
            // Convert to U16 for TIFF16
            let rgba16_data: Vec<u16> = match buffer.as_ref() {
                PixelBuffer::U8(data) => data.iter().map(|&v| (v as u16) * 257).collect(),
                PixelBuffer::F16(data) => data
                    .iter()
                    .map(|v| (v.to_f32().clamp(0.0, 1.0) * 65535.0) as u16)
                    .collect(),
                PixelBuffer::F32(data) => data
                    .iter()
                    .map(|&v| (v.clamp(0.0, 1.0) * 65535.0) as u16)
                    .collect(),
            };

            match channels {
                ChannelMode::Rgba => {
                    let img: ImageBuffer<Rgba<u16>, Vec<u16>> =
                        ImageBuffer::from_raw(width as u32, height as u32, rgba16_data)
                            .ok_or_else(|| {
                                EncodeError::EncodeFrameFailed(
                                    "Failed to create TIFF16 buffer".into(),
                                )
                            })?;
                    img.save(path).map_err(|e| {
                        EncodeError::EncodeFrameFailed(format!("TIFF16 save failed: {}", e))
                    })?;
                }
                ChannelMode::Rgb => {
                    let img: ImageBuffer<Rgb<u16>, Vec<u16>> = ImageBuffer::from_raw(
                        width as u32,
                        height as u32,
                        strip_alpha(&rgba16_data),
                    )
                    .ok_or_else(|| {
                        EncodeError::EncodeFrameFailed("Failed to create TIFF16 buffer".into())
                    })?;
                    img.save(path).map_err(|e| {
                        EncodeError::EncodeFrameFailed(format!("TIFF16 save failed: {}", e))
                    })?;
                }
            }
        }
    }

    let _ = settings.compression; // TODO: image crate doesn't expose TIFF compression settings easily
    Ok(())
}

/// Write frame to TGA file
fn write_tga_frame(
    frame: &crate::frame::Frame,
    path: &std::path::Path,
    _settings: &TgaSequenceSettings,
    channels: ChannelMode,
) -> Result<(), EncodeError> {
    use image::{ImageBuffer, Rgb, Rgba};

    let buffer = frame.buffer();
    let (width, height) = frame.resolution();

    let rgba_data = match buffer.as_ref() {
        PixelBuffer::U8(data) => data.clone(),
        _ => {
            return Err(EncodeError::EncodeFrameFailed(
                "TGA requires U8 data. Apply tonemapping for HDR sources.".into(),
            ));
        }
    };

    match channels {
        ChannelMode::Rgba => {
            let img: ImageBuffer<Rgba<u8>, Vec<u8>> =
                ImageBuffer::from_raw(width as u32, height as u32, rgba_data).ok_or_else(|| {
                    EncodeError::EncodeFrameFailed("Failed to create TGA buffer".into())
                })?;
            img.save(path)
                .map_err(|e| EncodeError::EncodeFrameFailed(format!("TGA save failed: {}", e)))?;
        }
        ChannelMode::Rgb => {
            let img: ImageBuffer<Rgb<u8>, Vec<u8>> =
                ImageBuffer::from_raw(width as u32, height as u32, strip_alpha(&rgba_data))
                    .ok_or_else(|| {
                        EncodeError::EncodeFrameFailed("Failed to create TGA buffer".into())
                    })?;
            img.save(path)
                .map_err(|e| EncodeError::EncodeFrameFailed(format!("TGA save failed: {}", e)))?;
        }
    }

    // TODO: RLE compression when image crate supports it
    Ok(())
}

/// Main function to export image sequence
///
/// Exports frames from comp to individual image files.
/// Supports EXR, PNG, JPEG, TIFF, TGA formats.
pub fn encode_image_sequence(
    comp: &Comp,
    _project: &crate::source::Project,
    output_path: &std::path::Path,
    settings: &SequenceSettings,
    progress_tx: Sender<EncodeProgress>,
    cancel_flag: Arc<AtomicBool>,
) -> Result<(), EncodeError> {
    let start_time = std::time::Instant::now();
    info!(
        "========== encode_image_sequence() ENTERED at {:?} ==========",
        start_time
    );

    // Get play range from Comp
    let play_range = comp.play_range(true);
    let total_frames = play_range.1.saturating_sub(play_range.0) + 1;

    info!(
        "Image sequence export: format={:?}, channels={:?}, frames={}",
        settings.format, settings.channels, total_frames
    );

    // Parse output pattern
    let filename = output_path
        .file_name()
        .and_then(|s| s.to_str())
        .unwrap_or("frame.####.exr");
    let base_dir = output_path.parent().unwrap_or(std::path::Path::new("."));

    // Ensure output directory exists
    if !base_dir.exists() {
        std::fs::create_dir_all(base_dir).map_err(|e| {
            EncodeError::OutputCreateFailed(format!("Failed to create output directory: {}", e))
        })?;
    }

    let (prefix, pattern, suffix) = parse_padding_pattern(filename);
    info!(
        "Pattern parsed: prefix='{}', pattern={:?}, suffix='{}'",
        prefix, pattern, suffix
    );

    // Stage 1: Validating
    if progress_tx
        .send(EncodeProgress {
            current_frame: 0,
            total_frames,
            stage: EncodeStage::Validating,
        })
        .is_err()
    {
        return Err(EncodeError::Cancelled);
    }

    // Check for cancellation
    if cancel_flag.load(Ordering::Relaxed) {
        return Err(EncodeError::Cancelled);
    }

    // Stage 2: Encoding loop
    if progress_tx
        .send(EncodeProgress {
            current_frame: 0,
            total_frames,
            stage: EncodeStage::Encoding,
        })
        .is_err()
    {
        return Err(EncodeError::Cancelled);
    }

    for frame_idx in play_range.0..=play_range.1 {
        // Check for cancellation
        if cancel_flag.load(Ordering::Relaxed) {
            return Err(EncodeError::Cancelled);
        }

        let current_frame = frame_idx - play_range.0 + 1;

        // Get frame from comp
        let frame = comp.get_frame(frame_idx, true).ok_or_else(|| {
            EncodeError::EncodeFrameFailed(format!("Frame {} not available", frame_idx))
        })?;

        // Apply tonemapping if needed (HDR -> LDR for non-EXR formats)
        let frame_to_write = if settings.apply_tonemap
            || (!settings.format.is_hdr() && frame.pixel_format() != PixelFormat::Rgba8)
        {
            frame
                .tonemap(settings.tonemap_mode)
                .map_err(|e| EncodeError::EncodeFrameFailed(format!("Tonemapping failed: {}", e)))?
        } else {
            frame.clone()
        };

        // Build output path for this frame
        let frame_path = build_frame_path(base_dir, &prefix, &pattern, &suffix, frame_idx);

        if frame_idx % 10 == 0 {
            info!("Writing frame {} -> {}", frame_idx, frame_path.display());
        }

        // Write frame based on format
        match settings.format {
            SequenceFormat::Exr => {
                #[cfg(not(feature = "exr"))]
                {
                    return Err(EncodeError::BackendUnavailable("OpenEXR"));
                }

                #[cfg(feature = "exr")]
                {
                    let exr_settings = &settings.format_settings.exr;
                    let did_pass_through = match exr_settings.mode {
                        ExrEncodeMode::PassThrough => {
                            write_exr_pass_through(comp, frame_idx, &frame_path)?
                        }
                        ExrEncodeMode::DisplayOnly => false,
                    };
                    if !did_pass_through {
                        // Either DisplayOnly mode or pass-through couldn't find an
                        // EXR source — fall back to compositor-output single-layer write.
                        write_exr_frame(
                            &frame_to_write,
                            &frame_path,
                            exr_settings,
                            settings.channels,
                            settings.bit_depth,
                        )?;
                    }
                }
            }
            SequenceFormat::Png => {
                write_png_frame(
                    &frame_to_write,
                    &frame_path,
                    &settings.format_settings.png,
                    settings.channels,
                    settings.bit_depth,
                )?;
            }
            SequenceFormat::Jpeg => {
                write_jpeg_frame(&frame_to_write, &frame_path, &settings.format_settings.jpeg)?;
            }
            SequenceFormat::Tiff => {
                write_tiff_frame(
                    &frame_to_write,
                    &frame_path,
                    &settings.format_settings.tiff,
                    settings.channels,
                    settings.bit_depth,
                )?;
            }
            SequenceFormat::Tga => {
                write_tga_frame(
                    &frame_to_write,
                    &frame_path,
                    &settings.format_settings.tga,
                    settings.channels,
                )?;
            }
        }

        // Update progress
        if progress_tx
            .send(EncodeProgress {
                current_frame,
                total_frames,
                stage: EncodeStage::Encoding,
            })
            .is_err()
        {
            return Err(EncodeError::Cancelled);
        }
    }

    // Stage 3: Complete
    let _ = progress_tx.send(EncodeProgress {
        current_frame: total_frames,
        total_frames,
        stage: EncodeStage::Complete,
    });

    let elapsed = start_time.elapsed();
    info!(
        "Image sequence export complete: {} frames in {:.2}s ({:.1} fps)",
        total_frames,
        elapsed.as_secs_f64(),
        total_frames as f64 / elapsed.as_secs_f64()
    );

    Ok(())
}
