//! End-to-end delivery oracles for the host adapter and the shared WarpBro exporter.
use media_encoder::{
    Comp, Container, EncodeProgress, EncoderSettings, Frame, FrameSource, Project, VideoCodec,
    encode_comp, hdr::PngEncoding,
};
use std::{
    fmt,
    process::Command,
    sync::{Arc, atomic::AtomicBool, mpsc},
};

struct Fixture(Vec<Frame>);

/// Retain verified deliveries when a review directory is explicitly requested.
fn retain_video(path: &std::path::Path) {
    if let Some(directory) = std::env::var_os("SQUAREBOB_VIDEO_DIR") {
        let directory = std::path::PathBuf::from(directory);
        std::fs::create_dir_all(&directory).unwrap();
        let destination = directory.join(path.file_name().unwrap());
        assert!(
            !destination.exists(),
            "Refusing to overwrite {}",
            destination.display()
        );
        std::fs::copy(path, &destination).unwrap();
        println!("Verified video: {}", destination.display());
    }
}

#[test]
#[ignore = "requires ffmpeg on PATH"]
fn native_prores_4444_alpha_stays_linear() {
    use media_encoder::ProResProfile;
    let dir = tempfile::tempdir().unwrap();
    for profile in [
        ProResProfile::FourFourFourFour,
        ProResProfile::FourFourFourFourXQ,
    ] {
        let frames = [128u8, 64, 255]
            .into_iter()
            .map(|alpha| {
                Frame::rgba8(
                    64,
                    64,
                    (0..64 * 64).flat_map(|_| [30, 128, 230, alpha]).collect(),
                )
                .unwrap()
            })
            .collect();
        let comp: Comp = Arc::new(Fixture(frames));
        let output = dir.path().join(format!("alpha-{profile:?}.mov"));
        let settings = EncoderSettings {
            output_path: output.clone(),
            codec: VideoCodec::ProRes,
            container: Container::MOV,
            prores_profile: Some(profile),
            ..Default::default()
        };
        let (tx, _rx) = mpsc::channel();
        encode_comp(
            &comp,
            &Project,
            &settings,
            tx,
            Arc::new(AtomicBool::new(false)),
        )
        .unwrap();
        let decoded = Command::new("ffmpeg")
            .args(["-v", "error", "-i"])
            .arg(&output)
            .args(["-f", "rawvideo", "-pix_fmt", "rgba64le", "pipe:1"])
            .output()
            .unwrap();
        assert!(
            decoded.status.success(),
            "{profile:?}: {}",
            String::from_utf8_lossy(&decoded.stderr)
        );
        assert_eq!(decoded.stdout.len(), 3 * 64 * 64 * 8);
        for (frame, alpha) in [128u8, 64, 255].into_iter().enumerate() {
            let offset = frame * 64 * 64 * 8 + (32 * 64 + 32) * 8 + 6;
            let value = u16::from_le_bytes([decoded.stdout[offset], decoded.stdout[offset + 1]])
                as f32
                / 65535.0;
            assert!(
                (value - f32::from(alpha) / 255.0).abs() < 0.002,
                "{profile:?} frame{frame}: alpha {value} != {alpha}/255"
            );
        }
        retain_video(&output);
    }
}
impl fmt::Display for Fixture {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("HDR display-light fixture")
    }
}
impl FrameSource for Fixture {
    fn play_range(&self, _: bool) -> (i32, i32) {
        (0, self.0.len() as i32 - 1)
    }
    fn get_frame(&self, index: i32, _: bool) -> Result<Frame, media_encoder::EncodeError> {
        self.0.get(index as usize).cloned().ok_or_else(|| {
            media_encoder::EncodeError::EncodeFrameFailed(format!("Fixture frame {index} missing"))
        })
    }
}

#[derive(Debug)]
struct FailedSource;

impl fmt::Display for FailedSource {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("failed render transform")
    }
}

impl FrameSource for FailedSource {
    fn play_range(&self, _: bool) -> (i32, i32) {
        (0, 0)
    }
    fn get_frame(&self, _: i32, _: bool) -> Result<Frame, media_encoder::EncodeError> {
        Err(media_encoder::EncodeError::EncodeFrameFailed(
            "OCIO view is unavailable".into(),
        ))
    }
}

#[test]
fn source_failure_retains_cause_and_completed_output() {
    let directory = tempfile::tempdir().unwrap();
    let output = directory.path().join("previous.mp4");
    std::fs::write(&output, b"completed output").unwrap();
    for encoding in [PngEncoding::Sdr8, PngEncoding::Hdr10] {
        let comp: Comp = Arc::new(FailedSource);
        let settings = EncoderSettings {
            output_path: output.clone(),
            codec: VideoCodec::H265,
            output_encoding: encoding,
            ..Default::default()
        };
        let (tx, _rx) = mpsc::channel();
        let error = encode_comp(
            &comp,
            &Project,
            &settings,
            tx,
            Arc::new(AtomicBool::new(false)),
        )
        .unwrap_err();
        assert!(
            matches!(&error, media_encoder::EncodeError::EncodeFrameFailed(cause) if cause == "OCIO view is unavailable")
        );
        assert_eq!(std::fs::read(&output).unwrap(), b"completed output");
        assert_eq!(std::fs::read_dir(directory.path()).unwrap().count(), 1);
    }
}

#[test]
fn unexpected_extent_fails_without_cropping_or_replacing_output() {
    let directory = tempfile::tempdir().unwrap();
    let output = directory.path().join("previous.mp4");
    std::fs::write(&output, b"completed output").unwrap();
    let comp: Comp = Arc::new(Fixture(vec![
        Frame::rgba8(64, 64, vec![128; 64 * 64 * 4]).unwrap(),
        Frame::rgba8(128, 64, vec![128; 128 * 64 * 4]).unwrap(),
    ]));
    let settings = EncoderSettings {
        output_path: output.clone(),
        ..Default::default()
    };
    let (tx, _rx) = mpsc::channel();
    let error = encode_comp(
        &comp,
        &Project,
        &settings,
        tx,
        Arc::new(AtomicBool::new(false)),
    )
    .unwrap_err();
    assert!(
        matches!(&error, media_encoder::EncodeError::EncodeFrameFailed(cause) if cause.contains("size changed: 128x64, expected 64x64"))
    );
    assert_eq!(std::fs::read(&output).unwrap(), b"completed output");
    assert_eq!(std::fs::read_dir(directory.path()).unwrap().count(), 1);
}

#[test]
#[ignore = "requires ffmpeg and ffprobe on PATH"]
fn native_sdr_delivery_decodes_bt1886_in_all_video_backends() {
    use media_encoder::ProResProfile;
    let dir = tempfile::tempdir().unwrap();
    for (label, codec, container, profile, prores_profile) in [
        ("h264", VideoCodec::H264, Container::MP4, Some("high"), None),
        (
            "hevc-main",
            VideoCodec::H265,
            Container::MP4,
            Some("main"),
            None,
        ),
        (
            "hevc-main10",
            VideoCodec::H265,
            Container::MP4,
            Some("main10"),
            None,
        ),
        ("av1", VideoCodec::AV1, Container::MP4, None, None),
        (
            "prores-422",
            VideoCodec::ProRes,
            Container::MOV,
            None,
            Some(ProResProfile::Standard),
        ),
        (
            "prores-4444",
            VideoCodec::ProRes,
            Container::MOV,
            None,
            Some(ProResProfile::FourFourFourFour),
        ),
    ] {
        let frames = [30u8, 128, 230]
            .into_iter()
            .map(|code| {
                Frame::rgba8(
                    64,
                    64,
                    (0..64 * 64).flat_map(|_| [code, code, code, 255]).collect(),
                )
                .unwrap()
            })
            .collect();
        let comp: Comp = Arc::new(Fixture(frames));
        let output = dir
            .path()
            .join(format!("{label}.{}", container.extension()));
        let settings = EncoderSettings {
            output_path: output.clone(),
            codec,
            container,
            profile: profile.map(str::to_owned),
            prores_profile,
            preset: Some("medium".into()),
            fps: 30000.0 / 1001.0,
            output_encoding: PngEncoding::Sdr8,
            ..Default::default()
        };
        let (tx, _rx) = mpsc::channel();
        encode_comp(
            &comp,
            &Project,
            &settings,
            tx,
            Arc::new(AtomicBool::new(false)),
        )
        .unwrap();
        let probe = Command::new("ffprobe")
            .args([
                "-v",
                "error",
                "-select_streams",
                "v:0",
                "-count_frames",
                "-show_entries",
                "stream=avg_frame_rate,nb_read_frames,color_primaries,color_space",
                "-of",
                "default=noprint_wrappers=1",
            ])
            .arg(&output)
            .output()
            .unwrap();
        assert!(
            probe.status.success(),
            "{label}: {}",
            String::from_utf8_lossy(&probe.stderr)
        );
        let tags = String::from_utf8(probe.stdout).unwrap();
        for expected in [
            "avg_frame_rate=30000/1001",
            "nb_read_frames=3",
            "color_primaries=bt709",
            "color_space=bt709",
        ] {
            assert!(tags.contains(expected), "{label}: {tags}");
        }
        let decoded = Command::new("ffmpeg")
            .args(["-v", "error", "-i"])
            .arg(&output)
            .args(["-f", "rawvideo", "-pix_fmt", "rgb48le", "pipe:1"])
            .output()
            .unwrap();
        assert!(
            decoded.status.success(),
            "{label}: {}",
            String::from_utf8_lossy(&decoded.stderr)
        );
        assert_eq!(decoded.stdout.len(), 3 * 64 * 64 * 6, "{label}");
        // Independently calculated sRGB inverse + BT.1886 gamma 2.4 oracles.
        for (frame, expected) in [0.1636465f32, 0.527925, 0.9070718].into_iter().enumerate() {
            let pixel = frame * 64 * 64 * 6 + (32 * 64 + 32) * 6;
            for channel in 0..3 {
                let offset = pixel + 2 * channel;
                let value = u16::from_le_bytes([decoded.stdout[offset], decoded.stdout[offset + 1]])
                    as f32
                    / 65535.0;
                assert!(
                    (value - expected).abs() < 0.012,
                    "{label} frame{frame}: {value} != {expected}"
                );
            }
        }
        retain_video(&output);
    }
}

#[test]
fn unsupported_hdr_delivery_fails_before_output_publication() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("existing.mp4");
    std::fs::write(&path, b"completed previous output").unwrap();
    let comp: Comp = Arc::new(Fixture(Vec::new()));
    let (tx, _rx) = mpsc::channel::<EncodeProgress>();
    let settings = EncoderSettings {
        output_path: path.clone(),
        output_encoding: PngEncoding::Hdr10,
        ..Default::default()
    };
    assert!(
        encode_comp(
            &comp,
            &Project,
            &settings,
            tx,
            Arc::new(AtomicBool::new(false))
        )
        .is_err()
    );
    assert_eq!(std::fs::read(&path).unwrap(), b"completed previous output");
    assert_eq!(std::fs::read_dir(dir.path()).unwrap().count(), 1);
}

/// Actual codec + container + RGB decode-back, not merely an argument/tag test.
#[test]
#[ignore = "requires ffmpeg with libx265/prores_ks and ffprobe on PATH"]
fn native_hdr_delivery_retains_light_and_fractional_frame_rate() {
    let dir = tempfile::Builder::new()
        .prefix("squarebob-hdr-%-test-")
        .tempdir()
        .unwrap();
    for encoding in [PngEncoding::Hdr10, PngEncoding::Hlg] {
        for (codec, container) in [
            (VideoCodec::H265, Container::MP4),
            (VideoCodec::ProRes, Container::MOV),
        ] {
            let mut frames = Vec::new();
            for relative in [1.0, 4.0, 2.0] {
                let pixels = (0..64 * 64)
                    .flat_map(|_| [relative, relative, relative, 1.0])
                    .collect();
                frames.push(
                    Frame::display_light(
                        64,
                        64,
                        pixels,
                        media_encoder::hdr::DisplayLight::Relative,
                        203.0,
                    )
                    .unwrap(),
                );
            }
            let comp: Comp = Arc::new(Fixture(frames));
            let output = dir
                .path()
                .join(format!("{encoding:?}-{codec:?}.{}", container.extension()));
            let settings = EncoderSettings {
                output_path: output.clone(),
                codec,
                container,
                output_encoding: encoding,
                fps: 30000.0 / 1001.0,
                ..Default::default()
            };
            let (tx, rx) = mpsc::channel();
            encode_comp(
                &comp,
                &Project,
                &settings,
                tx,
                Arc::new(AtomicBool::new(false)),
            )
            .unwrap();
            assert!(
                rx.try_iter()
                    .any(|p| p.stage == media_encoder::EncodeStage::Complete)
            );
            let probe = Command::new("ffprobe").args(["-v", "error", "-select_streams", "v:0", "-count_frames",
                "-show_entries", "stream=avg_frame_rate,nb_read_frames,color_primaries,color_transfer,color_space",
                "-of", "default=noprint_wrappers=1"]).arg(&output).output().unwrap();
            assert!(
                probe.status.success(),
                "{}",
                String::from_utf8_lossy(&probe.stderr)
            );
            let tags = String::from_utf8(probe.stdout).unwrap();
            assert!(tags.contains("avg_frame_rate=30000/1001"), "{tags}");
            assert!(tags.contains("nb_read_frames=3"), "{tags}");
            assert!(tags.contains("color_primaries=bt2020"), "{tags}");
            assert!(tags.contains("color_space=bt2020nc"), "{tags}");
            let transfer = if encoding == PngEncoding::Hdr10 {
                "smpte2084"
            } else {
                "arib-std-b67"
            };
            assert!(
                tags.contains(&format!("color_transfer={transfer}")),
                "{tags}"
            );
            let decoded = Command::new("ffmpeg")
                .args(["-v", "error", "-i"])
                .arg(&output)
                .args(["-f", "rawvideo", "-pix_fmt", "rgb48le", "pipe:1"])
                .output()
                .unwrap();
            assert!(
                decoded.status.success(),
                "{}",
                String::from_utf8_lossy(&decoded.stderr)
            );
            assert_eq!(decoded.stdout.len(), 3 * 64 * 64 * 6);
            for (frame, relative) in [1.0, 4.0, 2.0].into_iter().enumerate() {
                let nits = relative * 203.0;
                let expected = if encoding == PngEncoding::Hdr10 {
                    egui_display::pq(nits)
                } else {
                    egui_display::hlg([nits; 3], 1000.0)[0]
                };
                let pixel = frame * 64 * 64 * 6 + (32 * 64 + 32) * 6;
                for channel in 0..3 {
                    let offset = pixel + channel * 2;
                    let code =
                        u16::from_le_bytes([decoded.stdout[offset], decoded.stdout[offset + 1]])
                            as f32
                            / 65535.0;
                    assert!(
                        (code - expected).abs() < 0.01,
                        "{encoding:?}/{codec:?} frame{frame}: {code} != {expected}"
                    );
                }
            }
            retain_video(&output);
        }
    }
    assert!(
        std::fs::read_dir(dir.path()).unwrap().all(|entry| entry
            .unwrap()
            .file_type()
            .unwrap()
            .is_file()),
        "owned staging directories must be gone after publication"
    );
}
