//! Native CPU video encoding adapter over [`video_io::native::Encoder`].
//!
//! Converts `media-encoder` [`EncoderSettings`] and [`Frame`] buffers into [`video_io::catalog`]
//! and [`video_io::native::Pixels`], delegating colour signalling, swscale conversion, codec
//! draining, ISO-BMFF muxing, and transactional `AtomicOut` publication to `video-io`.

use std::sync::atomic::{AtomicBool, Ordering};

use video_io::catalog::{self, Chroma, Codec, Depth, Rate, VideoSettings};
use video_io::color::{bt1886_code, sdr_video_lut};
use video_io::export::{DisplayLight, PngEncoding};
use video_io::native::{Encoder, Params, Pixels};
use video_io::video::Outcome;

use super::encode::{
    Container, EncodeError, EncoderSettings, ProResProfile, QualityMode, VideoCodec,
};
use crate::frame::{Frame, PixelBuffer};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum PackedInput {
    Rgb24,
    Rgb48,
    Rgba64,
}

#[derive(Debug, PartialEq, Eq)]
enum PackedPixels {
    Rgb24(Vec<u8>),
    Rgb48(Vec<u16>),
    Rgba64(Vec<u16>),
}

impl PackedPixels {
    fn as_pixels(&self) -> Pixels<'_> {
        match self {
            Self::Rgb24(data) => Pixels::Rgb24(data),
            Self::Rgb48(data) => Pixels::Rgb48(data),
            Self::Rgba64(data) => Pixels::Rgba64(data),
        }
    }
}

/// Complete video encode session backed by [`video_io::native::Encoder`].
///
/// Dropping before [`Self::finish`] discards the uncommitted temporary file.
pub(super) struct VideoEncoder {
    encoder: Encoder,
    input: PackedInput,
    width: u32,
    height: u32,
}

impl VideoEncoder {
    pub(super) fn open(
        settings: &EncoderSettings,
        width: u32,
        height: u32,
        rate: (i32, i32),
    ) -> Result<Self, EncodeError> {
        if rate.0 <= 0 || rate.1 <= 0 {
            return Err(EncodeError::OutputCreateFailed(
                "frame rate must be finite and positive".to_string(),
            ));
        }
        let fps_num = u32::try_from(rate.0).map_err(|_| {
            EncodeError::OutputCreateFailed("frame-rate numerator exceeds u32".to_string())
        })?;
        let fps_den = u32::try_from(rate.1).map_err(|_| {
            EncodeError::OutputCreateFailed("frame-rate denominator exceeds u32".to_string())
        })?;
        let video = to_video_settings(settings)?;
        let encoder = Encoder::open(
            &Params {
                video: &video,
                signal: PngEncoding::Sdr8,
                width,
                height,
                fps_num,
                fps_den,
                levels: None,
                audio: None,
            },
            &settings.output_path,
            true,
        )
        .map_err(|e| EncodeError::OutputCreateFailed(e.to_string()))?;
        let input = if encoder.alpha() {
            PackedInput::Rgba64
        } else if video.codec == Codec::ProRes || video.depth == Depth::Ten {
            PackedInput::Rgb48
        } else {
            PackedInput::Rgb24
        };
        Ok(Self {
            encoder,
            input,
            width,
            height,
        })
    }

    pub(super) fn push(&mut self, frame: &Frame) -> Result<(), EncodeError> {
        let (frame_width, frame_height) = frame.resolution();
        if frame_width != self.width as usize || frame_height != self.height as usize {
            return Err(EncodeError::EncodeFrameFailed(format!(
                "frame dimensions changed from {}x{} to {}x{}",
                self.width, self.height, frame_width, frame_height
            )));
        }
        let packed = video_pixels(frame, self.input)?;
        self.encoder
            .push(packed.as_pixels())
            .map_err(|e| EncodeError::EncodeFrameFailed(e.to_string()))
    }

    pub(super) fn finish(self, cancel_flag: &AtomicBool) -> Result<(), EncodeError> {
        if cancel_flag.load(Ordering::Relaxed) {
            return Err(EncodeError::Cancelled);
        }
        match self
            .encoder
            .finish_or_cancel(&|| cancel_flag.load(Ordering::Relaxed))
        {
            Ok(Outcome::Published) => Ok(()),
            Ok(Outcome::Cancelled) => Err(EncodeError::Cancelled),
            Err(e) => Err(EncodeError::OutputCreateFailed(e.to_string())),
        }
    }
}

fn to_video_settings(settings: &EncoderSettings) -> Result<VideoSettings, EncodeError> {
    let codec = match settings.codec {
        VideoCodec::H264 => Codec::H264,
        VideoCodec::H265 => Codec::Hevc,
        VideoCodec::AV1 => Codec::Av1,
        VideoCodec::ProRes => Codec::ProRes,
    };
    let container = match settings.container {
        Container::MP4 => catalog::Container::Mp4,
        Container::MOV => catalog::Container::Mov,
    };
    let depth = match settings.codec {
        VideoCodec::H264 => match settings.profile.as_deref().unwrap_or("high") {
            "baseline" | "main" | "high" => Depth::Eight,
            other => {
                return Err(EncodeError::OutputCreateFailed(format!(
                    "unsupported H.264 profile: {other}"
                )));
            }
        },
        VideoCodec::H265 => match settings.profile.as_deref().unwrap_or("main") {
            "main" => Depth::Eight,
            "main10" => Depth::Ten,
            other => {
                return Err(EncodeError::OutputCreateFailed(format!(
                    "unsupported H.265 profile: {other}"
                )));
            }
        },
        VideoCodec::AV1 => Depth::Eight,
        VideoCodec::ProRes => Depth::Ten,
    };
    let rate = match settings.quality_mode {
        QualityMode::CRF => Rate::Crf(settings.quality_value),
        QualityMode::Bitrate => {
            if settings.quality_value == 0 || settings.quality_value.checked_mul(1000).is_none() {
                return Err(EncodeError::OutputCreateFailed(
                    "bitrate in kbps must be positive and fit in u32 bps".to_string(),
                ));
            }
            Rate::BitrateKbps(settings.quality_value)
        }
    };
    let preset = if codec == Codec::Hevc {
        settings.preset.clone().filter(|p| !p.is_empty())
    } else {
        None
    };
    let prores = match settings.prores_profile.unwrap_or(ProResProfile::Standard) {
        ProResProfile::Proxy => catalog::ProResProfile::Proxy,
        ProResProfile::LT => catalog::ProResProfile::Lt,
        ProResProfile::Standard => catalog::ProResProfile::Standard,
        ProResProfile::HQ => catalog::ProResProfile::Hq,
        ProResProfile::FourFourFourFour => catalog::ProResProfile::P4444,
        ProResProfile::FourFourFourFourXQ => catalog::ProResProfile::P4444Xq,
    };
    Ok(VideoSettings {
        codec,
        container,
        depth,
        chroma: Chroma::Yuv420,
        rate,
        preset,
        prores,
        // Read only for EXV, which this dialog does not offer (H.264 / H.265 / AV1 / ProRes).
        exv: catalog::ExvProfile::default(),
        encoder: None,
    })
}

/// One colour conversion for every native SDR codec: display light -> BT.1886.
fn video_pixels(frame: &Frame, input: PackedInput) -> Result<PackedPixels, EncodeError> {
    let lut = sdr_video_lut();
    let channels = match input {
        PackedInput::Rgb24 | PackedInput::Rgb48 => 3,
        PackedInput::Rgba64 => 4,
    };
    let elem_bytes = match input {
        PackedInput::Rgb24 => 1,
        PackedInput::Rgb48 | PackedInput::Rgba64 => 2,
    };
    let count = frame
        .layout()
        .elements_for(channels, elem_bytes)
        .map_err(|e| EncodeError::EncodeFrameFailed(e.to_string()))?;

    let mut rgb24 = Vec::new();
    let mut words = Vec::new();
    match input {
        PackedInput::Rgb24 => rgb24.reserve_exact(count),
        PackedInput::Rgb48 | PackedInput::Rgba64 => words.reserve_exact(count),
    }

    let mut push = |rgba: [u16; 4]| match input {
        PackedInput::Rgb24 => rgb24.extend(
            rgba[..3]
                .iter()
                .map(|&v| ((u32::from(v) + 128) / 257) as u8),
        ),
        PackedInput::Rgb48 => words.extend_from_slice(&rgba[..3]),
        PackedInput::Rgba64 => words.extend_from_slice(&rgba),
    };

    let buffer = frame.buffer();
    match buffer.as_ref() {
        PixelBuffer::U8(data) => {
            for p in data.chunks_exact(4) {
                push([
                    lut[usize::from(p[0]) * 257],
                    lut[usize::from(p[1]) * 257],
                    lut[usize::from(p[2]) * 257],
                    u16::from(p[3]) * 257,
                ]);
            }
        }
        PixelBuffer::F16(_) | PixelBuffer::F32(_) => {
            let (light, kind, _) = frame.hdr_light().map_err(EncodeError::EncodeFrameFailed)?;
            if kind != DisplayLight::Relative {
                return Err(EncodeError::EncodeFrameFailed(
                    "SDR video requires an SDR display view".into(),
                ));
            }
            for p in light.chunks_exact(4) {
                push([
                    float_to_u16(bt1886_code(p[0])),
                    float_to_u16(bt1886_code(p[1])),
                    float_to_u16(bt1886_code(p[2])),
                    float_to_u16(p[3]),
                ]);
            }
        }
    }

    Ok(match input {
        PackedInput::Rgb24 => PackedPixels::Rgb24(rgb24),
        PackedInput::Rgb48 => PackedPixels::Rgb48(words),
        PackedInput::Rgba64 => PackedPixels::Rgba64(words),
    })
}

fn float_to_u16(value: f32) -> u16 {
    (value.clamp(0.0, 1.0) * 65535.0).round() as u16
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn video_pixels_convert_srgb_to_bt1886_and_keep_alpha_linear() {
        let bytes = Frame::rgba8(1, 1, vec![30, 30, 30, 128]).unwrap();
        let light = video_io::color::display_encoded_to_linear([
            30.0 / 255.0,
            30.0 / 255.0,
            30.0 / 255.0,
            128.0 / 255.0,
        ]);
        let float =
            Frame::display_light(1, 1, light.to_vec(), DisplayLight::Relative, 203.0).unwrap();
        for input in [PackedInput::Rgb24, PackedInput::Rgb48, PackedInput::Rgba64] {
            assert_eq!(
                video_pixels(&bytes, input).unwrap(),
                video_pixels(&float, input).unwrap()
            );
        }
        assert_eq!(
            video_pixels(&bytes, PackedInput::Rgb24).unwrap(),
            PackedPixels::Rgb24(vec![42; 3])
        );
        // Independent sRGB EOTF followed by gamma 2.4: byte 30 becomes 10725/65535.
        assert_eq!(
            video_pixels(&bytes, PackedInput::Rgba64).unwrap(),
            PackedPixels::Rgba64(vec![10725, 10725, 10725, 128 * 257])
        );
    }

    #[test]
    fn video_pixels_reject_unlabelled_scene_and_absolute_hdr_light() {
        let scene = Frame::rgba_f32(1, 1, vec![1.0; 4]).unwrap();
        assert!(video_pixels(&scene, PackedInput::Rgb48).is_err());
        let absolute = Frame::display_light(
            1,
            1,
            vec![1.0; 4],
            DisplayLight::Absolute { peak_nits: 1000.0 },
            203.0,
        )
        .unwrap();
        assert!(video_pixels(&absolute, PackedInput::Rgb48).is_err());
    }

    #[test]
    fn all_video_backends_encode_and_publish_iso_bmff() {
        use std::time::{SystemTime, UNIX_EPOCH};

        let nonce = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("system clock")
            .as_nanos();
        let frame = Frame::rgba8(64, 64, vec![128; 64 * 64 * 4]).expect("test frame");
        let cases = [
            (
                "h264",
                VideoCodec::H264,
                Container::MP4,
                Some("high"),
                Some("medium"),
                None,
            ),
            (
                "hevc-main",
                VideoCodec::H265,
                Container::MP4,
                Some("main"),
                Some("medium"),
                None,
            ),
            (
                "hevc-main10",
                VideoCodec::H265,
                Container::MP4,
                Some("main10"),
                Some("medium"),
                None,
            ),
            ("av1", VideoCodec::AV1, Container::MP4, None, None, None),
            (
                "prores-422",
                VideoCodec::ProRes,
                Container::MOV,
                None,
                None,
                Some(ProResProfile::Standard),
            ),
            (
                "prores-4444",
                VideoCodec::ProRes,
                Container::MOV,
                None,
                None,
                Some(ProResProfile::FourFourFourFour),
            ),
        ];

        for (label, codec, container, profile, preset, prores_profile) in cases {
            let output_path = std::env::temp_dir().join(format!(
                "squarebob-media-encoder-{nonce}-{label}.{}",
                container.extension()
            ));
            let settings = EncoderSettings {
                output_path: output_path.clone(),
                container,
                codec,
                quality_mode: QualityMode::CRF,
                quality_value: match codec {
                    VideoCodec::H264 => 23,
                    VideoCodec::H265 => 28,
                    VideoCodec::AV1 => 30,
                    VideoCodec::ProRes => 0,
                },
                fps: 24.0,
                output_encoding: PngEncoding::Sdr8,
                white_nits: 203.0,
                preset: preset.map(str::to_owned),
                profile: profile.map(str::to_owned),
                prores_profile,
                tonemap_mode: Default::default(),
            };

            let result = (|| {
                let mut encoder = VideoEncoder::open(&settings, 64, 64, (24, 1))?;
                encoder.push(&frame)?;
                encoder.finish(&AtomicBool::new(false))
            })();
            let bytes = std::fs::read(&output_path).unwrap_or_default();
            let _ = std::fs::remove_file(&output_path);

            result.unwrap_or_else(|error| panic!("{label} encode failed: {error}"));
            assert!(bytes.len() > 32, "{label} output is too small");
            assert_eq!(&bytes[4..8], b"ftyp", "{label} ftyp missing");
            assert!(
                bytes.windows(4).any(|window| window == b"moov"),
                "{label} moov missing"
            );
        }
    }
}
