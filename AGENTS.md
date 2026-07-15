# AGENTS.md

## Bug-Hunt Operating Notes

This repository is a Rust workspace for a desktop disk-usage visualizer with CPU and GPU rendering paths. The current bug-hunt pass was run on 2026-05-16 from the repository root on branch `main`.

Primary constraints for future agents:

- Keep scan data ownership on the main UI side. `DirEntry.rect` uses `Cell`, so the owned tree is intentionally passed through channels and caches rather than shared by `Arc`.
- Treat `render_core::gpu::GpuContext::new` as the single source of truth for wgpu device setup. `main.rs` passes that same instance/device/queue to eframe and app renderers.
- Prefer central readback helpers over ad hoc `map_async` blocks. `wgpu::BufferSlice::map_async` returns a callback `Result`; never unwrap channel receive or map errors in UI render paths.
- Keep 2D/3D zero-copy paths on the eframe-backed device. CPU readback paths are legacy/fallback paths and must return recoverable errors instead of panicking.
- Do not remove `#[allow(dead_code)]` items without tracing intended feature toggles and platform/API parity stubs.

## Inventory Snapshot

- Root package: `squarebob-rs` binary `squarebob`.
- Workspace members: `squarebob-core`, `pt-core`, `bvh-gpu`, `pt-megakernel`, `pt-wavefront`, `pt-mats`, `render-core`, `render-shared`, `render-3d`, `media-encoder`, `xtask`, `treemap`, `pt-denoise-oidn`, `gpu-mem`.
- Files scanned excluding `target/**` and `.git/**`: 223.
- Existing historical bug-hunt artifact: `.bughunt/plan1.md`.
- This pass created `.bughunt/plan2.md` and `BUG_HUNT_REPORT.md`.

## High-Level Dataflow

```text
CLI args
  |
  v
main.rs
  |-- parse CLI / test mode
  |-- create shared render_core::gpu::GpuContext
  |-- pass WgpuSetup::Existing to eframe
  v
App::new
  |
  v
App::start_scan
  |-- load cache if available
  |-- choose scanner: jwalk or NTFS MFT on Windows
  |-- spawn background scanner
  v
ScanMsg channel
  |-- Progress -> App::poll_scan updates UI counters
  |-- Done(DirEntry) -> cache serialize + display tree rebuild
  |-- Error/NtfsFallback -> UI progress state
  v
App::ui_treemap
  |-- Mode2D CPU -> treemap::render -> egui texture
  |-- Mode2D GPU -> treemap::GpuRenderer2D -> eframe texture
  |-- Mode3D raster/PT -> render_3d::Renderer3D -> eframe texture or CPU readback
```

## GPU Readback Codepath

```text
2D legacy render
  crates/treemap/src/wgpu.rs:680 -> render_core::gpu::readback_texture
  crates/treemap/src/wgpu.rs:688 -> render_core::gpu::map_readback

3D raster legacy render
  crates/render-3d/src/lib.rs:1331 -> render_core::gpu::readback_texture
  crates/render-3d/src/lib.rs:1348 -> render_core::gpu::map_readback

PT megakernel readback render
  crates/render-3d/src/pt/megakernel/render.rs:465 -> render_core::gpu::readback_texture
  crates/render-3d/src/pt/megakernel/render.rs:481 -> render_core::gpu::map_readback

Shared failure point
  crates/render-core/src/lib.rs:227 -> BufferSlice::map_async callback
  crates/render-core/src/lib.rs:228 -> tx.send(result).unwrap()
  crates/render-core/src/lib.rs:232 -> rx.recv().unwrap().unwrap()
```

## Scan / Cache Codepath

```text
App::start_scan
  |
  |-- cache::load_cache(scan_path)
  |     |-- cache_path(scan_path)
  |     |-- bincode::deserialize_from
  |     `-- cached DirEntry tree returned to App
  |
  `-- scanner::scan_bg or scanner_ntfs::scan_ntfs_bg
        |
        |-- jwalk WalkDir / NTFS MFT enumeration
        |-- DirEntry::new_file / DirEntry::new_dir
        |-- sort_by_size
        `-- tx.send(ScanMsg::Done(tree))

App::poll_scan
  |
  |-- compute_ext_stats / compute_size_range
  |-- cache::serialize_cache
  |-- cache::write_cache_bytes on background thread
  `-- rebuild_display_tree + needs_layout
```

## Rendering Codepath

```text
App::ui_treemap
  |
  |-- callback path when wgpu_render_state and gpu_context exist
  |     |-- Mode2D + GPU: render_2d_callback
  |     |     |-- GpuRenderer2D::render_to_texture
  |     |     `-- egui_wgpu texture registration/update
  |     |
  |     `-- Mode3D: render_3d_callback
  |           |-- Renderer3D::render_to_view
  |           |-- object-id picking readback
  |           `-- egui_wgpu texture registration/update
  |
  `-- legacy path
        |-- render_treemap / Renderer3D::render
        `-- CPU pixel Vec uploaded to egui texture
```

## Current Bug-Hunt Focus Areas

Archived bug-hunt plans live in `md.old/bughunt-plan1.md` and
`md.old/bughunt-plan2.md`. The shared readback helper in
`render-core::gpu::map_readback` now returns `Vec::new()` and logs a
warning on failure instead of panicking, so the historical "panic on
map_async" entry is closed.

Remaining open work that earlier passes flagged but did not finish:

- Audit readback size arithmetic (`width * height * 4`) for `u32`
  overflow before casts to `usize` in 2D/3D/PT readback paths.
- Add `// SAFETY:` comments to remaining `unsafe` blocks across
  `crates/render-3d`, `crates/bvh-gpu`, and `crates/pt-megakernel`.
- Consider unifying megakernel readback vs no-readback init paths in
  `crates/render-3d/src/pt/megakernel/render.rs`.

<!-- gitnexus:start -->
# GitNexus — Code Intelligence

This project is indexed by GitNexus as **squarebob-rs** (4956 symbols, 14538 relationships, 300 execution flows). Use the GitNexus MCP tools to understand code, assess impact, and navigate safely.

> Index stale? Run `node .gitnexus/run.cjs analyze` from the project root — it auto-selects an available runner. No `.gitnexus/run.cjs` yet? `npx gitnexus analyze` (npm 11 crash → `npm i -g gitnexus`; #1939).

## Always Do

- **MUST run impact analysis before editing any symbol.** Before modifying a function, class, or method, run `impact({target: "symbolName", direction: "upstream"})` and report the blast radius (direct callers, affected processes, risk level) to the user.
- **MUST run `detect_changes()` before committing** to verify your changes only affect expected symbols and execution flows. For regression review, compare against the default branch: `detect_changes({scope: "compare", base_ref: "main"})`.
- **MUST warn the user** if impact analysis returns HIGH or CRITICAL risk before proceeding with edits.
- When exploring unfamiliar code, use `query({search_query: "concept"})` to find execution flows instead of grepping. It returns process-grouped results ranked by relevance.
- When you need full context on a specific symbol — callers, callees, which execution flows it participates in — use `context({name: "symbolName"})`.
- For security review, `explain({target: "fileOrSymbol"})` lists taint findings (source→sink flows; needs `analyze --pdg`).

## Never Do

- NEVER edit a function, class, or method without first running `impact` on it.
- NEVER ignore HIGH or CRITICAL risk warnings from impact analysis.
- NEVER rename symbols with find-and-replace — use `rename` which understands the call graph.
- NEVER commit changes without running `detect_changes()` to check affected scope.

## Resources

| Resource | Use for |
|----------|---------|
| `gitnexus://repo/squarebob-rs/context` | Codebase overview, check index freshness |
| `gitnexus://repo/squarebob-rs/clusters` | All functional areas |
| `gitnexus://repo/squarebob-rs/processes` | All execution flows |
| `gitnexus://repo/squarebob-rs/process/{name}` | Step-by-step execution trace |

## CLI

| Task | Read this skill file |
|------|---------------------|
| Understand architecture / "How does X work?" | `.claude/skills/gitnexus/gitnexus-exploring/SKILL.md` |
| Blast radius / "What breaks if I change X?" | `.claude/skills/gitnexus/gitnexus-impact-analysis/SKILL.md` |
| Trace bugs / "Why is X failing?" | `.claude/skills/gitnexus/gitnexus-debugging/SKILL.md` |
| Rename / extract / split / refactor | `.claude/skills/gitnexus/gitnexus-refactoring/SKILL.md` |
| Tools, resources, schema reference | `.claude/skills/gitnexus/gitnexus-guide/SKILL.md` |
| Index, status, clean, wiki CLI commands | `.claude/skills/gitnexus/gitnexus-cli/SKILL.md` |

<!-- gitnexus:end -->
