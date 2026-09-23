# plan12 — Systemic bug-hunt implementation review

Updated: 2026-09-23. This report follows [plan11.md](plan11.md). It records the implemented source changes and the review work that remains. The final incremental `cargo check -p squarebob-rs --locked` passed after all edits. No tests or application run were performed.

## Scope and evidence

This pass follows the confirmed scan/cache, exclusions, CLI, screenshot, rendering, and media findings in plan11. The documentation review uses the current source and worktree diff. Static inspection establishes what the code does; it does not establish runtime behavior or test results. The workspace-wide coverage boundary and unchecked crates remain in [plan11.md](plan11.md#coverage-boundary-and-remaining-audit).

- [x] Read plan11, AGENTS.md, and DIAGRAMS.md in full.
- [x] Inspect current scan/cache/exclusion, NTFS fallback, CLI, and screenshot diffs.
- [x] Inspect the integrated screenshot/image-sequence call paths and final CLI/main wiring.
- [x] Inspect final render and media source changes, including native/legacy and format branches; runtime parity remains open below.
- [x] Update [AGENTS.md](AGENTS.md) ASCII flows and [DIAGRAMS.md](DIAGRAMS.md) Mermaid flows against inspected source.
- [x] Read every changed document in full; check links, source line references, and markdown structure.
- [ ] Present the final report for review after the previously authorized main-branch push. No tests were run; compilation-check results are recorded below.

## Implemented source changes reviewed

### Canonical scan identity and exclusions

- [x] Cache decode and in-memory validation now accept a matching canonical root ID without requiring identical display spelling (`src/cache.rs:523-535,786-797`). A regression test was added at `src/cache.rs:973-992`; it has not been run.
- [x] After tracing callers and cache format, the unused `CachedScan.scan_path` runtime field and one-line `serialize_cache` wrapper were removed; the v4 flat-cache `scan_path` field remains serialized for on-disk decoding (`src/cache.rs:30-48,274-307,523-535`). The unused `ScanRoot::from_canonical_path` helper was removed; alias tests now construct equivalent paths through `from_input` (`src/path_key.rs:19-48,100-111`; `src/cache.rs:973-992`). These are scoped dead-code cleanup, not removal of cache or identity behavior.
- [x] Exclusions load uses `try_exists` to distinguish an inaccessible path from a missing file, validates canonical root ID and member paths, then refreshes display spelling for presentation (`src/exclusions.rs:82-116,140-158`). Save accepts the authoritative `ScanRoot` rather than rebuilding it from a display string (`src/exclusions.rs:122-138`). Tests for alias spelling and parent traversal were added at `src/exclusions.rs:164-182`; they have not been run.
- [x] A failed exclusions load leaves an empty, untrusted in-memory value, reports a scan warning, and blocks editing until repair/rescan via root-ID check (`src/app/scan_orchestration.rs:327-351`; `src/app/mod.rs:491-507`). The status bar displays scan warnings (`src/app/status_bar.rs:74-83`). The stored exclusions file is not overwritten through this path.

### Scan terminal path

- [x] NTFS fallback notification now uses the unbounded terminal channel rather than the lossy progress queue (`src/scanner.rs:134-139`; `src/scanner_ntfs.rs:399-417`). This removes the progress-full and progress-disconnected exits identified in plan11. The UI handles `NtfsFallback` as a warning (`src/app/scan_orchestration.rs:518-523`).
- [x] Inspect standard and NTFS terminal emission/cancellation branches: each worker derives one outcome and sends one `ScanMsg::Terminal`; NTFS fallback may first send one separate warning (`src/scanner.rs:170-185`; `src/scanner_ntfs.rs:399-425`). A disconnected progress receiver still maps to cancellation without suppressing the terminal send (`src/scanner.rs:274-285`).

### CLI and screenshot

- [x] CLI parsing now returns `Result<CliOptions, String>`, uses shared value/numeric/finite validation, rejects missing values and unknown options, and checks screenshot delay (`src/cli.rs:346-431,657-660`). `main` prints the error and exits with status 2 (`src/main.rs:19-28`).
- [x] Screenshot capture now returns a validated `Result<Vec<u8>, String>`; `handle_screenshot` marks success and requests exit only after `save_png` succeeds. Failures remain visible in a retry/dismiss window (`src/app/screenshot.rs:12-77,79-159,163-174`). The image-sequence caller completes its request with `None` and cancels the source on capture failure (`src/app/image_sequence.rs:256-271`).
- [x] CLI help derives the binary name from `CARGO_BIN_NAME`, advertises the actual OS-temp screenshot filename and active OIDN/SVO options, and removes obsolete material-override flags (`src/cli.rs:113-189`). OIDN mode and quality are parsed into their typed enums once, then applied without a second string parser (`src/cli.rs:506-525`; `src/app/cli_apply.rs:39-49`). The parser resolves the default screenshot path once (`src/cli.rs:686-695`); `main` logs it (`src/main.rs:140-144`), and `handle_screenshot` uses it (`src/app/screenshot.rs:47-55`).

### Render state and readback

- [x] Native `render_to_view` and legacy `render` now call shared `prepare_scene` for option normalization, target setup, layout, cache/instance collection, GPU instance upload, and empty-scene state (`crates/render-3d/src/lib.rs:467-590,1430-1500`; `crates/render-3d/src/renderer3d/render.rs:18-47`). This removes duplicated scene preparation while retaining separate output targets.
- [x] Empty 3D scenes continue through shared pass encoding. Native output clears its target and object-ID attachment, and `prepare_scene` clears picking/selection (`crates/render-3d/src/renderer3d/render.rs:46-60`; `crates/render-3d/src/lib.rs:561-570,878-1013`). The App also clears its selected IDs and sticky hover, skips CPU OCIO work, and avoids OIDN denoise for an empty scene (`src/app/treemap_view.rs:1157-1201`). Path-tracing and legacy-output parity remains a runtime and visual verification gate.
- [x] Picking now refreshes size/directory metadata when a path reuses an ID and rejects inactive IDs in forward/reverse lookups (`crates/render-3d/src/picking.rs:66-93,239-291`). Instance collection resets the active range (`crates/render-3d/src/renderer3d/instance_collect.rs:48-52`); a scene/cache rebuild remaps selected IDs by canonical path to active instances and clears hover when an ID changes path (`crates/render-3d/src/lib.rs:503-555`).
- [x] PT rendering refreshes the Object ID texture before an outline or selection-only recomposite, so the overlay uses current instances (`crates/render-3d/src/renderer3d/render.rs:112-138`).
- [x] Shift-drag marquee keeps its selection baseline as paths, remaps that baseline to active instance IDs on each preview/commit, and clears the transient state when a new scan replaces the scene (`src/app/treemap_view.rs:412-458,729-767`; `src/app/state.rs:386-389`; `src/app/scan_orchestration.rs:91-111`). This avoids carrying stale numeric IDs across an LOD/cache rebuild.
- [x] Central GPU readback reserves host output storage with `try_reserve_exact`, returns `HostAllocation` on failure, and handles mapped-range access as a typed error (`crates/render-core/src/lib.rs:725-805,810-839`). No caller-specific allocation workaround was added.
- [x] Complete the render-agent source review of native/legacy raster, PT, xray, selection, and empty-frame codepaths. The agent reported `rustfmt --check` and `git diff --check` passed for its changes. No runtime render comparison has been performed.

### Image-sequence encoding

- [x] `FrameConversion::tonemap` now receives an output `PixelFormat` and keeps alpha out of the color tonemap curve (`crates/media-encoder/src/frame.rs:381-444,565-590`). Sequence export retains floating precision for U16 output until common PNG/TIFF quantization (`crates/media-encoder/src/dialogs/encode/encode.rs:1342-1355,1574,1696,1854-1867`).
- [x] TIFF uses the selected None/LZW/ZIP/PackBits setting in the `tiff` encoder; TGA enables or disables RLE according to settings (`crates/media-encoder/src/dialogs/encode/encode.rs:1657-1715,1718-1762`; `crates/media-encoder/Cargo.toml:30`). `Cargo.lock` contains the added TIFF dependency resolution.
- [x] The duplicate `TiffSequenceSettings.bit_depth` was removed. `SequenceSettings.bit_depth` remains the writer's authority (`crates/media-encoder/src/dialogs/encode/encode.rs:858-865,890-902,1695-1712`). A workspace Rust-source search found no remaining `TiffBitDepth` or `tiff.bit_depth` consumer; backward compatibility is outside this task's requirements.
- [ ] End-to-end U16 precision remains limited for the App image-sequence source because `capture_viewport` returns RGBA8 before `Frame::rgba8` (`src/app/image_sequence.rs:257-267`). The writer preserves higher precision when its `Comp::get_frame` input is F16/F32, but it cannot recover precision lost by that producer. Trace whether a higher precision capture path is intended before claiming complete U16 export support.
- [ ] Review format/channel combinations and output decoding at a justified future verification gate. The current pass did not run exports.

## Risk and review gates

GitNexus upstream impact was reported before the corresponding edits: `parse_args` HIGH (main direct caller, four main flows); `capture_viewport` HIGH (screenshot and image sequence, three flows); `decode_flat_cache` CRITICAL (five impacted), and the later `serialize_cache`/`from_canonical_path` cleanup was also reported as CRITICAL; `update_exclusions` CRITICAL (nine impacted); `start_scan` HIGH (six impacted). Reported render impact includes `collect_cubes` CRITICAL (two callers, six flows) and `path_for_id` CRITICAL (two callers, five flows); the media agent reported LOW impact for its symbols. A full GitNexus reindex after edits reported 5,720 nodes and 13,170 relationships. `detect-changes --scope all` exited 0 with CRITICAL summary (102 changed symbols, 58 affected, 27 graph files). The report includes `xtask` Commands as touched although `git status` has no xtask diff; treat that as a graph-tool false positive, not a source change. Recheck graph freshness before further edits, as required by [AGENTS.md](AGENTS.md#gitnexus--code-intelligence).

## Remaining review and implementation work

- [ ] Review final native/legacy 3D parity, including xray, PT, empty-scene clear, and selection after LOD/cache rebuild (`crates/render-3d/src/lib.rs:467-590,1430-1510`; `crates/render-3d/src/renderer3d/render.rs:18-170`). The code-level changes above are implemented, but visual output has not been checked.
- [ ] Decide whether the App image-sequence producer needs a higher precision capture contract. Its current `Frame::rgba8` boundary prevents end-to-end U16 precision (`src/app/image_sequence.rs:257-271`). Preserve screenshot's RGBA8 PNG path.
- [ ] Review all image format/channel/depth combinations and error propagation after the new TIFF dependency and tonemap signature (`crates/media-encoder/src/dialogs/encode/encode.rs:1342-1355,1530-1762,1854-1937`; `crates/media-encoder/src/frame.rs:381-444`).
- [ ] Continue the remaining workspace audit, feature-gated and platform branches, TODO/FIXME inventory, and intentional dead-code review in [plan11.md](plan11.md#coverage-boundary-and-remaining-audit). Do not remove untraced features.
- [ ] Review the final diff and source-line references, then report the outcome after the authorized main-branch push.

## Verification record

- Source inspection: scan/cache/exclusions, NTFS fallback, CLI/main/screenshot, render, and media diffs inspected. The render and media agents completed source review, and the final incremental compilation check completed. Root's full GitNexus reindex and change gate completed; source diff review remains before the authorized main-branch push.
- Structural checks: the media agent reported `cargo metadata --locked --no-deps` and `git diff --check` exit 0 for its changes. The root's full GitNexus reindex and `detect-changes --scope all` also completed as detailed above.
- Compilation checks: an initial `cargo check -p squarebob-rs --locked` exited 0 in 4m38s before the final marquee and scan-cleanup edits. After removing a transient unfulfilled `#[expect(dead_code)]` attribute on the retained flat-cache wire field, the final incremental check exited 0 in 6.11s. Four `glam` deprecation warnings remain: `crates/render-shared/src/lib.rs:1304,1325` and `src/cli_test.rs:162,163`. The three earlier unused-site warnings were resolved by traced cleanup, not by suppressing compiler diagnostics.
- Release build/test: none for this bug-hunt worktree. plan11 records an earlier user-authorized successful release build before these edits; it does not validate them.
- Formatting and diff checks: targeted `rustfmt --check` and `git diff --check` passed. `cargo fmt --all --check` exits 1 only for pre-existing formatting in unchanged `crates/pt-denoise-oidn/src/lib.rs:583,800,830,857,903`; that file was not edited here.
- Runtime/UI checks: none in this pass.
- Commit/push: authorized for main; final commit SHA and remote confirmation are reported in chat after publication.
