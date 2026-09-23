# plan11 — Analytical bug hunt and systemic repair plan

Updated: 2026-09-23. Scope: analytical source review, dependency refresh, and user-authorized release build follow-up. The final release build succeeded after upstream EXR and local dock API repairs. No tests were run; the 12 bug-hunt fixes below remain proposed.

## Status and reading order

- [x] Reconcile the current scan, cache, render, CLI, screenshot, and media paths with source.
- [x] Record concrete failures and related paths with source locations.
- [x] Update [AGENTS.md](AGENTS.md) and [DIAGRAMS.md](DIAGRAMS.md) from the current code.
- [ ] Obtain review of this plan before implementation.
- [x] Rebuild the GitNexus graph (5,710 nodes reported) and run upstream impact analysis before the `DockTabs` edit: LOW risk, direct caller `App::run_frame`, two affected UI processes.
- [ ] Check graph freshness and run upstream impact analysis before each remaining symbol edit. Report HIGH/CRITICAL impact before edits.
- [ ] Implement fixes in the dependency order below, preserving features and avoiding parallel implementations.
- [ ] Review diffs and run GitNexus change detection before any commit. A commit is outside this analytical pass.
- [x] Record the authorized `python bootstrap.py b` build sequence: initial upstream EXR failure, post-push dock API failure, then successful release build. No tests were run.

## Dependency upgrade status and compatibility review

The user separately requested `cargo upgrade -i` across dependency versions. The dependency manifest and lockfile have been updated. The initial authorized release build failed before application compatibility could be established; the final release build succeeded after dependency and dock API repairs. The dependency changes are separate from the proposed bug fixes below.

- [x] `cargo upgrade -i` advanced 10 unpinned workspace requirements across `glam`, `pollster`, the `eframe`/`egui` family, `egui_dock`, `egui-phosphor`, `jwalk`, and `sysinfo` (`Cargo.toml:63-78,94-106`). The exact OpenH264 pin was then advanced from `=0.9.7` to `=0.9.8` with `cargo upgrade -i --pinned -p openh264` (`Cargo.toml:136`; `Cargo.lock:5665-5667`). Together these are 11 advanced dependency requirements in the reviewed manifest diff.
- [x] Keep `bincode = "=1.3.3"` as an explicit exception (`Cargo.toml:56-60`; `Cargo.lock:743-749`). `cargo upgrade -i` temporarily selected 3.0.0, whose upstream package intentionally fails compilation; the cache uses the bincode 1 `DefaultOptions` API (`src/cache.rs:273-309`). The lock was restored with `cargo update -p bincode --precise 1.3.3`. This is the latest usable 1.x codec for the existing cache format, not an unnoticed skipped upgrade.
- [x] Remove the obsolete Git `[patch.crates-io]` egui 0.35 block. The current lock resolves published `eframe` and `egui` 0.36.2 (`Cargo.lock:2713-2717,2750-2754`) and one `wgpu` 30.0.1 package (`Cargo.lock:8536-8540`); the old patch rationale no longer applies.
- [ ] Review each upgraded direct and transitive API against all workspace consumers, especially `egui`/`eframe`/`egui-wgpu`, `glam`, `pollster`, `jwalk`, `sysinfo`, and OpenH264. Record concrete source incompatibilities before editing code; preserve all current features.
- [ ] Reconcile package/MSRV and platform-specific lockfile changes. The final release build passed; test and runtime compatibility remain unverified and require separate, justified checks.

### Own GitHub crate refresh

The lockfile now resolves the 2026-09-23 `main` commits below. These are dependency resolution facts, not evidence of source or runtime compatibility.

| Repository | Resolved commit | Evidence |
| --- | --- | --- |
| `ffmpeg-rs` | `2248c158e3ecb841b57b7b0dacd3cb09dbc59798` | `Cargo.lock:300` |
| `oiio-rs` | `106b9bca0b935b20d10b7699cf80d4045ba33883` | `Cargo.lock:8169` |
| `vcv-rs` | `a5b09903731645f62b6a36012d9589258dda406b` | `Cargo.lock:8125`; `crates/xtask/Cargo.toml:16` now explicitly selects `main` |
| `exr-rs` | `b987a09d95da0cd6806b828dcc1b3a17811dcf48` | `Cargo.lock:2479` |
| `jph-rs` | `2a4dd48fbed59527689ca85ebf753ac934cbcfc9` | `Cargo.lock:4415` |

`oidn-rs`, `codec-simd-rs`, `lcms-rs`, and `murmur3` already resolved to their current `main` commits and did not change. `kvz-rs` remains revision-pinned; its pinned SHA equals current `main`. The lockfile update removed transitive `vfx-primaries` (present in the prior lock diff, absent from the current lock); its API impact is unassessed. The earlier verified `exr-rs` comparison from `44fed1b` to `2629a64` spans 63 commits and 168 changed files (+19,653/-625 lines, including EXR fixtures); it describes that earlier range, not the additional move to `b987a09`. A single-method compatibility fix does not by itself establish full API parity.

- [x] Refresh these five own GitHub repositories and resolve the lockfile. On the later user-requested refresh, targeted `cargo update -p vfx-core --precise 30d107e5a384071068f9e3ecd6bcb787d936a189` also advanced EXR, then `cargo update -p exr-core --precise b987a09d95da0cd6806b828dcc1b3a17811dcf48` completed the lockfile update. `git diff --check` and `cargo metadata --locked --no-deps` exited successfully.
- [ ] Review affected FFmpeg/video, OIIO/image/color, EXR/JPH, and Windows `xtask` API surfaces against the new commits. Trace whether the disappearance of `vfx-primaries` changes any consumer contract.
- [ ] Preserve the `kvz-rs` revision pin until its ownership policy is reviewed. Do not infer a need to modify `oidn-rs`, `codec-simd-rs`, `lcms-rs`, or `murmur3` merely because they are Git dependencies.

### Release build observation

The user authorized `python bootstrap.py b` (release build through `xtask`) on 2026-09-23. The first filesystem MCP `run_command` call timed out after about 300 seconds despite `timeoutMs=1800000`; the bootstrap process continued. Its later stderr log confirmed `Error: Build failed!` and `target\debug\xtask.exe build` exit code 1. The failed MCP call and the completed first build had different outcomes; the build result came from the log.

The earlier build reported three `E0599` errors in the `oiio-rs` checkout at commit `2c0fb00`, `crates/oiio/vfx-io/src/exr.rs:241,421,429`: `PixelType::size()` is absent. That build used the previous `oiio-rs` and `exr-rs` SHAs. A read-only inspection of intermediate heads (`30d107e` and `b987a09`) still showed the three calls and the fallible replacement. The local `oiio-rs` main branch already contained the correct EXR repair in commit `244722b` (15 commits ahead of origin). Its lockfile was updated to `exr-rs` `b987a09`; `cargo check -p vfx-io --locked` passed, then commit `106b9bca0b935b20d10b7699cf80d4045ba33883` was pushed to `origin/main`. Untracked `.claude/` and `task.md` in `oiio-rs` were preserved. Squarebob's local layered EXR entry points call the `vfx_io` wrapper (`crates/media-encoder/src/io/exr_layered.rs:5-27`), so the previously observed dependency compile error prevented local EXR behavior from being checked. Current `exr-rs` commit `b987a09` still omits that method from `crates/exr-core/src/attr/channel_list.rs:87-94` in favor of fallible `exr_core::misc::pixel_type_size` (`crates/exr-core/src/misc.rs:81`). The pushed `oiio-rs` checkout now wraps the canonical helper as fallible `pixel_size` (`crates/oiio/vfx-io/src/exr.rs:174-177`); no `PixelType::size()` calls remain in that file. The old build log also records two `glam` deprecation warnings at `crates/render-shared/src/lib.rs:1304,1325`.

- [x] Resolve the `oiio-rs`/`exr-rs` API mismatch in the separately owned upstream work; `cargo check -p vfx-io --locked` passed and Squarebob now locks the pushed `oiio-rs` commit `106b9bc`.
- [x] Continue the release build past `vfx-io`: the first post-push run exposed `E0046` because `egui_dock` 0.21.1 requires `TabViewer::id` (`src/app/dock.rs:105`). After a fresh GitNexus analysis reported LOW upstream impact (direct caller `App::run_frame`; two UI processes), `DockTabs::id` was added at `src/app/dock.rs:108-110` using `egui::Id::new(tab)`; `DockTab` derives `Hash` at `src/app/dock.rs:10-11`.
- [x] Run `python bootstrap.py b` again: exit code 0; bootstrap reported build success in 1m4s, and Cargo finished its release profile in 1m02s. Treat the two `glam` warnings as migration work rather than build failure.
- [ ] Review build warnings without assuming dead code: deprecated `glam` calls at `crates/render-shared/src/lib.rs:1304,1325` and `src/cli_test.rs:162,163`; unused `serialize_cache` at `src/cache.rs:273` and `ScanRoot::from_canonical_path` at `src/path_key.rs:43`. Trace tests, features, and consumers before removal.
- [ ] Audit the separate deep-EXR pixel-size mapping in upstream `oiio-rs/crates/oiio/vfx-io/src/exr_deep.rs:333-340`: it maps `PixelType::NumPixelTypes` to zero instead of using the fallible canonical helper. The reader validates channel types at `exr_deep.rs:388-391`, so this is a single-source-of-truth review item, not a demonstrated exploitable failure. Do not edit the separately owned upstream repository in this task.

Evidence: initial failure in `C:\Users\joss1\.filesystem-mcp-rs\tmp\run_command_1790192223146_4684d9f4-05b7-43c9-b180-adda6ff13005_stderr.log:576-647`; final success in `C:\Users\joss1\.filesystem-mcp-rs\tmp\run_command_1790197892061_e7381986-6bf6-4643-b103-4830f22c83d1_stdout.log:14-17` and warnings/release completion in the matching `_stderr.log:15-55`.

## Coverage boundary and remaining audit

The 12 findings below are confirmed for the cited source paths; they are not a claim that every workspace crate or codepath was audited. This pass examined scan/cache/exclusion orchestration, CLI/screenshot, 2D/3D render and shared readback, and media encoding/sequence export. Before declaring a repository-wide hunt complete, inspect the remaining members and their integration edges:

- [ ] Trace path-tracing and acceleration ownership through `crates/pt-core/`, `crates/pt-wavefront/`, `crates/pt-megakernel/`, and `crates/bvh-gpu/`; distinguish active feature paths from planned tile and BVH settings.
- [ ] Trace material, surface, and color contracts through `crates/pt-mats/`, `crates/pt-material/`, `crates/standard-surface/`, and `crates/color-pipeline/`.
- [ ] Trace composition and media source boundaries through `crates/playa-ae/` and the rest of `crates/media-encoder/`, including codec/container UI compatibility.
- [ ] Review `crates/squarebob-widgets/`, `crates/egui-colorpicker/`, and remaining `src/app/` settings/UI paths for inactive controls, state duplication, and accessibility issues.
- [ ] Review packaging and developer automation in `crates/xtask/`, plus remaining `crates/treemap/`, `crates/render-shared/`, `crates/gpu-mem/`, and `crates/pt-denoise-oidn/` paths.
- [ ] Cross-check public and feature-gated APIs, platform branches, and all `#[allow(dead_code)]` sites before proposing deletion.

Literal Rust `TODO`/`FIXME` search in this pass found `crates/media-encoder/src/dialogs/encode/encode.rs:1758,1803` (TIFF/TGA compression), `crates/render-3d/src/renderer3d/instance_collect.rs:3` (historical TODO4 roadmap mention), and `crates/pt-megakernel/src/compute.rs:406` (tile-slot design note). The latter two are references, not proof of unfinished runtime behavior. This inventory covers the searched Rust source and does not replace a full workspace audit of non-Rust files and features.

The root [PLAN.md](PLAN.md) describes an older, already migrated FFmpeg architecture. Current code instead calls `VideoEncoder` at `crates/media-encoder/src/dialogs/encode/encode.rs:1222-1296` and defines it in `video.rs:66-84`. Keep PLAN.md as a historical artifact; it is not an implementation checklist for this pass.

## Current dataflow

```text
CLI (src/main.rs:19-33, src/cli.rs:395-431)
  -> shared GpuContext -> eframe WgpuSetup::Existing (src/main.rs:145-185)
  -> App::new -> start_scan (src/app/scan_orchestration.rs:299-362)
       -> ScanRoot {display, canonical path, id} (src/path_key.rs:18-62)
       -> CacheService::load(generation) + scanner::spawn(generation)
       -> jwalk or NTFS MFT; NTFS may fall back to jwalk
       -> ScanMsg::Progress + terminal Completed/Partial/Cancelled/Failed
       -> generation/id gate -> install_tree -> display tree
       -> complete scan only: CacheService::store -> atomic_file::write
  -> ui_treemap (src/app/treemap_view.rs:20-98)
       -> CPU 2D pixel upload | GPU 2D native texture | 3D raster/PT native texture
       -> legacy CPU readback only when callback path unavailable/screenshot
  -> optional screenshot (src/app/screenshot.rs:14-59)
Media composition export:
  Comp::get_frame -> encode_sequence_from_comp -> VideoEncoder::push/finish
  or encode_image_sequence -> optional tonemap -> format writer
```

See [DIAGRAMS.md](DIAGRAMS.md) for Mermaid sequences and branches.

## Confirmed findings

### P1 — Identity and scan correctness

1. **Same canonical root can reject its own cache or exclusions under a different display spelling.** `ScanRoot::from_input` retains the input string while deriving `id` from the canonical path (`src/path_key.rs:18-40`, tests at `:116-123`). Cache and exclusion validation require both matching `id` and matching display strings (`src/cache.rs:531-542,798-810`; `src/exclusions.rs:135-149`). The UI then silently replaces rejected exclusions with an empty set (`src/app/scan_orchestration.rs:327-333`). Impact: scanning `.` then the canonical path causes a valid cache to fail validation; `load_cache` treats that failure as invalid and deletes the file (`src/cache.rs:473-474,777-787`). Exclusions fall back to an empty set and a later UI edit can save over the old policy (`src/app/mod.rs:491-495`). **Systemic repair:** make canonical root identity and canonical member paths authoritative for validation; store display spelling as presentation only. Distinguish corrupt/inaccessible exclusions from no file and show a recoverable warning rather than silently dropping policy. Review both cache and exclusion migration formats together.

2. **NTFS fallback status can disappear, and a disconnected progress receiver can suppress the terminal outcome.** In `src/scanner_ntfs.rs:409-415`, a full progress queue silently discards the optional `NtfsFallback` notification, so the UI can retain the wrong backend label and miss the warning. A disconnected progress receiver returns from `run_ntfs` before the standard fallback and terminal send. `poll_scan` treats disconnected terminal without outcome as a failed worker (`src/app/scan_orchestration.rs:518-526`). **Systemic repair:** keep informational progress independent of the terminal path; route all backend results through one terminal send, including fallback failures and cancellation. Inspect the standard scanner's terminal path (`src/scanner.rs:168-183`) when changing this.

### P1 — Render and export correctness

3. **An empty 3D scene can retain the previous frame.** `Renderer3D::render_to_view` returns before clearing or drawing the target for zero instances (`crates/render-3d/src/renderer3d/render.rs:128-137`). The UI may continue displaying the registered texture (`src/app/treemap_view.rs:1001-1024,1090-1123`). **Systemic repair:** render or clear the empty frame through the same target lifecycle as a nonempty frame, and reset pick/selection state in the same transition. Check the legacy `Renderer3D::render` branch for parity before consolidating.

4. **16-bit PNG/TIFF export may contain only 8-bit source precision.** Sequence export tonemaps every non-EXR floating frame to RGBA8 even when `bit_depth` is U16 (`crates/media-encoder/src/dialogs/encode/encode.rs:1891-1905`; `crates/media-encoder/src/frame.rs:386-406`). PNG/TIFF writers then promote each 8-bit channel by `* 257` (`encode.rs:1580-1592,1714-1725`). **Systemic repair:** choose conversion from requested output format and bit depth at one boundary; preserve F16/F32 precision until U16 quantization and apply tonemap at target precision when selected. Keep EXR and all channel modes intact.

5. **TIFF compression and TGA RLE controls are accepted but ignored.** UI exposes TIFF compression and TGA RLE (`crates/media-encoder/src/dialogs/encode/encode_ui.rs:1221-1250`); TIFF writer explicitly discards compression (`encode.rs:1758`), TGA writer ignores settings and records TODO (`encode.rs:1762-1804`). **Systemic repair:** route validated compression settings to writers that support each mode, or make unsupported modes explicit before export; do not silently replace requested behavior. Verify selected writer APIs before choosing dependency changes.

### P2 — State and API coherence

6. **Picking metadata can be stale and reverse lookup can find an inactive ID.** `reset_frame` retains all entries while resetting allocation (`crates/render-3d/src/picking.rs:64-69`); `alloc_id` skips updating size and directory flag when a path matches (`:74-93`). `id_for_path` searches all entries, including IDs at/above `next_id` (`:257-263`). **Systemic repair:** define one per-frame active-ID range, update metadata on every allocation, and constrain all lookups to active IDs. Preserve stable ID reuse during animation.

7. **3D render entry points duplicate scene setup and already diverge.** CPU-readback `Renderer3D::render` at `crates/render-3d/src/lib.rs:1276-1365` and native `render_to_view` at `crates/render-3d/src/renderer3d/render.rs:21-137` each perform PT option normalization, layout/cache setup, and instance collection. Native rendering forces `xray_alpha = 1.0` in raster mode (`render.rs:37-41`), while the legacy entry does not at `lib.rs:1296-1314`. **Systemic repair:** share the scene preparation and pass encoding path; choose output target/readback as a parameter or mode, preserving all rendering features. Inspect both paths' visual semantics before removing any branch.

8. **Screenshot completion is committed before capture/save succeeds.** `src/app/screenshot.rs:31-59` sets `screenshot_taken` first, logs save errors without changing outcome, and closes the viewport when `exit_after_screenshot` is set even if no pixels or save failure occurred. **Systemic repair:** make capture/save a result-bearing operation; mark success and close only after a completed save, and expose failures through the existing UI/CLI error state. The default path is also inconsistent: help/log claim `temp/screenshot.png` (`src/cli.rs:137`; `src/main.rs:135-142`) while actual output uses `temp_dir()/squarebob_screenshot.png` (`screenshot.rs:34-39`).

9. **CLI parser consumes a following option as a missing value and accepts non-finite delays.** `--mode`, `--backend`, `--screenshot`, and similar arms increment `i` without guarding whether the next token is another flag (`src/cli.rs:395-431`); unknown flags only print and continue (`:909-915`). `f32::parse` accepts NaN/infinity for screenshot delay (`:425-426`), which feeds elapsed-time comparison and repaint (`src/app/screenshot.rs:25-29`). Help says `squarebob-rs` while `Cargo.toml:11-13` names the binary `squarebob` (`src/cli.rs:112-115,245-251`). **Systemic repair:** parse value-bearing options through one checked value consumer, reject unknown/missing/invalid/non-finite arguments with a structured error, and derive/help-test executable naming from the binary identity. Audit all numeric options, not only screenshot delay.

10. **Two serialized fields claim TIFF bit depth ownership.** `TiffSequenceSettings.bit_depth` is serialized (`crates/media-encoder/src/dialogs/encode/encode.rs:879-891`) while `SequenceSettings.bit_depth` is separately serialized and passed to the TIFF writer (`:920-936,1669-1683`). **Systemic repair:** determine which setting has actual UI and project consumers, then keep one authoritative bit-depth field; explicitly handle old serialized values according to the chosen project-schema policy. Do not delete the field until serde consumers are traced.

11. **GPU readback still has a recoverability gap at allocation and a narrower panic surface.** Checked `TextureReadbackLayout` and `Result` propagation already exist (`crates/render-core/src/lib.rs:551-605,643-747`), and `map_buffer_read` handles callback/channel failure (`:807-827`). Pixel output still uses `Vec::with_capacity(layout.output_size)` (`:740`), which can abort instead of returning a readback error under allocation pressure. `get_mapped_range().expect` remains after a successful map (`:830-835`); verify the API guarantee before treating it as removable. **Systemic repair:** use fallible reservation in the central helper and keep callers on the shared readback API. The historical double-`unwrap` and unchecked width arithmetic claims are no longer current.

### P3 — Incomplete or potentially intentional code

12. `TiffCompression` and TGA RLE TODOs are active missing behavior, not dead code (`encode.rs:1758,1803`). The PT gbuffer tile-slot comment refers to planned per-tile packing (`crates/pt-megakernel/src/compute.rs:401-406`); trace host allocation and the historical design before changing it. The depth texture field is retained to keep its view alive (`crates/render-3d/src/targets.rs:10-12`). Megakernel methods at `crates/pt-megakernel/src/compute.rs:1622,1661,1699` are called from `crates/render-3d/src/pt/megakernel/render.rs:390-400`. `high_quality` and `wide_bvh` are reserved BVH options (`crates/bvh-gpu/src/bvh_gpu/mod.rs:125-140`); dirty-state setters are documented migration surfaces (`src/app/settings/dirty.rs:115-154`). `EncodeStage::Error` is matched in the UI (`encode.rs:1091-1099`; `encode_ui.rs:418,494,794`). No feature-removal recommendation follows from this audit. The earlier missing-`// SAFETY:` item is closed by a source scan of `render-3d`, `bvh-gpu`, and `pt-megakernel`.

## Dependency-ordered implementation checklist

- [ ] **A. Contract and identity:** decide canonical root/cache/exclusion policy, error presentation, and serialized schema rules; test alias spellings and corrupt files analytically before editing. Inspect GitNexus impact of `ScanRoot`, cache validation, and exclusions.
- [ ] **B. Terminal lifecycle:** centralize NTFS/standard terminal emission and ensure optional progress cannot suppress it. Keep cancellation and partial outcomes distinct.
- [ ] **C. Render state:** consolidate 3D scene preparation; establish empty-scene clear and active picking metadata in both native and readback paths.
- [ ] **D. Export conversion:** establish one format/bit-depth conversion contract for PNG, TIFF, TGA, and EXR; honor compression settings; trace all serde/UI consumers before dropping duplicated TIFF depth.
- [ ] **E. Input and screenshot:** use one strict CLI value parser and a result-bearing screenshot completion; align help/log/default path.
- [ ] **F. Shared readback:** replace infallible pixel reservation centrally; inspect mapped-range API contract and all readback callers.
- [ ] **G. Evidence gate:** review each exact diff and relevant feature/platform path. Run focused checks only if a concrete unresolved question makes execution essential; otherwise keep this pass analytical. Run GitNexus `detect_changes` before committing.

The official [Rust API Guidelines checklist](https://rust-lang.github.io/api-guidelines/checklist.html) informs validation, useful error types, failure behavior, and intermediate states (C-VALIDATE, C-GOOD-ERR, C-FAILURE, C-CUSTOM-TYPE, C-INTERMEDIATE). It is guidance for the proposed APIs, not a claim that every internal method must be redesigned. Context7 and Fetch MCP were unavailable in this session; no dependency API claims here rely on them.

## Open questions requiring source/API confirmation during implementation

- Which TIFF/TGA writer supports each exposed compression mode without losing channel depth or alpha?
- Which serialized project settings consume `TiffSequenceSettings.bit_depth`, and what migration behavior is acceptable?
- What exact output should an empty 3D scene show (background/environment/selection clear) in raster and PT modes?
- Does wgpu guarantee `get_mapped_range` after a successful whole-buffer map under the pinned version?
- Which PT tile-slot comment describes an unfinished feature versus an obsolete design note?

## Verification record

- Source inspection: completed for cited paths.
- Application run: not performed.
- Build: first release build failed in `vfx-io` (`E0599`, three sites); first post-push build got past it but failed on `DockTabs` `E0046`; final `python bootstrap.py b` release build exited 0. The initial MCP call timed out while its child process continued; its later failure was read from the log.
- Tests: not performed.
- Fix implementation: upstream EXR compatibility and local `DockTabs::id` migration completed. The 12 bug-hunt findings remain open.
- GitNexus graph: rebuilt (5,710 nodes reported); `DockTabs` upstream impact was LOW before the edit. Check freshness again before future edits.
