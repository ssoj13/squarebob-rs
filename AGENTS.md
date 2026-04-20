# AGENTS.md — dirstat-rs orientation

## Purpose

Assist AI/coding agents working on **dirstat-rs**: a desktop disk-usage viewer (egui + wgpu) with 2D treemap layout and an optional 3D renderer with path tracing stacks.

## Crate roles (workspace)

| Crate | Role |
|-------|------|
| `dirstat-rs` (binary) | UI shell (`src/app/`), filesystem scan orchestration, cache I/O, CLI |
| `dirstat-core` | `DirEntry` tree model (serde, layout `rect` via `Cell`) |
| `treemap` | Treemap layout + optional wgpu 2D path |
| `render-core`, `render-shared`, `render-3d` | GPU resources, 3D scene, PBR cubes, path tracing integration |
| `pt-core`, `pt-mats`, `pt-megakernel`, `pt-wavefront`, `bvh-gpu` | PT backends and GPU BVH |

## Single sources of truth

- **Tree semantics:** `dirstat_core::DirEntry` — recursive size, counts, extension, optional mtime, children order via `sort_by_size` / `sort_children_by_size_desc`.
- **User exclusions:** `src/exclusions.rs` — JSON under project data dir, keyed by SHA-256 of scan root.
- **Disk cache:** `src/cache.rs` — bincode, SHA-256 filename, version `CACHE_VERSION`.

## ASCII dataflow (human)

```
CLI + persistence
       |
       v
+------------------+
|   app::App       |
|  state + egui    |
+--------+---------+
         |
   +-----+-----+
   |           |
   v           v
cache load   scan thread (jwalk or NTFS)
   |           |
   +-----+-----+
         |
         v
   DirEntry (tree)
         |
         v
   filters -> display_tree_cache
         |
         v
   treemap layout + viewport
         |
         +------+--------+
         v      v        v
    CPU 2D  GPU 2D   Renderer3D (+ PT)
         |      |        |
         v      v        v
   egui::Texture (RGBA upload)
```

## ASCII codepath (scan → screen)

```
main.rs::parse_args
    -> eframe::run_native(App::new)
        -> App: channel receiver for ScanMsg
            -> ScanMsg::Progress | Done | Error | NtfsFallback(win)
        -> rebuild_display_tree()
            -> filters / exclusions / size / mask / extension
            -> DirEntry::sort_children_by_size_desc (display order)
        -> treemap_view / tree_panel: hit-test + navigation
        -> paint: renderer::* or Renderer3D::render -> ColorImage -> texture
```

## Conventions for edits

- Prefer extending **existing** `impl DirEntry` / filter functions with parameters rather than new one-off helpers when behavior overlaps.
- GPU / PT crates intentionally carry `dead_code` during iteration; do not mass-delete without proving GPU init paths.
- Never remove features without explicit product consent; refactors should preserve CLI and UI behavior.

## Related docs

- `plan1.md` — latest audit report and backlog (bug hunt).
- `DIAGRAMS.md` — Mermaid diagrams (same flows).
