//! Retained human-review movies rendered by SquareBob's real PBR renderer.
//! The scene is synthetic, and this fixture's camera orbit is not an App export timeline.

use std::fmt;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::{Arc, atomic::AtomicBool, mpsc};

use media_encoder::{
    Comp, Container, EncodeError, EncoderSettings, Frame, FrameSource, Project, VideoCodec,
    encode_comp,
    hdr::{DisplayLight, PngEncoding},
};
use render_shared::{ColorMode, CubeHeightMode, HashTransformEffect, OrbitCamera, Render3DOptions};
use squarebob_core::DirEntry;

const WIDTH: u32 = 256;
const HEIGHT: u32 = 256;
const FRAME_COUNT: usize = 60;
const FPS: f32 = 24.0;
const WHITE_NITS: f32 = 203.0;

struct RenderedMotion(Vec<Frame>);

impl fmt::Display for RenderedMotion {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("SquareBob synthetic treemap / real PBR / demo camera orbit")
    }
}

impl FrameSource for RenderedMotion {
    fn play_range(&self, _: bool) -> (i32, i32) {
        (0, self.0.len() as i32 - 1)
    }

    fn get_frame(&self, index: i32, _: bool) -> Result<Frame, EncodeError> {
        usize::try_from(index)
            .ok()
            .and_then(|index| self.0.get(index))
            .cloned()
            .ok_or_else(|| {
                EncodeError::EncodeFrameFailed(format!("Rendered motion frame {index} missing"))
            })
    }
}

fn synthetic_scene() -> DirEntry {
    let mut root = DirEntry::new_dir("SQUAREBOB SYNTHETIC DEMO".into(), "demo".into());
    let extensions = ["rs", "png", "exr", "mp4", "wav", "md"];
    for group in 0..5 {
        let mut directory = DirEntry::new_dir(
            format!("Demo collection {}", group + 1),
            PathBuf::from(format!("demo/collection-{}", group + 1)),
        );
        for (index, extension) in extensions.iter().enumerate() {
            let size = 1024 * (1 + ((group * 7 + index * 3) % 17)) as u64;
            let name = format!("demo-{}-{}.{}", group + 1, index + 1, extension);
            directory.children.push(DirEntry::new_file(
                name.clone(),
                directory.path.join(name),
                size,
                (*extension).to_owned(),
                None,
            ));
            directory.size += size;
            directory.file_count += 1;
        }
        root.size += directory.size;
        root.file_count += directory.file_count;
        root.dir_count += 1;
        root.children.push(directory);
    }
    root.sort_by_size();
    root
}

fn render_motion() -> RenderedMotion {
    let gpu = Arc::new(render_core::gpu::GpuContext::new().expect("GPU required"));
    let mut renderer = render_3d::Renderer3D::new(gpu);
    let root = synthetic_scene();
    let options = Render3DOptions {
        path_tracing: false,
        env_map_enabled: false,
        env_map_visible: false,
        env_map_path: None,
        hash_effect: HashTransformEffect::None,
        height_mode: CubeHeightMode::Constant,
        color_mode: ColorMode::FileType,
        materialize_mix: 0.0,
        roughness: 0.35,
        metalness: 0.15,
        background_color: [0.02, 0.025, 0.035],
        ..Default::default()
    };
    let treemap_options = treemap::TreeMapOptions::default();
    let mut initial_camera = OrbitCamera::default();
    initial_camera.set_front_view_for_viewport(WIDTH as f32, HEIGHT as f32, 1.0);
    initial_camera.distance *= 1.08;
    initial_camera.orbit(-35.0, 35.0);

    // Bounded at 60 × 256² × RGBA32F (~60 MiB); Frame clones share their pixel buffers.
    let mut frames = Vec::with_capacity(FRAME_COUNT);
    for index in 0..FRAME_COUNT {
        let phase = index as f32 / (FRAME_COUNT - 1) as f32;
        let mut camera = initial_camera.clone();
        camera.orbit(phase * 100.0, (phase * std::f32::consts::PI).sin() * 12.0);
        renderer
            .render_to_view(
                &root,
                WIDTH,
                HEIGHT,
                &camera,
                &options,
                &treemap_options,
                None,
            )
            .unwrap();
        assert!(
            renderer
                .cached_instances()
                .is_some_and(|instances| instances.len() >= 24),
            "Synthetic scene must produce visible cube geometry"
        );
        let bytes = renderer.readback_render_texture(true).unwrap();
        assert_eq!(bytes.len(), WIDTH as usize * HEIGHT as usize * 8);
        let encoded: Vec<_> = bytes
            .chunks_exact(2)
            .map(|channel| {
                half::f16::from_bits(u16::from_le_bytes([channel[0], channel[1]])).to_f32()
            })
            .collect();
        frames.push(
            super::display_canvas_frame(
                WIDTH,
                HEIGHT,
                &encoded,
                DisplayLight::Relative,
                WHITE_NITS,
            )
            .unwrap(),
        );
    }
    let first = frames.first().unwrap().hdr_light().unwrap().0;
    let last = frames.last().unwrap().hdr_light().unwrap().0;
    let changed = first
        .chunks_exact(4)
        .zip(last.chunks_exact(4))
        .filter(|(first, last)| {
            first[..3]
                .iter()
                .zip(&last[..3])
                .any(|(first, last)| (first - last).abs() > 0.01)
        })
        .count();
    assert!(
        changed > 1000,
        "Camera orbit must visibly change the actual render: {changed} pixels"
    );
    RenderedMotion(frames)
}

fn verify_motion_video(path: &Path) -> serde_json::Value {
    let probe = Command::new("ffprobe")
        .args([
            "-v",
            "error",
            "-select_streams",
            "v:0",
            "-count_frames",
            "-show_entries",
            "stream=width,height,nb_read_frames,avg_frame_rate,duration,color_transfer",
            "-of",
            "json",
        ])
        .arg(path)
        .output()
        .unwrap();
    assert!(
        probe.status.success(),
        "{}",
        String::from_utf8_lossy(&probe.stderr)
    );
    let metadata: serde_json::Value = serde_json::from_slice(&probe.stdout).unwrap();
    let stream = &metadata["streams"][0];
    assert_eq!(stream["width"].as_u64().unwrap(), u64::from(WIDTH));
    assert_eq!(stream["height"].as_u64().unwrap(), u64::from(HEIGHT));
    assert_eq!(
        stream["nb_read_frames"].as_str().unwrap(),
        FRAME_COUNT.to_string()
    );
    assert_eq!(stream["avg_frame_rate"].as_str().unwrap(), "24/1");
    let duration: f32 = stream["duration"].as_str().unwrap().parse().unwrap();
    assert!((duration - FRAME_COUNT as f32 / FPS).abs() < 1.0 / FPS);

    let decoded = Command::new("ffmpeg")
        .args(["-v", "error", "-i"])
        .arg(path)
        .args([
            "-map", "0:v:0", "-f", "rawvideo", "-pix_fmt", "rgb24", "pipe:1",
        ])
        .output()
        .unwrap();
    assert!(
        decoded.status.success(),
        "{}",
        String::from_utf8_lossy(&decoded.stderr)
    );
    let frame_bytes = WIDTH as usize * HEIGHT as usize * 3;
    assert_eq!(decoded.stdout.len(), FRAME_COUNT * frame_bytes);
    assert_ne!(
        &decoded.stdout[..frame_bytes],
        &decoded.stdout[(FRAME_COUNT - 1) * frame_bytes..],
        "Decoded movie must contain real motion"
    );
    metadata
}

#[test]
#[ignore = "requires GPU, ffmpeg and ffprobe; retains review movies in C:/Temp/bob"]
fn retained_squarebob_pbr_motion_movies_sdr_pq_hlg() {
    let base = std::env::var_os("SQUAREBOB_VIDEO_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("C:/Temp/bob"));
    std::fs::create_dir_all(&base).unwrap();
    let directory = base.join(format!("squarebob-motion-{}", uuid::Uuid::new_v4()));
    std::fs::create_dir(&directory).unwrap();
    let comp: Comp = Arc::new(render_motion());
    let mut deliveries = Vec::new();
    for (label, encoding) in [
        ("sdr", PngEncoding::Sdr8),
        ("pq-hdr10", PngEncoding::Hdr10),
        ("hlg", PngEncoding::Hlg),
    ] {
        let path = directory.join(format!("squarebob-pbr-demo-orbit-{label}.mp4"));
        assert!(!path.exists(), "Refusing to overwrite {}", path.display());
        let settings = EncoderSettings {
            output_path: path.clone(),
            codec: VideoCodec::H265,
            container: Container::MP4,
            fps: FPS,
            output_encoding: encoding,
            white_nits: WHITE_NITS,
            profile: Some("main10".into()),
            preset: Some("medium".into()),
            quality_value: 18,
            ..Default::default()
        };
        let (sender, _receiver) = mpsc::channel();
        encode_comp(
            &comp,
            &Project,
            &settings,
            sender,
            Arc::new(AtomicBool::new(false)),
        )
        .unwrap();
        let probe = verify_motion_video(&path);
        println!("Verified SquareBob PBR motion movie: {}", path.display());
        deliveries.push(serde_json::json!({ "file": path.file_name().unwrap().to_string_lossy(), "encoding": label, "probe": probe }));
    }
    let manifest = serde_json::json!({
        "scene": "Synthetic SquareBob treemap demo; no private filesystem scan",
        "renderer": "Actual SquareBob Renderer3D PBR",
        "camera": "Test-only OrbitCamera trajectory; not an App export camera timeline",
        "capture": "Existing RGBA16F canvas, typed relative Rec.709 display light",
        "reference_white_nits": WHITE_NITS,
        "frames": FRAME_COUNT,
        "fps": FPS,
        "deliveries": deliveries,
    });
    std::fs::write(
        directory.join("manifest.json"),
        serde_json::to_vec_pretty(&manifest).unwrap(),
    )
    .unwrap();
    println!("Retained review movies: {}", directory.display());
}
