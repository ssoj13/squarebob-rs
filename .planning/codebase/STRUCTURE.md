# Repository Structure

**Analysis Date:** 2026-04-20

## Top Level

| Path | Role |
|------|------|
| `Cargo.toml` / `Cargo.lock` | Workspace manifest; binary `dirstat-rs` + member crates |
| `src/` | Main application sources (binary root) |
| `crates/` | Workspace libraries (core, rendering, path tracing, treemap) |
| `data/` | Static assets (e.g. screenshots for README) |
| `plan1.md` | Session audit / bug-hunt notes (informal) |
| `.planning/` | GSD planning outputs (this map, future `PROJECT.md`, etc.) |

## `src/` (binary)

| Path | Purpose |
|------|---------|
| `src/main.rs` | CLI, logging, `eframe` + wgpu bootstrap, `App::new` |
| `src/app/mod.rs` | Central `App` state, `run_frame`, `eframe::App`, screenshot PNG |
| `src/app/dock.rs` | Dock layout / tabs wiring |
| `src/app/treemap_view.rs` | 2D/3D treemap view, input, context menu |
| `src/app/tree_panel.rs` | Virtual file tree panel |
| `src/app/ext_panel.rs` | Extension statistics panel |
| `src/app/filters.rs` | Display-tree filtering |
| `src/app/helpers.rs` | UI helpers, formatting |
| `src/app/shell.rs` | OS shell actions |
| `src/app/state.rs` | Serializable / persisted fragments |
| `src/app/presets.rs` | Preset load/save |
| `src/app/settings/` | Settings sub-UI (appearance, scanner, renderer, view, exclusions) |
| `src/cache.rs` | Load/write/cache age/delete scan cache |
| `src/exclusions.rs` | Path exclusion persistence |
| `src/path_key.rs` | SHA-256 hex key for cache paths |
| `src/events.rs` | `EventBus`, event types |
| `src/scanner.rs` | Background scan, `jwalk` |
| `src/scanner_ntfs.rs` | Windows NTFS fast scan |
| `src/renderer.rs` | Render mode/backend dispatch to treemap / 3D |

## `crates/` (selected)

| Crate | Role |
|-------|------|
| `dirstat-core` | `DirEntry`, tree stats |
| `treemap` | Layout + 2D render (CPU/GPU wgpu) |
| `render-core` / `render-shared` / `render-3d` | 3D renderer and shared types |
| `pt-core`, `bvh-gpu`, `pt-megakernel`, `pt-wavefront`, `pt-mats` | GPU path tracing pipeline pieces |

## Naming Conventions

- **Modules** — snake_case files; `mod.rs` for directories.
- **Crates** — kebab-case directory names matching Cargo package names.
- **App** — Large `App` struct in `src/app/mod.rs` with focused methods in submodules.

---

*Structure analysis: 2026-04-20*
