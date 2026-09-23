# PLAN10 — Remove `ffmpeg-sys-next` and rebuild video export

Updated: 2026-07-15  
Workspace: `C:\projects\projects.rust.cg\cglibs\squarebob-rs`  
Branch: `main`  
Squarebob checkpoint: `fe367d5b3633f990707b7e54f471ef89a9c8d31e`  
Required media source: `ssh://git@github.com/ssoj13/ffmpeg-rs.git`  
Verified ffmpeg-rs ref: `c0a8e10b7416ca7a24223f42e3e137809d1d20b1`

## Mission

Remove `playa-ffmpeg` and the complete `ffmpeg-sys-next` dependency chain from Squarebob. Rebuild video export on explicit CPU encoders and the current `ffmpeg-rs` codec/container primitives. Preserve rendering, progress, cancellation, output validation, codec profiles, quality modes, and container correctness. Hardware encode support may be removed. No temporary raw-video files. No external FFmpeg installation. No silent setting fallback.

Final result must support:

- H.264/AVC through bundled OpenH264.
- H.265/HEVC Main 8-bit and Main10 10-bit through `ffmpeg-rs`.
- AV1 through `ffmpeg-rs`.
- All six ProRes profiles through `ffmpeg-rs`.
- MP4/MOV muxing through `ffmpeg-rs::MovWriter`.
- Existing composition rendering, tonemapping, progress, and cancellation paths.
- Clean dependency graph with zero `playa-ffmpeg` and zero `ffmpeg-sys-next`.

## Emergency resume summary

No implementation edits have been made for this migration yet.

Current Squarebob worktree baseline:

```text
## main...origin/main
 M AGENTS.md
 M CLAUDE.md
```

Those two files are user-owned changes. Preserve them. Do not overwrite, revert, stage, or fold them into an unrelated commit without explicit user approval.

Current ffmpeg-rs worktree baseline:

```text
## main...origin/main
 M AGENTS.md
 M CLAUDE.md
```

Do not edit the ffmpeg-rs repository. Consume it as a pinned SSH git dependency. Its current `main` and `origin/main` both point to `c0a8e10b7416ca7a24223f42e3e137809d1d20b1`.

Three prior subagents failed because of external GSD usage limits. Do not wait for them. Do not introduce GSD into this workflow.

Communicate every step to the user in short Russian commentary. Code, comments, commits, and Markdown remain English.

## Verified current defects

1. The unwanted native FFmpeg chain is still active.

   - `Cargo.toml:111` declares `playa-ffmpeg`.
   - `crates/media-encoder/Cargo.toml:9` enables the old `ffmpeg` feature by default.
   - `crates/media-encoder/Cargo.toml:10` maps that feature to `playa-ffmpeg`.
   - `crates/media-encoder/Cargo.toml:22` declares the optional dependency.
   - `Cargo.lock:2957` contains `ffmpeg-sys-next`.
   - `Cargo.lock:5533` contains `playa-ffmpeg`.

2. Media code exposes the old binding as public API.

   - `crates/media-encoder/src/lib.rs:7`
   - `crates/media-encoder/src/lib.rs:8`
   - `crates/media-encoder/src/lib.rs:18`
   - `crates/media-encoder/src/lib.rs:19`

3. Encoder selection is coupled to FFmpeg registry names and hardware vendors.

   - `crates/media-encoder/src/dialogs/encode/encode.rs:401`
   - `crates/media-encoder/src/dialogs/encode/encode.rs:460`
   - `crates/media-encoder/src/dialogs/encode/encode.rs:1241`
   - `crates/media-encoder/src/dialogs/encode/encode_ui.rs:1055`
   - `crates/media-encoder/src/dialogs/encode/encode_ui.rs:1395`

4. The encode loop is a monolith containing renderer orchestration, pixel conversion, codec configuration, packet draining, muxing, progress, and cancellation.

   - `crates/media-encoder/src/dialogs/encode/encode.rs:1379`
   - `crates/media-encoder/src/dialogs/encode/encode.rs:2031`
   - `crates/media-encoder/src/dialogs/encode/encode.rs:2063`
   - `crates/media-encoder/src/dialogs/encode/encode.rs:2091`

5. Documentation and packaging still claim FFmpeg-system integration.

   - `README.md:258`
   - `README.md:431`
   - `crates/xtask/src/main.rs:957`
   - `crates/xtask/src/env_setup.rs:6`
   - `crates/xtask/src/env_setup.rs:28`

6. Existing serialized settings contain backend/vendor states that will cease to exist.

   - `crates/media-encoder/src/dialogs/encode/encode.rs:95`
   - `crates/media-encoder/src/dialogs/encode/encode.rs:135`
   - `crates/media-encoder/src/dialogs/encode/encode.rs:157`
   - `crates/media-encoder/src/dialogs/encode/encode.rs:316`
   - `crates/media-encoder/src/dialogs/encode/encode.rs:460`

## Verified dependency capabilities

Use these as implementation evidence. Re-read at the pinned ref before coding if the checkout changes.

- `../ffmpeg-rs/crates/av-codec/Cargo.toml:27`: `enc-hevc` feature.
- `../ffmpeg-rs/crates/av-codec/Cargo.toml:30`: `enc-av1` feature.
- `../ffmpeg-rs/crates/av-codec/src/codec.rs:105`: registry lookup by encoder name.
- `../ffmpeg-rs/crates/av-codec/src/codec.rs:199`: HEVC `annexb_to_length_prefixed`, `build_hvcc`, `hvcc_from_au`.
- `../ffmpeg-rs/crates/av-codec/src/codec.rs:207`: AV1 `av1c_from_au`.
- `../ffmpeg-rs/crates/av-codec-h265-enc/encoder.rs:101`: one build advertises 8-bit and 10-bit input.
- `../ffmpeg-rs/crates/av-codec-h265-enc/encoder.rs:826`: runtime input bit-depth selection.
- `../ffmpeg-rs/crates/av-codec-h265-enc/mp4.rs:74`: HEVC sample conversion.
- `../ffmpeg-rs/crates/av-codec-h265-enc/mp4.rs:116`: `hvcC` generation from first AU.
- `../ffmpeg-rs/crates/av-codec-av1-enc/av_codec_av1_enc.rs:480`: `av1C` generation.
- `../ffmpeg-rs/crates/av-codec-av1-enc/av_codec_av1_enc.rs:595`: AV1 speed is currently hard-coded to 6.
- `../ffmpeg-rs/crates/av-codec-av1-enc/av_codec_av1_enc.rs:962`: AV1 QP mapping.
- `../ffmpeg-rs/crates/av-codec-av1-enc/av_codec_av1_enc.rs:970`: AV1 bitrate mapping.
- `../ffmpeg-rs/crates/av-format-movenc/av_format_movenc.rs:235`: `MovWriter::new`.
- `../ffmpeg-rs/crates/av-format-movenc/av_format_movenc.rs:261`: H.264/HEVC/AV1 stream declaration.
- `../ffmpeg-rs/crates/av-format-movenc/av_format_movenc.rs:313`: ProRes stream declaration.
- `../ffmpeg-rs/crates/av-swscale/src/context.rs:781`: conversion context construction.
- `../ffmpeg-rs/crates/av-swscale/src/swscale.rs:279`: frame conversion.
- `../ffmpeg-rs/crates/av-util-frame/src/frame.rs:330`: frame allocation.
- `../ffmpeg-rs/crates/av-util-frame/src/frame.rs:535`: frame buffer allocation.
- `../ffmpeg-rs/crates/av-graph/src/nodes.rs:1280`: reference codec/mux lifecycle.
- `../ffmpeg-rs/crates/av-graph/src/nodes.rs:1583`: reference pixel-format negotiation.
- `../ffmpeg-rs/crates/av-graph/tests/transcode_gate.rs:916`: regression coverage for capped-run finalization.

Important limitation: `EncodeMuxSink` is graph-internal and resolves frame rate from an upstream demux `In` node. Squarebob generates frames from a composition. Do not force Squarebob through graph demux, temporary image sequences, or temporary raw-video files. Reuse the lower-level primitives and lifecycle semantics.

H.264 encoder status:

- `ffmpeg-rs` currently has H.264 decode, not H.264 encode.
- The inspected `heif-rs` AVC codec remains an unsupported scaffold.
- Use `openh264 = 0.9.7` with its bundled `source` feature.
- OpenH264 exposes Baseline/Main/High profiles, Low/Medium/High complexity, quantizer, bitrate, frame rate, and thread configuration.
- Build `avcC` from the first encoded key access unit and convert Annex-B samples to length-prefixed MP4 form. No equivalent public builder was found in the pinned `ffmpeg-rs`; keep this logic H.264-local and test it byte-for-byte.

## Architecture target

Create a single streaming video-export subsystem inside `media-encoder`.

Suggested module layout:

```text
crates/media-encoder/src/video/
  mod.rs
  session.rs
  frame.rs
  avcodec.rs
  h264.rs
  mux.rs
```

Responsibilities:

- `session.rs`: one `VideoEncodeSession`; validates settings; owns lifecycle; routes frames; guarantees exactly-once finalization.
- `frame.rs`: checked geometry/stride/plane allocation and RGB(A) to codec pixel-format conversion.
- `avcodec.rs`: shared `AVCodecContext` adapter for HEVC, AV1, and ProRes. No codec-name branching in the caller.
- `h264.rs`: OpenH264 configuration, frame submission, SPS/PPS extraction, `avcC` construction, Annex-B conversion.
- `mux.rs`: one `MovWriter<File>` adapter; header-on-first-packet rules; packet timing; atomic output commit.
- `mod.rs`: narrow public domain API and error types.

Core API shape:

```rust
pub struct VideoEncodeSession { /* owns encoder + mux + output transaction */ }

impl VideoEncodeSession {
    pub fn open(settings: ValidatedVideoSettings, size: FrameSize) -> Result<Self, VideoEncodeError>;
    pub fn push_frame(&mut self, frame: RenderedVideoFrame<'_>) -> Result<(), VideoEncodeError>;
    pub fn finish(self) -> Result<VideoEncodeSummary, VideoEncodeError>;
}
```

Use an enum for codec backends, not a trait-object hierarchy:

```rust
enum CodecBackend {
    H264(H264Encoder),
    Hevc(AvCodecEncoder),
    Av1(AvCodecEncoder),
    ProRes(AvCodecEncoder),
}
```

One common encoded packet model:

```rust
struct EncodedSample {
    data: Vec<u8>,
    pts: i64,
    dts: i64,
    key: bool,
}
```

Rules:

- Rendered frame ownership remains outside the encoder session.
- No codec backend writes files.
- No UI type reaches codec code.
- No mux type reaches rendering code.
- Every multiplication involving width, height, stride, plane size, bitrate, or timestamp uses checked arithmetic and fallible conversion.
- FPS becomes a validated rational value. Do not derive long-run timestamps by repeatedly adding `f32`.
- The muxer owns the constant-frame-rate timebase.
- Encoder delay is drained exactly once.
- Container is finalized exactly once.
- Cancellation never publishes a corrupt target file.
- Encode into a sibling temporary file, call `MovWriter::finish`, close it, then atomically rename to the requested target.
- On error or cancellation, delete only the session-owned temporary file. Never delete a pre-existing user target.
- Never unwrap codec, conversion, channel, file, or mux errors.

## Execution checklist

### 0. Re-establish safety baseline

- [ ] Run `git status --short --branch`.
- [ ] Confirm only user-owned `AGENTS.md` and `CLAUDE.md` modifications exist before migration edits.
- [ ] Run `gitnexus-rs graph-status` or the equivalent MCP graph-status tool.
- [ ] Re-run upstream impact for every public symbol to be removed or structurally changed:
  - `encode_sequence_from_comp`
  - `EncoderSettings`
  - `EncodeDialogSettings`
  - `H26xSettingsMut`
  - `EncoderImpl`
  - `VideoCodec::is_available`
  - `init_ffmpeg`
- [ ] Abort and ask user if any impact report is HIGH or CRITICAL.
- [ ] Query downstream dependencies/sister-sites before changing shared settings or encode entry points.
- [ ] Do not run broad tests yet.

Prior checked impacts for `encode_sequence_from_comp`, `VideoCodec`, and `EncoderImpl` were LOW. Re-run after graph freshness check; do not trust stale results blindly.

### 1. Replace the dependency graph

- [ ] Remove `playa-ffmpeg` from `Cargo.toml:111`.
- [ ] Add only required `ffmpeg-rs` packages using the same SSH URL and exact `rev = "c0a8e10b7416ca7a24223f42e3e137809d1d20b1"`.
- [ ] Expected packages:
  - `av-codec` with `features = ["enc-hevc", "enc-av1"]`
  - `av-codec-core`
  - `av-format`
  - `av-swscale`
  - `av-util-frame`
  - `av-util-pixfmt`
- [ ] Add exact `openh264 = "=0.9.7"` with bundled source support.
- [ ] Replace the `media-encoder` `ffmpeg` feature with a backend-neutral `video` feature, or make video support unconditional if all required dependencies are always shipped.
- [ ] Do not retain a compatibility alias named `ffmpeg`.
- [ ] Remove `pub use playa_ffmpeg as ffmpeg` and `init_ffmpeg` from `crates/media-encoder/src/lib.rs`.
- [ ] Regenerate `Cargo.lock` through Cargo.
- [ ] Run dependency-only checks:
  - `cargo tree -i ffmpeg-sys-next -e features` must fail with “package ID specification did not match”.
  - `cargo tree -i playa-ffmpeg -e features` must fail likewise.
  - `cargo tree -p media-encoder -e features` must show the pinned SSH `ffmpeg-rs` crates and OpenH264.
- [ ] Update third-party notices if the repository has such a file. Do not change Squarebob's package license merely because the user allowed it; change only if the actual linked dependency terms require it.

### 2. Replace backend/vendor settings with typed codec settings

- [ ] Delete `EncoderImpl` and every `encoder_impl` field. Hardware sacrifice is explicitly approved.
- [ ] Delete hardware selection controls from both duplicated UI paths in `encode_ui.rs`; unify the two renderers instead of editing only one.
- [ ] Replace string presets/profiles with typed enums:
  - `H264Complexity::{Low, Medium, High}`
  - `H264Profile::{Baseline, Main, High}`
  - `HevcPreset` covering the actual kvz preset set
  - `HevcProfile::{Main, Main10}`
  - `ProResProfile` remains six typed variants
- [ ] Rename `QualityMode::CRF` to a backend-true constant-quality name such as `Quantizer`. OpenH264, kvz, and rav1e consume quantizers, not identical CRF semantics.
- [ ] Give each codec its real validated range:
  - H.264 QP: verify OpenH264 accepted range from crate/source before encoding.
  - HEVC QP: 0..=51.
  - AV1 quantizer: 0..=255.
  - Bitrate: checked kbps-to-bits/s conversion.
- [ ] Remove AV1 vendor strings such as `p4`, `libsvtav1`, and `libaom`.
- [ ] Preserve every real capability. Do not leave controls that are ignored by the backend.

AV1 speed gate:

- The pinned `ffmpeg-rs` encoder hard-codes rav1e speed 6 at `../ffmpeg-rs/crates/av-codec-av1-enc/av_codec_av1_enc.rs:595`.
- Before deleting the current AV1 preset/speed control, re-check the exact pinned API.
- If no public speed option exists, stop and tell the user. Do not silently ignore the setting. Do not patch the ffmpeg-rs checkout. Do not add a second direct rav1e backend without explicit approval because that creates two AV1 codepaths.

Serialization policy:

- User explicitly permits discarding Squarebob's own backward compatibility.
- Remove obsolete variants cleanly rather than retaining dead serde aliases.
- Still add round-trip tests for the new settings schema.
- Project/settings load errors must be explicit and recoverable, not panics.

### 3. Extract rendering from encoding

- [ ] Keep `encode_sequence_from_comp` as orchestration only.
- [ ] Reuse existing composition evaluation, preview/runtime override restoration, tonemapping, and progress messages.
- [ ] Move all codec names, pixel formats, packet loops, and mux calls out of `encode.rs`.
- [ ] Introduce `RenderedVideoFrame` with explicit:
  - width
  - height
  - row stride
  - channel order
  - bit depth
  - alpha presence
  - color metadata
  - borrowed bytes
- [ ] Make the renderer return one canonical representation. Do not create separate rendering paths per codec.
- [ ] Preserve alpha for ProRes 4444/4444 XQ.
- [ ] Apply HDR-to-LDR tonemapping only for codecs/profiles that require it.
- [ ] Keep 10-bit source precision for HEVC Main10 and eligible ProRes/AV1 paths. Do not tonemap or truncate merely to satisfy an 8-bit staging buffer.
- [ ] Add checked geometry validation before allocation or conversion.

### 4. Build one pixel-conversion path

- [ ] Use `av-swscale` for RGB(A)/planar conversion instead of hand-written duplicated converters.
- [ ] Negotiate target format from codec/profile:
  - H.264: I420/YUV420P 8-bit.
  - HEVC Main: YUV420P 8-bit.
  - HEVC Main10: YUV420P10LE.
  - AV1: YUV420P or YUV420P10LE according to the finalized settings model.
  - ProRes Proxy/LT/Standard/HQ: YUV422P10LE.
  - ProRes 4444/4444 XQ: YUVA444P10LE or the exact format declared by the selected encoder.
- [ ] Follow the registry-declared pixel-format negotiation model demonstrated at `../ffmpeg-rs/crates/av-graph/src/nodes.rs:1583`.
- [ ] Cache conversion contexts by source/destination geometry and formats.
- [ ] Preserve odd-dimension behavior explicitly:
  - reject codecs requiring even chroma geometry with a precise error, or
  - pad/crop through a documented policy that keeps display geometry correct.
- [ ] No unchecked plane pointer arithmetic.
- [ ] Unit-test stride padding, odd dimensions, zero dimensions, overflow dimensions, alpha, 8-bit, and 10-bit conversion.

### 5. Implement H.264 backend

- [ ] Configure OpenH264 once per session.
- [ ] Map typed profile, complexity, FPS, QP/bitrate, and threads explicitly.
- [ ] Disable silent frame skipping unless the UI setting explicitly requests it.
- [ ] Feed converted I420 frames.
- [ ] Parse the first key access unit into NAL units.
- [ ] Require SPS and PPS before declaring the MOV stream.
- [ ] Construct a full `avcC` box with checked lengths.
- [ ] Convert each Annex-B AU to four-byte length-prefixed MP4 sample form.
- [ ] Remove in-band SPS/PPS from samples only according to the chosen `avc1` contract.
- [ ] Preserve keyframe flags and monotonically correct PTS/DTS.
- [ ] Add parser tests for 3-byte and 4-byte start codes, missing SPS/PPS, truncated NALs, oversized NALs, and multiple parameter sets.
- [ ] Add real encode/mux/demux/decode smoke test.

### 6. Implement shared ffmpeg-rs codec backend

- [ ] Resolve encoders once:
  - HEVC: `hevc_kvz`
  - AV1: `av1_rav1e`
  - ProRes: verify and select the intended `prores_ks` or `prores_aw` registry entry
- [ ] Allocate one `AVCodecContext` per session.
- [ ] Set geometry, pixel format, timebase, profile, QP/bitrate, GOP, and private options before open.
- [ ] Use `open_with_opts` only for actual exposed options.
- [ ] Stamp every input frame with rational presentation time.
- [ ] Drain all available packets after every submitted frame.
- [ ] On finish, submit EOF exactly once and drain until codec EOF.
- [ ] Treat EAGAIN as flow control, not an error or completion.
- [ ] Preserve packet PTS, DTS, composition offsets, and key flags.
- [ ] HEVC:
  - derive `hvcC` from the first encoded AU
  - convert Annex-B samples with the provided helper
  - use Main/Main10 runtime pixel format, not compile-time feature forks
- [ ] AV1:
  - derive `av1C` from the first AU
  - preserve encoder packet form expected by `MovWriter`
- [ ] ProRes:
  - declare the stream with the exact profile FOURCC
  - do not invent codec config data
  - preserve alpha for 4444 profiles
- [ ] Match lifecycle behavior proven by `EncodeMuxSink`, but do not copy its graph-specific source-rate discovery or IR plumbing.

### 7. Implement transactional muxing

- [ ] Create a same-directory unique temporary target.
- [ ] Open `MovWriter` on that file.
- [ ] Use a validated rational media timescale and sample duration.
- [ ] Write stream header only when codec configuration is valid:
  - H.264: first SPS/PPS-bearing key AU
  - HEVC: first AU yielding valid `hvcC`
  - AV1: first AU yielding valid `av1C`
  - ProRes: immediately after encoder open/profile FOURCC resolution
- [ ] Write each sample with duration, composition offset, and sync flag.
- [ ] Finish/backpatch `mdat` and `moov`.
- [ ] Flush and close the file.
- [ ] Validate non-empty stream/sample count before commit.
- [ ] Atomically replace only according to an explicit overwrite policy.
- [ ] Cancellation and error remove the temporary file and leave any old target intact.
- [ ] Add a forced-cancel test and a forced-encoder-error test proving no corrupt final target is published.

### 8. Rewire UI and progress

- [ ] Keep one settings renderer per codec; remove duplicate H.26x hardware branches.
- [ ] Show only settings the selected backend consumes.
- [ ] Validate codec/container/profile combinations before spawning the worker.
- [ ] Preserve `EncodeProgress` and all current stages.
- [ ] Add explicit stages if useful:
  - validating
  - rendering
  - converting
  - encoding
  - finalizing
- [ ] Keep cancellation checks:
  - before render
  - after render
  - before frame submission
  - during packet drain
  - before final commit
- [ ] Ensure cancellation still drains/finalizes only the private temporary file when needed for safe teardown, then deletes it.
- [ ] Report backend errors to UI; no panic and no “success” for partial output.

### 9. Remove the old class of code

- [ ] Delete old FFmpeg imports and all `cfg(feature = "ffmpeg")` branches.
- [ ] Delete `get_encoder_name`.
- [ ] Delete hardware registry probing.
- [ ] Delete old swscale wrappers after the new centralized converter is proven.
- [ ] Delete dead vendor preset/profile tables in both UI paths.
- [ ] Search the whole repository, excluding historical reports, for:
  - `playa-ffmpeg`
  - `playa_ffmpeg`
  - `ffmpeg-sys-next`
  - `init_ffmpeg`
  - `EncoderImpl`
  - `h264_nvenc`
  - `hevc_nvenc`
  - `av1_nvenc`
  - `libx264`
  - `libx265`
  - `libsvtav1`
  - `libaom-av1`
- [ ] Update `README.md:258` and `README.md:431`.
- [ ] Remove FFmpeg-only environment setup from `crates/xtask/src/env_setup.rs` only after impact analysis confirms it has no other native dependency purpose.
- [ ] Update `crates/xtask/src/main.rs:957`.
- [ ] Keep historical `.bughunt/*.md` factual; do not rewrite history merely to make grep empty. Searches for acceptance should exclude historical artifacts.

### 10. Test after implementation chunks are reviewed

Do not start with full workspace builds. First complete code review of each chunk.

Unit tests:

- [ ] Settings validation and serde round-trip.
- [ ] Rational FPS and timestamp generation, including 23.976/29.97/59.94.
- [ ] Checked geometry/stride/size arithmetic.
- [ ] Pixel conversion: 8-bit, 10-bit, alpha, padded stride.
- [ ] H.264 Annex-B parser and `avcC` builder.
- [ ] Output transaction commit/rollback.
- [ ] Codec-specific profile/rate-control mapping.

Integration matrix:

- [ ] H.264 Baseline/Main/High, QP and bitrate.
- [ ] HEVC Main 8-bit, QP and bitrate.
- [ ] HEVC Main10 10-bit, QP and bitrate.
- [ ] AV1 8-bit and 10-bit if exposed by finalized UI.
- [ ] ProRes Proxy, LT, Standard, HQ, 4444, 4444 XQ.
- [ ] MP4 and MOV for every combination currently allowed by the UI.
- [ ] One-frame export.
- [ ] Multi-frame export.
- [ ] Frame-limited/cancelled export.
- [ ] Existing target preservation on failure.
- [ ] Odd geometry behavior.
- [ ] Large geometry overflow rejection.

Container validation:

- [ ] Re-open every completed output with `ffmpeg-rs::Demuxer`.
- [ ] Assert codec, dimensions, sample count, codec tag, and config box.
- [ ] Decode at least first/middle/last frame where a decoder exists.
- [ ] Verify H.264 `avc1` + `avcC`.
- [ ] Verify HEVC `hvc1` + `hvcC`.
- [ ] Verify AV1 `av01` + `av1C`.
- [ ] Verify ProRes profile FOURCC.
- [ ] Verify Main10 decoded format/bit depth.
- [ ] Verify ProRes 4444 alpha survives.

Build/check order:

```powershell
cargo fmt --all -- --check
cargo check -p media-encoder
cargo test -p media-encoder
cargo check --workspace --all-targets
cargo test --workspace
cargo clippy --workspace --all-targets --all-features -- -D warnings
cargo tree -i ffmpeg-sys-next -e features
cargo tree -i playa-ffmpeg -e features
```

The last two commands must report that the package ID does not match any package. If they print a dependency tree, migration is incomplete.

### 11. GitNexus scope verification

- [ ] Reanalyze the updated repository with `gitnexus-rs` after edits.
- [ ] Run `gitnexus-rs detect-changes` / MCP `detect_changes(scope="all")`.
- [ ] Inspect every affected process.
- [ ] Run downstream impact for the new shared encode session and changed settings builders.
- [ ] Query sister-sites for duplicated encode/mux/pixel-conversion paths.
- [ ] Use Cypher to confirm removed functions have zero remaining callers before deleting any additional dead code.
- [ ] Cross-check TODO/FIXME before deleting dead-code candidates.
- [ ] Do not delete unrelated dead code without user approval.

### 12. Commit and push

User previously requested all finished work in `main`.

Before commit:

- [ ] `git status --short`
- [ ] `git diff --check`
- [ ] `git diff --stat`
- [ ] Review exact diff; preserve user-owned `AGENTS.md` and `CLAUDE.md`.
- [ ] Confirm no local-path dependency.
- [ ] Confirm all `ffmpeg-rs` dependencies use SSH URL plus exact rev.
- [ ] Confirm no `playa-ffmpeg` / `ffmpeg-sys-next` dependency.
- [ ] Confirm tests and GitNexus change detection pass.

Commit only migration-owned files with an intentional message, for example:

```text
refactor(media): replace ffmpeg-sys video export backend
```

Push `main` only after successful verification. Do not force-push.

## Acceptance criteria

Migration is complete only when all statements are true:

- `cargo tree` contains no `playa-ffmpeg`.
- `cargo tree` contains no `ffmpeg-sys-next`.
- Build requires no system FFmpeg, vcpkg FFmpeg, FFmpeg DLL, or FFmpeg environment variables.
- H.264, HEVC Main, HEVC Main10, AV1, and all ProRes profiles produce valid files.
- Encoded files re-open through `ffmpeg-rs` and have correct sample counts/config boxes.
- Cancellation never leaves or replaces a corrupt final output.
- UI exposes no hardware state and no ignored setting.
- Rendering/tonemapping behavior remains intact.
- All size/time/bitrate conversions are checked.
- No codec/mux error path panics.
- Documentation and packaging match the new backend.
- GitNexus reports reviewed scope.
- Migration is committed and pushed to `origin/main`.

## Stop conditions

Stop and tell the user instead of improvising if any occurs:

- GitNexus reports HIGH or CRITICAL impact before an edit.
- The pinned SSH ref cannot be fetched.
- The current `ffmpeg-rs` public API cannot express a required existing setting, especially AV1 speed.
- A codec/profile requires silently dropping alpha, 10-bit precision, color metadata, cancellation, or valid container finalization.
- A proposed fix requires editing the ffmpeg-rs checkout.
- A proposed workaround needs temporary image/raw-video files or an external FFmpeg process.
- Existing user changes overlap migration-owned lines and cannot be preserved safely.
- Correct implementation requires a licensing decision not already covered by actual dependency licenses.

## First action for the next agent

1. Read this file completely.
2. Read repository `AGENTS.md`.
3. Report concise Russian status to the user.
4. Check both worktrees and the pinned ffmpeg-rs ref.
5. Refresh GitNexus.
6. Resolve the AV1 speed gate.
7. Begin dependency replacement only after LOW/MEDIUM impact is confirmed.
