# AGENTS.md

## Bug-hunt operating notes

This is the `squarebob-rs` Rust package and `squarebob` binary (`Cargo.toml:1-13`). Source paths below were checked on 2026-09-23. On 2026-09-23, the first user-authorized `python bootstrap.py b` release build failed in upstream `vfx-io` with `E0599`. After the upstream EXR fix and a local `egui_dock` `TabViewer::id` implementation (`src/app/dock.rs:105-110`), a second release build succeeded. No tests ran; see [plan11.md](plan11.md) for the build sequence and remaining warnings. Read [DIAGRAMS.md](DIAGRAMS.md) for Mermaid dataflows; plan11 records the active findings and repair checklist.

Primary constraints for future agents:

- Keep owned scan data on the UI side. `DirEntry.rect` uses `Cell`; scanner/cache workers transfer owned trees through channels.
- Use `ScanRoot`'s canonical path and native-path ID for operational identity. Its `display` string preserves user spelling for UI/history (`src/path_key.rs:6-62`). Existing cache/exclusion display checks are a confirmed defect in plan11.
- A scan generation owns a `ScanSession`, progress receiver, and separate terminal receiver. Replacement cancels and retires the prior session; `poll_scan` discards stale generation/root outcomes (`src/scanner.rs:85-153`; `src/app/scan_orchestration.rs:38-69,456-528`).
- `CacheService` owns ordered cache I/O, generation watermarks, and atomic replacement. Only complete live scans queue a cache store (`src/cache.rs:107-245,909-919`; `src/app/scan_orchestration.rs:237-250`).
- `render_core::gpu::GpuContext::new` is the wgpu device setup source. `main.rs:145-185` passes its instance/device/queue to eframe and app renderers.
- Use central `readback_texture`, `map_readback`, and `map_buffer_read` helpers. They return `Result` for layout, map, poll, and channel failures (`crates/render-core/src/lib.rs:551-747,807-836`). Output allocation remains an audit item.
- Keep native 2D/3D textures on the eframe device. CPU pixels/readback serve 2D CPU, fallback, and screenshot paths (`src/app/treemap_view.rs:20-98`; `src/app/mod.rs:608-647`).
- Trace `#[allow(dead_code)]`, TODO/FIXME, feature gates, and platform stubs before deleting them. Preserve unrelated worktree changes.

## Workspace inventory

`Cargo.toml:15-37` lists 20 members: `squarebob-core`, `pt-core`, `bvh-gpu`, `pt-megakernel`, `pt-wavefront`, `pt-mats`, `render-core`, `render-shared`, `render-3d`, `media-encoder`, `xtask`, `treemap`, `pt-denoise-oidn`, `gpu-mem`, `standard-surface`, `squarebob-widgets`, `pt-material`, `playa-ae`, `egui-colorpicker`, and `color-pipeline`. Earlier file counts and bug-hunt artifact claims were historical snapshots.

## Application dataflow

```text
CLI -> main.rs:parse_args
    -> GpuContext::new -> eframe WgpuSetup::Existing -> App::new
    -> App::start_scan
         -> ScanRoot(display, canonical path, id)
         -> CacheService::Load(generation) -> optional cache preview
         -> scanner::spawn(generation, jwalk | NTFS MFT)
              -> NTFS unavailable: standard fallback
              -> Progress + Terminal(Completed | Partial | Cancelled | Failed)
         -> App::poll_scan: reject stale generation/id; install tree
         -> complete scan: CacheService::Store -> atomic cache write
    -> display_root -> App::ui_treemap
         -> 2D CPU -> pixel buffer -> egui texture
         -> 2D GPU -> GpuRenderer2D -> native egui_wgpu texture
         -> 3D raster/PT -> Renderer3D -> native egui_wgpu texture
    -> optional screenshot -> capture_viewport -> save_png
```

Sources: `src/main.rs:19-33,145-185`; `src/app/scan_orchestration.rs:299-362,456-528`; `src/app/treemap_view.rs:20-98`; `src/app/screenshot.rs:14-59`.

## Scan and cache codepath

```text
start_scan
  |-- retire old scan; clear presentation; advance generation
  |-- ScanRoot::from_input -> canonical path + stable id
  |-- exclusions::load; CacheService::load(generation, root)
  `-- scanner::spawn(generation, root, backend)
        |-- standard jwalk OR NTFS MFT with standard fallback
        |-- scanner::finish_build: sort tree + derive stats
        |-- complete tree: serialize_cache_ref on scan worker
        `-- terminal channel delivers typed ScanOutcome
poll_scan
  |-- poll ordered cache events; gate by generation + root_id
  |-- drain progress and terminal channels; gate by generation + identity
  |-- install_tree -> rebuild_display_tree + invalidate layout/render
  `-- complete outcome -> queue CacheService::store -> atomic_file::write
```

Sources: `src/app/scan_orchestration.rs:38-69,130-250,299-528`; `src/scanner.rs:134-235`; `src/cache.rs:90-245,273-309,909-919`.

## Rendering and readback codepath

```text
App::ui_treemap
  |-- native 2D GPU: render_2d_callback -> render_to_texture -> egui_wgpu
  |-- native 3D: render_3d_callback -> render_to_view
  |      |-- raster passes + object-ID picking
  |      `-- path tracing + optional OIDN denoise
  `-- legacy: render_treemap -> CPU 2D OR GPU/3D pixel readback -> egui upload
GPU pixel readback
  -> TextureReadbackLayout::new (checked geometry)
  -> readback_texture (reusable staging buffer)
  -> map_readback -> map_buffer_read
  -> callback Result/channel -> ReadbackError or packed Vec<u8>
```

Sources: `src/app/treemap_view.rs:20-98,1001-1123,1325-1398`; `src/app/mod.rs:557-647`; `crates/render-core/src/lib.rs:551-747,807-836`. The old double-`unwrap` diagram described a previous implementation.

## Media export codepath

```text
Comp::get_frame
  |-- video: encode_sequence_from_comp
  |      -> crop/conditional tonemap
  |      -> VideoEncoder::open/push/finish
  |      -> codec conversion + MovWriter + private partial file + commit
  `-- image sequence: encode_image_sequence
         -> optional tonemap -> PNG/TIFF/TGA/EXR/JPEG writer
```

Sources: `crates/media-encoder/src/dialogs/encode/encode.rs:1173-1309,1811-1905`; `crates/media-encoder/src/dialogs/encode/video.rs:66-118`. Current TIFF/TGA compression and U16 conversion defects are in plan11.

## Current bug-hunt focus

[plan11.md](plan11.md) is the current review gate. It records canonical identity/display mismatch, NTFS fallback terminal loss, empty 3D target behavior, picking metadata, render duplication, sequence precision/compression, CLI validation, screenshot completion, and readback allocation. Historical bug-hunt references in older notes must be checked against the current worktree before use; the previously named `md.old/` artifacts are absent from the current inventory.

<!-- gitnexus:start -->
# GitNexus — Code Intelligence

GitNexus was rebuilt on 2026-09-23 (5,710 nodes reported). Check graph freshness before relying on query/impact results after further edits; reanalyze when needed. The earlier 5,550-symbol/15,854-relationship/300-flow counts are historical.

> Call `graph_status` when freshness matters. Use `reanalyze` (incremental) or `gitnexus-rs analyze` (full). `detect_changes` does **not** re-index.

## Always Do

- **Check graph freshness** with `graph_status` before trusting query/impact on a repo you have been editing.
- **MUST run impact analysis before editing any symbol.** Before modifying a function, class, or method, run `gitnexus_impact({target: "symbolName", direction: "upstream"})` and report the blast radius (direct callers, affected processes, risk level) to the user.
- **MUST run `gitnexus_detect_changes()` before committing** to verify your changes only affect expected symbols and execution flows.
- **MUST warn the user** if impact analysis returns HIGH or CRITICAL risk before proceeding with edits.
- When exploring unfamiliar code, use `gitnexus_query({query: "concept"})` to find execution flows instead of grepping. It returns process-grouped results ranked by relevance.
- When you need full context on a specific symbol — callers, callees, which execution flows it participates in — use `gitnexus_context({name: "symbolName"})`.
- **Check graph freshness** with `graph_status` before trusting query/impact results on a repo you have been editing.
- **Update the graph** with `reanalyze` (incremental) or `gitnexus-rs analyze` (full). `detect_changes` does **not** re-index.
- **Uncommitted edits** while commit is fresh: `reanalyze` with `scope: "unstaged"`, or run `gitnexus-rs watch` in a terminal.

## Never Do

- NEVER edit a function, class, or method without first running `gitnexus_impact` on it.
- NEVER ignore HIGH or CRITICAL risk warnings from impact analysis.
- NEVER rename symbols with find-and-replace — use `gitnexus_rename` which understands the call graph.
- NEVER commit changes without running `gitnexus_detect_changes()` to check affected scope.

## Resources

| Resource | Use for |
|----------|---------|
| `gitnexus://repo/squarebob-rs/context` | Codebase overview, check index freshness |
| `gitnexus://repo/squarebob-rs/clusters` | All functional areas |
| `gitnexus://repo/squarebob-rs/processes` | All execution flows |
| `gitnexus://repo/squarebob-rs/process/{name}` | Step-by-step execution trace |

## Tool discovery

The old `.claude/skills/gitnexus/` links are absent from this worktree. Discover the currently available GitNexus MCP tools or the `gitnexus-rs` CLI help before use; check graph status and reanalyze if the index is stale. The policy above applies whenever the graph is available.

<!-- gitnexus:end -->
