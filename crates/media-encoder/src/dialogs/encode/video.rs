//! Pure-Rust CPU video encoding and ISO-BMFF muxing.
//!
//! One state machine owns conversion, codec draining, sample timing, mux finalisation,
//! and transactional publication. Failed or cancelled encodes never replace a valid output.

use std::borrow::Cow;
use std::fs::{File, OpenOptions};
use std::path::{Path, PathBuf};

use av_codec::{
    annexb_to_length_prefixed, av1c_from_au, avcodec_find_encoder_by_name, hvcc_from_au,
};
use av_codec_core::{
    AV_CODEC_FLAG_QSCALE, AV_PKT_FLAG_KEY, AV_PROFILE_UNKNOWN, AVCodecColorMetadata,
    AVCodecContext, AVPacket, FF_QP2LAMBDA, avcodec_alloc_context3,
};
use av_format::{Codec as MuxCodec, MovWriter};
use av_swscale::{SwsContext, sws_alloc_context, sws_init_context, sws_scale_frame};
use av_util_core::AvError;
use av_util_frame::{AVFrame, av_frame_alloc, av_frame_get_buffer};
use av_util_pixfmt::{AVColorRange, AVColorSpace, AVPixelFormat};
use openh264::OpenH264API;
use openh264::encoder::{
    BitRate, Complexity, Encoder as H264Encoder, EncoderConfig, FrameRate, FrameType,
    Profile as H264Profile, QpRange, RateControlMode, VuiConfig,
};
use openh264::formats::YUVBuffer;

use super::encode::{
    Container, EncodeError, EncoderSettings, ProResProfile, QualityMode, VideoCodec,
};
use crate::frame::{Frame, PixelBuffer};

const EAGAIN: i32 = 11;
const SDR_VIDEO_COLOUR: AVCodecColorMetadata = AVCodecColorMetadata::bt709_limited();
const UNITY_MATRIX: [u8; 36] = [
    0x00, 0x01, 0x00, 0x00, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0x00, 0x01, 0x00, 0x00, 0, 0, 0, 0,
    0, 0, 0, 0, 0, 0, 0, 0, 0x40, 0x00, 0x00, 0x00,
];

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum CodecKind {
    H264,
    Hevc,
    Av1,
    ProRes,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum PackedInput {
    Rgb24,
    Rgb48,
    Rgba64,
}

enum Backend {
    H264 {
        encoder: Box<H264Encoder>,
        sws: Box<SwsContext>,
    },
    AvCodec {
        ctx: Box<AVCodecContext>,
        sws: Box<SwsContext>,
    },
}

/// Complete video encode session.
///
/// Dropping before `finish` removes the private partial file. The destination is
/// replaced only after codec drain and mux finalisation both succeed.
pub(super) struct VideoEncoder {
    kind: CodecKind,
    input: PackedInput,
    width: u32,
    height: u32,
    timescale: u32,
    sample_duration: u32,
    frames_fed: i64,
    writer: Option<MovWriter<File>>,
    backend: Backend,
    header_written: bool,
    temp_path: PathBuf,
    output_path: PathBuf,
    committed: bool,
}

impl VideoEncoder {
    pub(super) fn open(
        settings: &EncoderSettings,
        width: u32,
        height: u32,
        rate: (i32, i32),
    ) -> Result<Self, EncodeError> {
        validate_geometry(settings, width, height)?;
        if rate.0 <= 0 || rate.1 <= 0 {
            return Err(EncodeError::OutputCreateFailed(
                "frame rate must be finite and positive".to_string(),
            ));
        }
        let timescale = u32::try_from(rate.0).map_err(|_| {
            EncodeError::OutputCreateFailed("frame-rate numerator exceeds u32".to_string())
        })?;
        let sample_duration = u32::try_from(rate.1).map_err(|_| {
            EncodeError::OutputCreateFailed("frame-rate denominator exceeds u32".to_string())
        })?;

        let (temp_path, file) = create_partial(&settings.output_path)?;
        let cleanup_path = temp_path.clone();
        let writer = match MovWriter::new(file) {
            Ok(writer) => writer,
            Err(error) => {
                let _ = std::fs::remove_file(&cleanup_path);
                return Err(EncodeError::OutputCreateFailed(format!(
                    "MOV writer open failed: {error}"
                )));
            }
        };

        let result = match settings.codec {
            VideoCodec::H264 => Self::open_h264(
                settings,
                width,
                height,
                timescale,
                sample_duration,
                writer,
                temp_path,
            ),
            VideoCodec::H265 => {
                let (input, pix_fmt) = match settings.profile.as_deref().unwrap_or("main") {
                    "main" => (PackedInput::Rgb24, AVPixelFormat::YUV420P),
                    "main10" => (PackedInput::Rgb48, AVPixelFormat::YUV420P10LE),
                    _ => (PackedInput::Rgb24, AVPixelFormat::YUV420P),
                };
                Self::open_avcodec(
                    settings,
                    CodecKind::Hevc,
                    input,
                    pix_fmt,
                    width,
                    height,
                    timescale,
                    sample_duration,
                    writer,
                    temp_path,
                )
            }
            VideoCodec::AV1 => Self::open_avcodec(
                settings,
                CodecKind::Av1,
                PackedInput::Rgb24,
                AVPixelFormat::YUV420P,
                width,
                height,
                timescale,
                sample_duration,
                writer,
                temp_path,
            ),
            VideoCodec::ProRes => {
                let profile = settings.prores_profile.unwrap_or(ProResProfile::Standard);
                let (input, pix_fmt) = if matches!(
                    profile,
                    ProResProfile::FourFourFourFour | ProResProfile::FourFourFourFourXQ
                ) {
                    (PackedInput::Rgba64, AVPixelFormat::YUVA444P10LE)
                } else {
                    (PackedInput::Rgb48, AVPixelFormat::YUV422P10LE)
                };
                Self::open_avcodec(
                    settings,
                    CodecKind::ProRes,
                    input,
                    pix_fmt,
                    width,
                    height,
                    timescale,
                    sample_duration,
                    writer,
                    temp_path,
                )
            }
        };

        if result.is_err() {
            // Construction failed before a VideoEncoder could own Drop cleanup.
            // Never touch the pre-existing final output.
            let _ = std::fs::remove_file(cleanup_path);
        }
        result
    }

    fn open_h264(
        settings: &EncoderSettings,
        width: u32,
        height: u32,
        timescale: u32,
        sample_duration: u32,
        writer: MovWriter<File>,
        temp_path: PathBuf,
    ) -> Result<Self, EncodeError> {
        let fps = timescale as f32 / sample_duration as f32;
        let profile = match settings.profile.as_deref().unwrap_or("high") {
            "baseline" => H264Profile::Baseline,
            "main" => H264Profile::Main,
            "high" => H264Profile::High,
            other => {
                return Err(EncodeError::OutputCreateFailed(format!(
                    "unsupported H.264 profile: {other}"
                )));
            }
        };
        let complexity = h264_complexity(settings.preset.as_deref())?;
        let mut config = EncoderConfig::new()
            .max_frame_rate(FrameRate::from_hz(fps))
            .profile(profile)
            .complexity(complexity)
            .skip_frames(false)
            .vui(VuiConfig::bt709());

        match settings.quality_mode {
            QualityMode::CRF => {
                let qp = u8::try_from(settings.quality_value)
                    .ok()
                    .filter(|q| *q <= 51)
                    .ok_or_else(|| {
                        EncodeError::OutputCreateFailed(
                            "H.264 quality must be in 0..=51".to_string(),
                        )
                    })?;
                config = config
                    .rate_control_mode(RateControlMode::Off)
                    .qp(QpRange::new(qp, qp));
            }
            QualityMode::Bitrate => {
                let bps = bitrate_bps(settings.quality_value)?;
                config = config
                    .rate_control_mode(RateControlMode::Bitrate)
                    .bitrate(BitRate::from_bps(bps));
            }
        }

        let encoder = H264Encoder::with_api_config(OpenH264API::from_source(), config)
            .map_err(|e| EncodeError::OutputCreateFailed(format!("OpenH264 init failed: {e}")))?;
        let sws = conversion_context(width, height, AVPixelFormat::RGB24, AVPixelFormat::YUV420P)?;

        Ok(Self {
            kind: CodecKind::H264,
            input: PackedInput::Rgb24,
            width,
            height,
            timescale,
            sample_duration,
            frames_fed: 0,
            writer: Some(writer),
            backend: Backend::H264 {
                encoder: Box::new(encoder),
                sws,
            },
            header_written: false,
            temp_path,
            output_path: settings.output_path.clone(),
            committed: false,
        })
    }

    #[allow(clippy::too_many_arguments)]
    fn open_avcodec(
        settings: &EncoderSettings,
        kind: CodecKind,
        input: PackedInput,
        pix_fmt: AVPixelFormat,
        width: u32,
        height: u32,
        timescale: u32,
        sample_duration: u32,
        mut writer: MovWriter<File>,
        temp_path: PathBuf,
    ) -> Result<Self, EncodeError> {
        let name = match kind {
            CodecKind::Hevc => "hevc_kvz",
            CodecKind::Av1 => "av1_rav1e",
            CodecKind::ProRes => "prores_aw",
            CodecKind::H264 => unreachable!("H.264 has a dedicated backend"),
        };
        let entry = avcodec_find_encoder_by_name(name).ok_or(EncodeError::EncoderNotFound)?;
        let mut ctx = avcodec_alloc_context3();
        ctx.width = i32::try_from(width)
            .map_err(|_| EncodeError::OutputCreateFailed("width exceeds i32".to_string()))?;
        ctx.height = i32::try_from(height)
            .map_err(|_| EncodeError::OutputCreateFailed("height exceeds i32".to_string()))?;
        ctx.pix_fmt = pix_fmt;
        ctx.set_color_metadata(SDR_VIDEO_COLOUR);
        ctx.set_cfr_timing(timescale, sample_duration)
            .map_err(|error| {
                EncodeError::OutputCreateFailed(format!("invalid encoder frame rate: {error}"))
            })?;
        ctx.profile = match kind {
            CodecKind::ProRes => {
                prores_profile_index(settings.prores_profile.unwrap_or(ProResProfile::Standard))
            }
            _ => AV_PROFILE_UNKNOWN,
        };

        if !matches!(kind, CodecKind::ProRes) {
            match settings.quality_mode {
                QualityMode::CRF => {
                    let max = if matches!(kind, CodecKind::Av1) {
                        63
                    } else {
                        51
                    };
                    let qp = i32::try_from(settings.quality_value)
                        .ok()
                        .filter(|q| *q <= max)
                        .ok_or_else(|| {
                            EncodeError::OutputCreateFailed(format!(
                                "{} quality must be in 0..={max}",
                                codec_label(kind)
                            ))
                        })?;
                    ctx.flags |= AV_CODEC_FLAG_QSCALE;
                    ctx.global_quality = qp * FF_QP2LAMBDA;
                }
                QualityMode::Bitrate => {
                    ctx.bit_rate = i64::from(bitrate_bps(settings.quality_value)?);
                }
            }
        }

        let instance = (entry.make)();
        let open_result = if matches!(kind, CodecKind::Hevc) {
            match settings.preset.as_deref().filter(|p| !p.is_empty()) {
                Some(preset) => ctx.open_with_opts(instance, &[("preset", preset)]),
                None => ctx.open(instance),
            }
        } else {
            ctx.open(instance)
        };
        open_result.map_err(|e| {
            EncodeError::OutputCreateFailed(format!("{name} encoder open failed: {e}"))
        })?;

        let src_fmt = match input {
            PackedInput::Rgb24 => AVPixelFormat::RGB24,
            PackedInput::Rgb48 => AVPixelFormat::RGB48LE,
            PackedInput::Rgba64 => AVPixelFormat::RGBA64LE,
        };
        let sws = conversion_context(width, height, src_fmt, pix_fmt)?;

        let mut header_written = false;
        if matches!(kind, CodecKind::ProRes) {
            let fourcc = ctx.codec_tag.to_le_bytes();
            writer
                .add_video_prores(fourcc, width, height, timescale, &UNITY_MATRIX)
                .map_err(|e| {
                    EncodeError::OutputCreateFailed(format!("ProRes track creation failed: {e}"))
                })?;
            set_sdr_track_colour(&mut writer)?;
            header_written = true;
        }

        Ok(Self {
            kind,
            input,
            width,
            height,
            timescale,
            sample_duration,
            frames_fed: 0,
            writer: Some(writer),
            backend: Backend::AvCodec {
                ctx: Box::new(ctx),
                sws,
            },
            header_written,
            temp_path,
            output_path: settings.output_path.clone(),
            committed: false,
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

        let (format, pixel_bytes) = match self.input {
            PackedInput::Rgb24 => (AVPixelFormat::RGB24, 3),
            PackedInput::Rgb48 => (AVPixelFormat::RGB48LE, 6),
            PackedInput::Rgba64 => (AVPixelFormat::RGBA64LE, 8),
        };
        let data = video_pixels(frame, self.input)?;
        let mut encoded_frame = self.convert_packed(format, &data, pixel_bytes)?;
        match &self.backend {
            Backend::AvCodec { ctx, .. } => {
                encoded_frame.pts = ctx.cfr_pts(self.frames_fed);
                encoded_frame.duration = ctx.cfr_duration();
            }
            Backend::H264 { .. } => {
                encoded_frame.pts = self
                    .frames_fed
                    .checked_mul(i64::from(self.sample_duration))
                    .ok_or_else(|| {
                        EncodeError::EncodeFrameFailed("frame PTS overflow".to_string())
                    })?;
                encoded_frame.duration = i64::from(self.sample_duration);
            }
        }

        match &mut self.backend {
            Backend::H264 { encoder, .. } => {
                let i420 = compact_i420(&encoded_frame)?;
                let yuv = YUVBuffer::from_vec(i420, self.width as usize, self.height as usize);
                let bitstream = encoder.encode(&yuv).map_err(|e| {
                    EncodeError::EncodeFrameFailed(format!("OpenH264 encode failed: {e}"))
                })?;
                let frame_type = bitstream.frame_type();
                let annexb = bitstream.to_vec();
                self.write_h264(&annexb, frame_type)?;
            }
            Backend::AvCodec { ctx, .. } => {
                ctx.avcodec_send_frame(Some(&encoded_frame)).map_err(|e| {
                    EncodeError::EncodeFrameFailed(format!("encoder send_frame failed: {e}"))
                })?;
                self.drain_avcodec()?;
            }
        }

        self.frames_fed += 1;
        Ok(())
    }

    fn convert_packed(
        &mut self,
        src_fmt: AVPixelFormat,
        bytes: &[u8],
        row_bytes_per_pixel: usize,
    ) -> Result<Box<AVFrame>, EncodeError> {
        let row_bytes = (self.width as usize)
            .checked_mul(row_bytes_per_pixel)
            .ok_or_else(|| {
                EncodeError::EncodeFrameFailed("source row size overflow".to_string())
            })?;
        let expected = row_bytes.checked_mul(self.height as usize).ok_or_else(|| {
            EncodeError::EncodeFrameFailed("source frame size overflow".to_string())
        })?;
        if bytes.len() != expected {
            return Err(EncodeError::EncodeFrameFailed(format!(
                "invalid packed frame size: expected {expected}, got {}",
                bytes.len()
            )));
        }

        let mut src = alloc_frame(src_fmt, self.width, self.height)?;
        let stride = usize::try_from(src.linesize[0])
            .map_err(|_| EncodeError::EncodeFrameFailed("negative source stride".to_string()))?;
        let dst = src.data[0]
            .as_mut()
            .and_then(|b| b.data_mut())
            .ok_or_else(|| {
                EncodeError::EncodeFrameFailed("source frame is not writable".to_string())
            })?;
        for y in 0..self.height as usize {
            let source_offset = y * row_bytes;
            let dest_offset = y * stride;
            dst[dest_offset..dest_offset + row_bytes]
                .copy_from_slice(&bytes[source_offset..source_offset + row_bytes]);
        }
        src.color_range = AVColorRange::AVCOL_RANGE_JPEG;
        src.colorspace = AVColorSpace::AVCOL_SPC_BT709;

        let dst_fmt = match &self.backend {
            Backend::H264 { sws, .. } | Backend::AvCodec { sws, .. } => sws.dst_format,
        };
        let mut dst = alloc_frame(dst_fmt, self.width, self.height)?;
        SDR_VIDEO_COLOUR.apply_to_frame(&mut dst);
        let sws = match &self.backend {
            Backend::H264 { sws, .. } | Backend::AvCodec { sws, .. } => sws,
        };
        sws_scale_frame(sws, &mut dst, &src).map_err(|e| {
            EncodeError::EncodeFrameFailed(format!(
                "pixel conversion {src_fmt:?}->{dst_fmt:?} failed: {e}"
            ))
        })?;
        Ok(dst)
    }

    fn drain_avcodec(&mut self) -> Result<(), EncodeError> {
        loop {
            let mut packet = AVPacket::new();
            let result = match &mut self.backend {
                Backend::AvCodec { ctx, .. } => ctx.avcodec_receive_packet(&mut packet),
                Backend::H264 { .. } => unreachable!("OpenH264 does not use AVPacket"),
            };
            match result {
                Ok(()) => self.write_av_packet(&packet)?,
                Err(AvError::Posix(EAGAIN)) | Err(AvError::Eof) => return Ok(()),
                Err(e) => {
                    return Err(EncodeError::EncodeFrameFailed(format!(
                        "encoder receive_packet failed: {e}"
                    )));
                }
            }
        }
    }

    fn write_av_packet(&mut self, packet: &AVPacket) -> Result<(), EncodeError> {
        let (packet_cts, packet_sync) = match &self.backend {
            Backend::AvCodec { ctx, .. } => packet_timing(packet, ctx)?,
            Backend::H264 { .. } => unreachable!("OpenH264 does not use AVPacket"),
        };
        let (sample, cts_offset, is_sync): (Cow<'_, [u8]>, i32, bool) = match self.kind {
            CodecKind::ProRes => (Cow::Borrowed(packet.data()), packet_cts, packet_sync),
            CodecKind::Hevc => {
                if !self.header_written {
                    let config = hvcc_from_au(packet.data()).map_err(|e| {
                        EncodeError::EncodeFrameFailed(format!("hvcC build failed: {e}"))
                    })?;
                    self.add_compressed_track(MuxCodec::Hevc, &config)?;
                }
                (
                    Cow::Owned(annexb_to_length_prefixed(packet.data())),
                    packet_cts,
                    packet_sync,
                )
            }
            CodecKind::Av1 => {
                if !self.header_written {
                    let config = av1c_from_au(packet.data()).map_err(|e| {
                        EncodeError::EncodeFrameFailed(format!("av1C build failed: {e}"))
                    })?;
                    self.add_compressed_track(MuxCodec::Av1, &config)?;
                }
                (Cow::Borrowed(packet.data()), packet_cts, packet_sync)
            }
            CodecKind::H264 => unreachable!("H.264 has a dedicated packet path"),
        };
        let sample_duration = self.sample_duration;
        self.writer_mut()?
            .write_sample(0, &sample, sample_duration, cts_offset, is_sync)
            .map_err(|e| EncodeError::EncodeFrameFailed(format!("sample mux failed: {e}")))
    }

    fn write_h264(&mut self, annexb: &[u8], frame_type: FrameType) -> Result<(), EncodeError> {
        if !self.header_written {
            let config = avcc_from_annexb(annexb)?;
            self.add_compressed_track(MuxCodec::H264, &config)?;
        }
        let sample = h264_sample_from_annexb(annexb)?;
        let is_sync = matches!(frame_type, FrameType::IDR);
        let sample_duration = self.sample_duration;
        self.writer_mut()?
            .write_sample(0, &sample, sample_duration, 0, is_sync)
            .map_err(|e| EncodeError::EncodeFrameFailed(format!("H.264 sample mux failed: {e}")))
    }

    fn add_compressed_track(&mut self, codec: MuxCodec, config: &[u8]) -> Result<(), EncodeError> {
        let width = self.width;
        let height = self.height;
        let timescale = self.timescale;
        self.writer_mut()?
            .add_video(codec, config, width, height, timescale, &UNITY_MATRIX)
            .map_err(|e| {
                EncodeError::OutputCreateFailed(format!("video track creation failed: {e}"))
            })?;
        set_sdr_track_colour(self.writer_mut()?)?;
        self.header_written = true;
        Ok(())
    }

    fn writer_mut(&mut self) -> Result<&mut MovWriter<File>, EncodeError> {
        self.writer.as_mut().ok_or_else(|| {
            EncodeError::OutputCreateFailed("video writer already finalized".to_string())
        })
    }

    pub(super) fn finish(
        mut self,
        cancel_flag: &std::sync::atomic::AtomicBool,
    ) -> Result<(), EncodeError> {
        use std::sync::atomic::Ordering;

        if cancel_flag.load(Ordering::Relaxed) {
            return Err(EncodeError::Cancelled);
        }
        if self.frames_fed == 0 {
            return Err(EncodeError::OutputCreateFailed(
                "cannot finalize a zero-frame video".to_string(),
            ));
        }

        if let Backend::AvCodec { ctx, .. } = &mut self.backend {
            ctx.avcodec_send_frame(None).map_err(|e| {
                EncodeError::EncodeFrameFailed(format!("encoder drain failed: {e}"))
            })?;
            if cancel_flag.load(Ordering::Relaxed) {
                return Err(EncodeError::Cancelled);
            }
            self.drain_avcodec()?;
        }
        if cancel_flag.load(Ordering::Relaxed) {
            return Err(EncodeError::Cancelled);
        }
        if !self.header_written {
            return Err(EncodeError::OutputCreateFailed(
                "encoder emitted no decodable video header".to_string(),
            ));
        }

        let writer = self.writer.take().ok_or_else(|| {
            EncodeError::OutputCreateFailed("video writer already finalized".to_string())
        })?;
        writer.finish().map_err(|e| {
            EncodeError::OutputCreateFailed(format!("MOV finalization failed: {e}"))
        })?;
        atomic_replace(&self.temp_path, &self.output_path).map_err(|e| {
            EncodeError::OutputCreateFailed(format!(
                "publishing {} failed: {e}",
                self.output_path.display()
            ))
        })?;
        self.committed = true;
        Ok(())
    }
}

impl Drop for VideoEncoder {
    fn drop(&mut self) {
        if !self.committed {
            self.writer.take();
            if let Err(error) = std::fs::remove_file(&self.temp_path)
                && error.kind() != std::io::ErrorKind::NotFound
            {
                log::warn!(
                    "failed to remove partial video {}: {error}",
                    self.temp_path.display()
                );
            }
        }
    }
}

fn validate_geometry(
    settings: &EncoderSettings,
    width: u32,
    height: u32,
) -> Result<(), EncodeError> {
    let codec = settings.codec;
    if width == 0 || height == 0 {
        return Err(EncodeError::OutputCreateFailed(
            "video dimensions must be non-zero".to_string(),
        ));
    }
    if codec == VideoCodec::ProRes && settings.container != Container::MOV {
        return Err(EncodeError::OutputCreateFailed(
            "ProRes requires a MOV container".to_string(),
        ));
    }
    if matches!(codec, VideoCodec::H264 | VideoCodec::H265 | VideoCodec::AV1)
        && (!width.is_multiple_of(2) || !height.is_multiple_of(2))
    {
        return Err(EncodeError::OutputCreateFailed(format!(
            "{codec} 4:2:0 encoding requires even dimensions, got {width}x{height}"
        )));
    }
    let prores_422 = codec == VideoCodec::ProRes
        && !matches!(
            settings.prores_profile.unwrap_or(ProResProfile::Standard),
            ProResProfile::FourFourFourFour | ProResProfile::FourFourFourFourXQ
        );
    if prores_422 && !width.is_multiple_of(2) {
        return Err(EncodeError::OutputCreateFailed(format!(
            "ProRes 4:2:2 encoding requires even width, got {width}"
        )));
    }
    if codec == VideoCodec::H264 && (width > 3840 || height > 2160) {
        return Err(EncodeError::OutputCreateFailed(format!(
            "OpenH264 supports at most 3840x2160, got {width}x{height}"
        )));
    }
    if codec == VideoCodec::H265
        && !matches!(
            settings.profile.as_deref().unwrap_or("main"),
            "main" | "main10"
        )
    {
        return Err(EncodeError::OutputCreateFailed(format!(
            "unsupported H.265 profile: {}",
            settings.profile.as_deref().unwrap_or("main")
        )));
    }
    Ok(())
}

fn conversion_context(
    width: u32,
    height: u32,
    src: AVPixelFormat,
    dst: AVPixelFormat,
) -> Result<Box<SwsContext>, EncodeError> {
    let mut context = sws_alloc_context();
    context
        .set_src(width as usize, height as usize, src)
        .set_dst(width as usize, height as usize, dst)
        .set_colorspace(AVColorSpace::AVCOL_SPC_BT709)
        .set_range(true, false);
    sws_init_context(&mut context).map_err(|e| {
        EncodeError::OutputCreateFailed(format!(
            "pixel conversion {src:?}->{dst:?} is unavailable: {e}"
        ))
    })?;
    Ok(context)
}

fn alloc_frame(
    format: AVPixelFormat,
    width: u32,
    height: u32,
) -> Result<Box<AVFrame>, EncodeError> {
    let mut frame = av_frame_alloc();
    frame.format = format as i32;
    frame.width = i32::try_from(width)
        .map_err(|_| EncodeError::EncodeFrameFailed("width exceeds i32".to_string()))?;
    frame.height = i32::try_from(height)
        .map_err(|_| EncodeError::EncodeFrameFailed("height exceeds i32".to_string()))?;
    av_frame_get_buffer(&mut frame, 0)
        .map_err(|e| EncodeError::EncodeFrameFailed(format!("frame allocation failed: {e}")))?;
    Ok(frame)
}

fn compact_i420(frame: &AVFrame) -> Result<Vec<u8>, EncodeError> {
    let width = usize::try_from(frame.width)
        .map_err(|_| EncodeError::EncodeFrameFailed("negative I420 width".to_string()))?;
    let height = usize::try_from(frame.height)
        .map_err(|_| EncodeError::EncodeFrameFailed("negative I420 height".to_string()))?;
    let y_size = width
        .checked_mul(height)
        .ok_or_else(|| EncodeError::EncodeFrameFailed("I420 size overflow".to_string()))?;
    let chroma_width = width / 2;
    let chroma_height = height / 2;
    let chroma_size = chroma_width
        .checked_mul(chroma_height)
        .ok_or_else(|| EncodeError::EncodeFrameFailed("I420 chroma size overflow".to_string()))?;
    let mut output = Vec::with_capacity(y_size + 2 * chroma_size);

    for (plane, row_width, rows) in [
        (0usize, width, height),
        (1usize, chroma_width, chroma_height),
        (2usize, chroma_width, chroma_height),
    ] {
        let data = frame.data[plane]
            .as_ref()
            .map(|b| b.data())
            .ok_or_else(|| EncodeError::EncodeFrameFailed(format!("I420 plane {plane} missing")))?;
        let stride = usize::try_from(frame.linesize[plane])
            .map_err(|_| EncodeError::EncodeFrameFailed("negative I420 stride".to_string()))?;
        for row in 0..rows {
            let start = row * stride;
            output.extend_from_slice(&data[start..start + row_width]);
        }
    }
    Ok(output)
}

/// One colour conversion for every native SDR codec: display light → BT.1886.
/// The 256-entry sRGB lookup avoids per-pixel powers for viewport RGBA8 capture.
fn video_pixels(frame: &Frame, input: PackedInput) -> Result<Vec<u8>, EncodeError> {
    static SRGB_TO_VIDEO: std::sync::LazyLock<[u16; 256]> = std::sync::LazyLock::new(|| {
        std::array::from_fn(|code| {
            let light =
                crate::hdr::display_encoded_to_linear([code as f32 / 255.0, 0.0, 0.0, 1.0])[0];
            float_to_u16(crate::hdr::bt1886_code(light))
        })
    });
    let pixel_bytes = match input {
        PackedInput::Rgb24 => 3,
        PackedInput::Rgb48 => 6,
        PackedInput::Rgba64 => 8,
    };
    let capacity = frame
        .layout()
        .elements_for(pixel_bytes, 1)
        .map_err(|e| EncodeError::EncodeFrameFailed(e.to_string()))?;
    let mut output = Vec::with_capacity(capacity);
    let mut write = |rgba: [u16; 4]| match input {
        PackedInput::Rgb24 => output.extend(
            rgba[..3]
                .iter()
                .map(|&v| ((u32::from(v) + 128) / 257) as u8),
        ),
        PackedInput::Rgb48 => {
            for value in &rgba[..3] {
                output.extend_from_slice(&value.to_le_bytes());
            }
        }
        PackedInput::Rgba64 => {
            for value in rgba {
                output.extend_from_slice(&value.to_le_bytes());
            }
        }
    };
    let buffer = frame.buffer();
    match buffer.as_ref() {
        PixelBuffer::U8(data) => {
            for p in data.chunks_exact(4) {
                write([
                    SRGB_TO_VIDEO[p[0] as usize],
                    SRGB_TO_VIDEO[p[1] as usize],
                    SRGB_TO_VIDEO[p[2] as usize],
                    u16::from(p[3]) * 257,
                ]);
            }
        }
        PixelBuffer::F16(_) | PixelBuffer::F32(_) => {
            let (light, kind, _) = frame.hdr_light().map_err(EncodeError::EncodeFrameFailed)?;
            if kind != crate::hdr::DisplayLight::Relative {
                return Err(EncodeError::EncodeFrameFailed(
                    "SDR video requires an SDR display view".into(),
                ));
            }
            for p in light.chunks_exact(4) {
                write([
                    float_to_u16(crate::hdr::bt1886_code(p[0])),
                    float_to_u16(crate::hdr::bt1886_code(p[1])),
                    float_to_u16(crate::hdr::bt1886_code(p[2])),
                    float_to_u16(p[3]),
                ]);
            }
        }
    }
    Ok(output)
}

fn set_sdr_track_colour(writer: &mut MovWriter<File>) -> Result<(), EncodeError> {
    let nclx = av_format::video_colr_nclx(SDR_VIDEO_COLOUR).map_err(|e| {
        EncodeError::OutputCreateFailed(format!("video colour metadata failed: {e}"))
    })?;
    writer
        .set_video_colr(&nclx)
        .map_err(|e| EncodeError::OutputCreateFailed(format!("video colour metadata failed: {e}")))
}

fn float_to_u16(value: f32) -> u16 {
    (value.clamp(0.0, 1.0) * 65535.0).round() as u16
}

fn packet_timing(packet: &AVPacket, ctx: &AVCodecContext) -> Result<(i32, bool), EncodeError> {
    let delta = packet.pts.checked_sub(packet.dts).ok_or_else(|| {
        EncodeError::EncodeFrameFailed("encoded packet timestamp overflow".to_string())
    })?;
    if delta < 0 {
        return Err(EncodeError::EncodeFrameFailed(format!(
            "encoded packet pts {} precedes dts {}",
            packet.pts, packet.dts
        )));
    }
    let media_delta = ctx.to_media_ticks(delta);
    let cts_offset = i32::try_from(media_delta).map_err(|_| {
        EncodeError::EncodeFrameFailed(format!("composition offset {media_delta} exceeds i32"))
    })?;
    Ok((cts_offset, packet.flags & AV_PKT_FLAG_KEY != 0))
}

fn h264_complexity(preset: Option<&str>) -> Result<Complexity, EncodeError> {
    match preset.unwrap_or("medium") {
        "ultrafast" | "superfast" | "veryfast" | "faster" | "fast" => Ok(Complexity::Low),
        "medium" => Ok(Complexity::Medium),
        "slow" | "slower" | "veryslow" => Ok(Complexity::High),
        other => Err(EncodeError::OutputCreateFailed(format!(
            "unsupported H.264 preset: {other}"
        ))),
    }
}

fn bitrate_bps(kbps: u32) -> Result<u32, EncodeError> {
    if kbps == 0 {
        return Err(EncodeError::OutputCreateFailed(
            "bitrate must be greater than zero".to_string(),
        ));
    }
    kbps.checked_mul(1000)
        .ok_or_else(|| EncodeError::OutputCreateFailed("bitrate in bits/s exceeds u32".to_string()))
}

fn prores_profile_index(profile: ProResProfile) -> i32 {
    match profile {
        ProResProfile::Proxy => 0,
        ProResProfile::LT => 1,
        ProResProfile::Standard => 2,
        ProResProfile::HQ => 3,
        ProResProfile::FourFourFourFour => 4,
        ProResProfile::FourFourFourFourXQ => 5,
    }
}

fn codec_label(kind: CodecKind) -> &'static str {
    match kind {
        CodecKind::H264 => "H.264",
        CodecKind::Hevc => "H.265",
        CodecKind::Av1 => "AV1",
        CodecKind::ProRes => "ProRes",
    }
}

fn annexb_nalus(data: &[u8]) -> Result<Vec<&[u8]>, EncodeError> {
    fn start_code(data: &[u8], from: usize) -> Option<(usize, usize)> {
        let mut index = from;
        while index + 3 <= data.len() {
            if data[index..].starts_with(&[0, 0, 1]) {
                return Some((index, 3));
            }
            if index + 4 <= data.len() && data[index..].starts_with(&[0, 0, 0, 1]) {
                return Some((index, 4));
            }
            index += 1;
        }
        None
    }

    let (mut start, mut prefix) = start_code(data, 0)
        .ok_or_else(|| EncodeError::EncodeFrameFailed("H.264 packet is not Annex-B".to_string()))?;
    let mut nalus = Vec::new();
    loop {
        let payload_start = start + prefix;
        match start_code(data, payload_start) {
            Some((next, next_prefix)) => {
                if next > payload_start {
                    nalus.push(&data[payload_start..next]);
                }
                start = next;
                prefix = next_prefix;
            }
            None => {
                if payload_start < data.len() {
                    nalus.push(&data[payload_start..]);
                }
                break;
            }
        }
    }
    if nalus.is_empty() || nalus.iter().any(|n| n.is_empty()) {
        return Err(EncodeError::EncodeFrameFailed(
            "H.264 Annex-B packet contains no NAL units".to_string(),
        ));
    }
    Ok(nalus)
}

fn avcc_from_annexb(data: &[u8]) -> Result<Vec<u8>, EncodeError> {
    let nalus = annexb_nalus(data)?;
    let sps = nalus
        .iter()
        .copied()
        .find(|n| n[0] & 0x1f == 7)
        .ok_or_else(|| EncodeError::EncodeFrameFailed("H.264 SPS missing".to_string()))?;
    let pps = nalus
        .iter()
        .copied()
        .find(|n| n[0] & 0x1f == 8)
        .ok_or_else(|| EncodeError::EncodeFrameFailed("H.264 PPS missing".to_string()))?;
    if sps.len() < 4 {
        return Err(EncodeError::EncodeFrameFailed(
            "H.264 SPS is too short".to_string(),
        ));
    }
    let sps_len = u16::try_from(sps.len())
        .map_err(|_| EncodeError::EncodeFrameFailed("H.264 SPS exceeds u16".to_string()))?;
    let pps_len = u16::try_from(pps.len())
        .map_err(|_| EncodeError::EncodeFrameFailed("H.264 PPS exceeds u16".to_string()))?;

    let mut payload = Vec::with_capacity(11 + sps.len() + pps.len());
    payload.extend_from_slice(&[1, sps[1], sps[2], sps[3], 0xff, 0xe1]);
    payload.extend_from_slice(&sps_len.to_be_bytes());
    payload.extend_from_slice(sps);
    payload.push(1);
    payload.extend_from_slice(&pps_len.to_be_bytes());
    payload.extend_from_slice(pps);

    let box_size = u32::try_from(payload.len() + 8)
        .map_err(|_| EncodeError::EncodeFrameFailed("avcC box exceeds u32".to_string()))?;
    let mut full_box = Vec::with_capacity(payload.len() + 8);
    full_box.extend_from_slice(&box_size.to_be_bytes());
    full_box.extend_from_slice(b"avcC");
    full_box.extend_from_slice(&payload);
    Ok(full_box)
}

fn h264_sample_from_annexb(data: &[u8]) -> Result<Vec<u8>, EncodeError> {
    let mut sample = Vec::with_capacity(data.len());
    for nalu in annexb_nalus(data)? {
        let kind = nalu[0] & 0x1f;
        if matches!(kind, 7 | 8) {
            continue;
        }
        let len = u32::try_from(nalu.len()).map_err(|_| {
            EncodeError::EncodeFrameFailed("H.264 NAL unit exceeds u32".to_string())
        })?;
        sample.extend_from_slice(&len.to_be_bytes());
        sample.extend_from_slice(nalu);
    }
    if sample.is_empty() {
        return Err(EncodeError::EncodeFrameFailed(
            "H.264 access unit contains only parameter sets".to_string(),
        ));
    }
    Ok(sample)
}

fn create_partial(output: &Path) -> Result<(PathBuf, File), EncodeError> {
    let parent = output
        .parent()
        .filter(|p| !p.as_os_str().is_empty())
        .unwrap_or(Path::new("."));
    let name = output.file_name().and_then(|n| n.to_str()).ok_or_else(|| {
        EncodeError::OutputCreateFailed("output filename is not valid UTF-8".to_string())
    })?;
    for attempt in 0..1000u32 {
        let suffix = if attempt == 0 {
            format!(".{name}.part")
        } else {
            format!(".{name}.part.{attempt}")
        };
        let path = parent.join(suffix);
        match OpenOptions::new()
            .read(true)
            .write(true)
            .create_new(true)
            .open(&path)
        {
            Ok(file) => return Ok((path, file)),
            Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => continue,
            Err(error) => {
                return Err(EncodeError::OutputCreateFailed(format!(
                    "cannot create partial output {}: {error}",
                    path.display()
                )));
            }
        }
    }
    Err(EncodeError::OutputCreateFailed(
        "could not allocate a unique partial output path".to_string(),
    ))
}

#[cfg(not(windows))]
fn atomic_replace(from: &Path, to: &Path) -> std::io::Result<()> {
    std::fs::rename(from, to)
}

#[cfg(windows)]
fn atomic_replace(from: &Path, to: &Path) -> std::io::Result<()> {
    use std::os::windows::ffi::OsStrExt;
    use windows::Win32::Storage::FileSystem::{
        MOVEFILE_REPLACE_EXISTING, MOVEFILE_WRITE_THROUGH, MoveFileExW,
    };
    use windows::core::PCWSTR;

    let from_wide: Vec<u16> = from.as_os_str().encode_wide().chain(Some(0)).collect();
    let to_wide: Vec<u16> = to.as_os_str().encode_wide().chain(Some(0)).collect();
    // SAFETY: Both pointers reference live, NUL-terminated UTF-16 buffers for the
    // duration of the call. Flags request an atomic same-volume replacement and
    // synchronous metadata flush; no aliases or ownership cross the FFI boundary.
    unsafe {
        MoveFileExW(
            PCWSTR(from_wide.as_ptr()),
            PCWSTR(to_wide.as_ptr()),
            MOVEFILE_REPLACE_EXISTING | MOVEFILE_WRITE_THROUGH,
        )
        .map_err(|error| std::io::Error::other(error.to_string()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn native_sdr_metadata_matches_shared_video_signal() {
        use av_util_pixfmt::{
            av_color_primaries_name, av_color_space_name, av_color_transfer_name,
        };
        let [primaries, transfer, matrix] = crate::hdr::video_tags(crate::hdr::PngEncoding::Sdr8);
        assert_eq!(
            av_color_primaries_name(SDR_VIDEO_COLOUR.primaries),
            Some(primaries)
        );
        assert_eq!(
            av_color_transfer_name(SDR_VIDEO_COLOUR.transfer),
            Some(transfer)
        );
        assert_eq!(av_color_space_name(SDR_VIDEO_COLOUR.matrix), Some(matrix));
    }

    #[test]
    fn video_pixels_convert_srgb_to_bt1886_and_keep_alpha_linear() {
        let bytes = Frame::rgba8(1, 1, vec![30, 30, 30, 128]).unwrap();
        let light = crate::hdr::display_encoded_to_linear([
            30.0 / 255.0,
            30.0 / 255.0,
            30.0 / 255.0,
            128.0 / 255.0,
        ]);
        let float = Frame::display_light(
            1,
            1,
            light.to_vec(),
            crate::hdr::DisplayLight::Relative,
            203.0,
        )
        .unwrap();
        for input in [PackedInput::Rgb24, PackedInput::Rgb48, PackedInput::Rgba64] {
            assert_eq!(
                video_pixels(&bytes, input).unwrap(),
                video_pixels(&float, input).unwrap()
            );
        }
        assert_eq!(video_pixels(&bytes, PackedInput::Rgb24).unwrap(), [42; 3]);
        let words: Vec<u16> = video_pixels(&bytes, PackedInput::Rgba64)
            .unwrap()
            .chunks_exact(2)
            .map(|v| u16::from_le_bytes([v[0], v[1]]))
            .collect();
        // Independent sRGB EOTF followed by gamma 2.4: byte 30 becomes 10725/65535.
        assert_eq!(words, [10725, 10725, 10725, 128 * 257]);
    }

    #[test]
    fn video_pixels_reject_unlabelled_scene_and_absolute_hdr_light() {
        let scene = Frame::rgba_f32(1, 1, vec![1.0; 4]).unwrap();
        assert!(video_pixels(&scene, PackedInput::Rgb48).is_err());
        let absolute = Frame::display_light(
            1,
            1,
            vec![1.0; 4],
            crate::hdr::DisplayLight::Absolute { peak_nits: 1000.0 },
            203.0,
        )
        .unwrap();
        assert!(video_pixels(&absolute, PackedInput::Rgb48).is_err());
    }

    #[test]
    fn h264_annexb_builds_avcc_and_strips_parameter_sets() {
        let au = [
            0, 0, 0, 1, 0x67, 100, 0, 40, 1, 2, 0, 0, 1, 0x68, 3, 4, 0, 0, 1, 0x65, 9, 8, 7,
        ];
        let avcc = avcc_from_annexb(&au).expect("avcC");
        assert_eq!(&avcc[4..8], b"avcC");
        assert_eq!(&avcc[8..12], &[1, 100, 0, 40]);

        let sample = h264_sample_from_annexb(&au).expect("sample");
        assert_eq!(sample, [0, 0, 0, 4, 0x65, 9, 8, 7]);
    }

    #[test]
    fn packet_timing_rejects_negative_composition_offset() {
        let mut ctx = avcodec_alloc_context3();
        ctx.set_cfr_timing(30000, 1001).expect("fractional CFR");
        let mut packet = AVPacket::new();
        packet.pts = 9;
        packet.dts = 10;
        assert!(packet_timing(&packet, &ctx).is_err());
        packet.pts = 12;
        assert_eq!(packet_timing(&packet, &ctx).unwrap().0, 2002);
    }

    #[test]
    fn bitrate_conversion_is_checked() {
        assert!(bitrate_bps(0).is_err());
        assert_eq!(bitrate_bps(8_000).unwrap(), 8_000_000);
        assert!(bitrate_bps(u32::MAX).is_err());
    }

    #[test]
    fn all_video_backends_encode_and_publish_iso_bmff() {
        use std::sync::atomic::AtomicBool;
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
                output_encoding: crate::hdr::PngEncoding::Sdr8,
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
