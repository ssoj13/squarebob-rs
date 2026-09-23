# AGENTS.md

## Bug-hunt operating notes

This is the `squarebob-rs` Rust package and `squarebob` binary (`Cargo.toml:1-13`). Source paths below were checked on 2026-09-23. On 2026-09-23, the first user-authorized `python bootstrap.py b` release build failed in upstream `vfx-io` with `E0599`. After the upstream EXR fix and a local `egui_dock` `TabViewer::id` implementation (`src/app/dock.rs:105-110`), a second release build succeeded. No tests ran; see [plan11.md](plan11.md) for that build sequence and remaining warnings. The current bug-hunt edits are tracked in [plan12.md](plan12.md), and their build/runtime status must be checked there. Read [DIAGRAMS.md](DIAGRAMS.md) for Mermaid dataflows.

Primary constraints for future agents:

- Keep owned scan data on the UI side. `DirEntry.rect` uses `Cell`; scanner/cache workers transfer owned trees through channels.
- Use `ScanRoot`'s canonical path and native-path ID for operational identity. Its `display` string preserves user spelling for UI/history (`src/path_key.rs:6-62`). Cache and exclusion validation now use the canonical identity; inspect `plan12.md` for the source review and outstanding verification.
- A scan generation owns a `ScanSession`, progress receiver, and separate terminal receiver. Replacement cancels and retires the prior session; `poll_scan` discards stale generation/root outcomes (`src/scanner.rs:85-153`; `src/app/scan_orchestration.rs:38-69,456-528`).
- `CacheService` owns ordered cache I/O, generation watermarks, and atomic replacement. Only complete live scans queue a cache store (`src/cache.rs:107-245,893-905`; `src/app/scan_orchestration.rs:237-250`). Flat cache v4 still carries a serialized display-path field for decoding, but runtime validation uses canonical root ID (`src/cache.rs:39-48,523-535,786-797`).
- `render_core::gpu::GpuContext::new` is the wgpu device setup source. `main.rs:145-185` passes its instance/device/queue to eframe and app renderers.
- Use central `readback_texture`, `map_readback`, and `map_buffer_read` helpers. They return `Result` for layout, map, poll, channel, mapped-range, and host-allocation failures (`crates/render-core/src/lib.rs:551-805,807-839`).
- Keep native 2D/3D textures on the eframe device. CPU pixels/readback serve 2D CPU, fallback, and screenshot paths (`src/app/treemap_view.rs:20-98`; `src/app/mod.rs:608-647`).
- Trace `#[allow(dead_code)]`, TODO/FIXME, feature gates, and platform stubs before deleting them. Preserve unrelated worktree changes.

## Workspace inventory

`Cargo.toml:15-37` lists 20 members: `squarebob-core`, `pt-core`, `bvh-gpu`, `pt-megakernel`, `pt-wavefront`, `pt-mats`, `render-core`, `render-shared`, `render-3d`, `media-encoder`, `xtask`, `treemap`, `pt-denoise-oidn`, `gpu-mem`, `standard-surface`, `squarebob-widgets`, `pt-material`, `playa-ae`, `egui-colorpicker`, and `color-pipeline`. Earlier file counts and bug-hunt artifact claims were historical snapshots.

## Application dataflow

```text
CLI -> cli::parse_args: Result; main exits 2 on invalid input
    -> GpuContext::new -> eframe WgpuSetup::Existing -> App::new
    -> App::start_scan
         -> ScanRoot(display, canonical path, id)
         -> exclusions::load -> valid policy or visible warning + edit guard
         -> CacheService::Load(generation) -> optional cache preview
         -> scanner::spawn(generation, jwalk | NTFS MFT)
              -> NTFS unavailable: terminal-channel warning + standard fallback
              -> Progress + Terminal(Completed | Partial | Cancelled | Failed)
         -> App::poll_scan: reject stale generation/id; install tree
         -> complete scan: CacheService::Store -> atomic cache write
    -> display_root -> App::ui_treemap
         -> 2D CPU -> pixel buffer -> egui texture
         -> 2D GPU -> GpuRenderer2D -> native egui_wgpu texture
         -> 3D raster/PT -> Renderer3D -> native egui_wgpu texture
    -> optional screenshot -> capture_viewport(Result) -> save_png(Result)
         -> success: mark taken/optional exit; failure: retry/dismiss UI
```

Sources: `src/main.rs:19-28,145-185`; `src/app/scan_orchestration.rs:299-363,456-528`; `src/app/treemap_view.rs:20-98`; `src/app/screenshot.rs:12-77`.

## Scan and cache codepath

```text
start_scan
  |-- retire old scan; clear presentation; advance generation
  |-- ScanRoot::from_input -> canonical path + stable id
  |-- exclusions::load; warn and block edits on unreadable policy
  |-- CacheService::load(generation, root)
  `-- scanner::spawn(generation, root, backend)
        |-- standard jwalk OR NTFS MFT with terminal-channel fallback warning
        |-- scanner::finish_build: sort tree + derive stats
        |-- complete tree: serialize_cache_ref on scan worker
        `-- terminal channel delivers typed ScanOutcome
poll_scan
  |-- poll ordered cache events; gate by generation + root_id
  |-- drain progress and terminal channels; gate by generation + identity
  |-- install_tree -> rebuild_display_tree + invalidate layout/render
  `-- complete outcome -> queue CacheService::store -> atomic_file::write
```

Sources: `src/app/scan_orchestration.rs:38-69,130-250,299-528`; `src/scanner.rs:134-235`; `src/cache.rs:90-245,273-307,893-905`.

## Rendering and readback codepath

```text
App::ui_treemap
  |-- native 2D GPU: render_2d_callback -> render_to_texture -> egui_wgpu
  |-- native 3D: render_3d_callback -> render_to_view -> prepare_scene
  |      |-- empty scene: clear color/object ID; reset picking/UI selection; skip OIDN
  |      |-- raster passes + active object-ID picking; remap selection by path on rebuild
  |      |-- shift-drag marquee: path baseline -> active instance IDs per preview/commit
  |      `-- path tracing + current Object ID for outline + optional OIDN denoise
  `-- legacy: render_treemap -> CPU 2D OR GPU/3D pixel readback -> egui upload
        3D readback also uses prepare_scene + shared pass encoding
GPU pixel readback
  -> TextureReadbackLayout::new (checked geometry)
  -> readback_texture (reusable staging buffer)
  -> map_readback -> map_buffer_read
  -> callback Result/channel -> fallible host allocation -> ReadbackError or packed Vec<u8>
```

Sources: `src/app/treemap_view.rs:20-98,1034-1217,1373-1430`; `src/app/mod.rs:569-659`; `crates/render-3d/src/lib.rs:467-590,1430-1510`; `crates/render-3d/src/renderer3d/render.rs:18-60,112-138`; `crates/render-core/src/lib.rs:551-805,810-839`. The old double-`unwrap` diagram described a previous implementation.

## Media export codepath

```text
Comp::get_frame
  |-- video: encode_sequence_from_comp
  |      -> crop/conditional tonemap
  |      -> VideoEncoder::open/push/finish
  |      -> codec conversion + MovWriter + private partial file + commit
  `-- image sequence: encode_image_sequence
         -> target-depth tonemap (U16 retains float precision)
         -> PNG/TIFF/TGA/EXR/JPEG writer
         -> TIFF selected compression; TGA selected RLE
```

Sources: `crates/media-encoder/src/dialogs/encode/encode.rs:1148-1280,1657-1762,1769-1937`; `crates/media-encoder/src/dialogs/encode/video.rs:66-118`. The TIFF/TGA compression and U16 conversion changes are reviewed in plan12; plan11 retains the original findings. App image-sequence capture currently supplies RGBA8 (`src/app/image_sequence.rs:257-267`), so its U16 output cannot regain lost source precision.

## Current bug-hunt focus

[plan12.md](plan12.md) is the current review gate. It tracks the code changes for canonical identity, NTFS fallback, empty 3D frames, picking, render preparation, sequence precision/compression, CLI validation, screenshot completion, and readback allocation. [plan11.md](plan11.md) retains the original evidence and wider audit backlog. Historical bug-hunt references in older notes must be checked against the current worktree before use; the previously named `md.old/` artifacts are absent from the current inventory.

<!-- gitnexus:start -->
# GitNexus — Code Intelligence

GitNexus was fully reindexed on 2026-09-23 after the bug-hunt edits (5,720 nodes and 13,170 relationships reported). Check graph freshness before relying on query/impact results after further edits; reanalyze when needed. Earlier graph counts are historical.

> Call `graph_status` when freshness matters. Use `reanalyze` (incremental) or `gitnexus-rs analyze` (full). `detect_changes` does **not** re-index.

## Always Do

- **Check graph freshness** with `graph_status` before trusting query/impact on a repo you have been editing.
- **MUST run impact analysis before editing any symbol.** Before modifying a function, class, or method, run `gitnexus_impact({target: "symbolName", direction: "upstream"})` and report the blast radius (direct callers, affected processes, risk level) to the user.
- **MUST run `gitnexus_detect_changes()` before committing** to verify your changes only affect expected symbols and execution flows.
- **MUST warn the user** if impact analysis returns HIGH or CRITICAL risk before proceeding with edits.
- When exploring unfamiliar code, use `gitnexus_query({query: "concept"})` to find execution flows instead of grepping. It returns process-grouped results ranked by relevance.
- When you need full context on a specific symbol — callers, callees, which execution flows it participates in — use `gitnexus_context({name: "symbolName"})`.
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
