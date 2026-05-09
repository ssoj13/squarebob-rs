# Coding Conventions

**Analysis Date:** 2026-05-09

This document captures the Rust style and idioms observed across the
`dirstat-rs` workspace (binary at `src/` plus 10 crates under `crates/`).
Use it as a guide when adding code so new modules blend in with the
existing GPU/CPU codepaths.

## Edition and Toolchain

- **Edition:** 2021 (root `Cargo.toml` line 4, all member crates inherit).
- **MSRV / `rust-version`:** 1.95 (root `Cargo.toml` line 5).
- **Workspace layout:** single root binary (`dirstat-rs`) plus 10 library
  crates declared in `Cargo.toml` `[workspace]` block (lines 7-19).
- **Lint profile:** `cargo clippy --workspace --all-targets` is the
  expected baseline (see `AGENTS.md` "Maintenance commands"). Last bughunt
  pass cleared workspace clippy warnings (per `AGENTS.md` final note).

## Naming Patterns

**Files / modules** (snake_case, often role-suffixed):
- `crates/render-3d/src/env_map.rs`, `crates/render-3d/src/geometry.rs`,
  `crates/render-3d/src/picking.rs`, `crates/render-3d/src/pipelines.rs`,
  `crates/render-3d/src/targets.rs`.
- WGSL siblings live under `wavefront/`, `restir/`, `pathguide/`,
  `adaptive/` subfolders, e.g.
  `crates/pt-megakernel/src/restir/spatial.wgsl`.
- App layer organizes by view in `src/app/`: `mod.rs`, `state.rs`,
  `filters.rs`, `treemap_view.rs`, `tree_panel.rs`, `toolbar.rs`,
  `status_bar.rs`, plus `settings/` submodule.

**Functions** (snake_case, terse but meaningful):
- `sort_children_by_size_desc`, `merge_tree_by_size_range`,
  `count_files_outside_range`, `start_scan`, `render_treemap`,
  `from_eframe`, `block_on`. See `crates/dirstat-core/src/lib.rs:55-108`
  and `src/app/filters.rs:128, 281, 335`.

**Types** (UpperCamelCase, descriptive):
- Domain types: `DirEntry`, `LodKind`, `LodExpandInfo`
  (`crates/dirstat-core/src/lib.rs`).
- GPU types: `GpuBvhBuilder`, `GpuBvhConfig`, `MortonPrimitive`,
  `BuildParams`, `RadixParams` (`crates/bvh-gpu/src/bvh_gpu/mod.rs`).
- Render types: `Renderer3D`, `OrbitCamera`, `Render3DOptions`,
  `CameraUniform`, `LightRigUniform` (`crates/render-shared/src/lib.rs`).
- Path tracer: `PathTraceCompute`, `PtCameraUniform`, `WavefrontPipeline`
  (`crates/pt-megakernel/src/lib.rs`, `crates/pt-wavefront/src/lib.rs`).

**Constants** (`SCREAMING_SNAKE_CASE`):
- WGSL string constants: `MORTON_WGSL`, `RADIX_WGSL`, `LBVH_WGSL`,
  `AABB_WGSL` (`crates/bvh-gpu/src/bvh_gpu/mod.rs:131-134`); same pattern
  in `crates/pt-megakernel/src/compute.rs:58-61` and
  `crates/pt-wavefront/src/wavefront/pipeline.rs:22-26`.
- Layout/palette: `DEFAULT_PALETTE`, `PALETTE_BRIGHTNESS`
  (`crates/treemap/src/lib.rs:49-71`).
- Scene constants: `DEFAULT_SCENE_LAYOUT_SIZE`
  (`crates/render-3d/src/lib.rs:39`).

**Enums** (UpperCamelCase variants, often `Default` derived):
- `LodKind::BelowMin / AboveMax`, `LayoutStyle::KDirStat / SequoiaView`,
  `MaterialSource::{None, Extension, Path, Size, Age, Depth, Random}`
  (`crates/pt-mats/src/lib.rs:14-23`).

## Code Style

**Formatting:** standard `rustfmt` defaults; no `rustfmt.toml` is
checked in. Imports use grouped `use` blocks ordered std → external →
local (see `crates/render-3d/src/lib.rs:17-37`,
`crates/bvh-gpu/src/bvh_gpu/mod.rs:9-15`).

**Linting:** `cargo clippy --workspace --all-targets` run regularly.
Several PT pipeline modules carry crate-local
`#![allow(dead_code)]` because they expose generated/experimental WGSL
bindings ahead of feature wiring:
- `crates/pt-megakernel/src/adaptive/pipeline.rs:2`
- `crates/pt-megakernel/src/pathguide/pipeline.rs:2`
- `crates/pt-megakernel/src/restir/pipeline.rs:2`
- `crates/pt-wavefront/src/wavefront/pipeline.rs:2`
Per `AGENTS.md`, prune only after proving unused at link time across
features. Field-level `#[allow(dead_code)]` is also used (e.g.
`crates/bvh-gpu/src/bvh_gpu/mod.rs:71, 74, 91`).

## Module Organization

**Library crate root pattern** (`crates/<name>/src/lib.rs` is a thin
facade that declares modules and re-exports the public API):
- `crates/pt-core/src/lib.rs` declares `build`, `bvh`, `gpu_data` and
  re-exports `build_instance_bvh`, `BvhNode`, `GpuAabb`, `GpuMaterial`,
  `Instance`, `build_gpu_data_from_nodes`, `build_instance_gpu_data`.
- `crates/pt-megakernel/src/lib.rs` declares `adaptive`, `compute`,
  `pathguide`, `restir` and re-exports `PathTraceCompute`,
  `PtCameraUniform`.
- `crates/pt-wavefront/src/lib.rs` re-exports `WavefrontConfig`,
  `WavefrontPipeline`, `WfDims`, `WfHit`, `WfRay`.
- `crates/bvh-gpu/src/lib.rs` re-exports `GpuBvhBuilder`, `GpuBvhConfig`.

**Submodule grouping** mirrors GPU pipeline domains. Example:
`crates/pt-megakernel/src/` has `adaptive/`, `restir/`, `pathguide/`,
`wavefront/` directories each carrying a `pipeline.rs` (Rust) plus
`*.wgsl` shaders co-located in the same folder.

**Render-3D split** (`crates/render-3d/src/lib.rs:10-15`):
```rust
pub mod env_map;
pub mod geometry;
pub mod picking;
pub mod pipelines;
mod pt;
pub mod targets;
```
Public modules expose reusable building blocks; `pt` is private glue.

**Visibility:** prefer `pub(super)` for module-private helpers callable
by sibling modules (see all helpers in `src/app/filters.rs:19, 52, 69,
102, 128, 281, 335, 343, 414, 464, 517`). `pub` is reserved for the
crate's stable surface.

## Error Handling

**Default style:** propagate with `?` and rely on `anyhow::Result`
contexts. `anyhow = "1"` is a workspace dep (root `Cargo.toml:33`).

**Where used:**
- `crates/render-shared/src/lib.rs` (env-map / IO paths).
- `src/app/filters.rs` and `crates/bvh-gpu/src/bvh_gpu/mod.rs` (one
  `anyhow::` reference each — most builders return plain values and
  surface failure via logs).

**Optional / fallible patterns:**
- Construction returns `Option<Self>` when GPU adapter acquisition can
  fail: `GpuContext::new() -> Option<Self>`
  (`crates/render-core/src/lib.rs:88-99`). The caller falls back to
  `from_eframe(...)` when integrated with `eframe`.
- Many GPU helpers return `Result<T, wgpu::Error>` indirectly by relying
  on the registered `uncaptured-error` hook described in `AGENTS.md`
  ("register wgpu uncaptured-error hook").

**Logging instead of returning errors:** background workers (scanner,
BVH builder) log `warn!`/`error!` and signal completion through
`crossbeam_channel` messages — see `ScanMsg::NtfsFallback` flow in
`AGENTS.md` "ASCII codepath — scan".

## Logging

**Framework:** `log` facade (`log = "0.4"`) plus `env_logger = "0.11"`
initialized in `src/main.rs`. The `AGENTS.md` startup diagram lists
`env_logger filters (dirstat_rs, optional pt/wf/pg)`.

**Idiomatic import:** group all five macros explicitly when used:
```rust
use log::{debug, info, trace, warn};
```
- `crates/render-3d/src/lib.rs:18`
- `crates/bvh-gpu/src/bvh_gpu/mod.rs:9`
- `crates/treemap/src/lib.rs:4` uses `use log::trace;` only.

**Log target conventions:** GPU subsystems use namespaced targets like
`pt`, `wf`, `pg` (per `AGENTS.md` env_logger hint) so users can isolate
path-tracer / wavefront / path-guide noise.

## Async and GPU Blocking

The project does **not** use Tokio. All `async` boundaries from `wgpu`
(adapter request, buffer mapping) are bridged synchronously with
`pollster`:
- `pollster = "0.4"` declared in `Cargo.toml:46`.
- `pollster::block_on(instance.request_adapter(...))` —
  `crates/render-core/src/lib.rs:94-99`.
- Treemap GPU readback also blocks via `pollster::block_on` —
  `src/app/treemap_view.rs`.

Background CPU work uses native threads + `crossbeam_channel`
(see scan dataflow in `AGENTS.md`); CPU-parallel passes use
`rayon::prelude::*` (`crates/treemap/src/lib.rs:5`).

## GPU Type Conventions (`bytemuck` Pod/Zeroable)

Every uniform / SSBO struct uploaded to wgpu is `#[repr(C)]` with derived
`Pod, Zeroable`. Two equivalent import styles coexist:

**Direct import** (preferred in compute crates):
```rust
use bytemuck::{Pod, Zeroable};

#[repr(C)]
#[derive(Debug, Clone, Copy, Pod, Zeroable)]
pub struct MortonPrimitive { pub code: u32, pub index: u32 }
```
See `crates/bvh-gpu/src/bvh_gpu/mod.rs:12, 18-49` and
`crates/pt-core/src/bvh.rs:67, 78, 99, 171, 182`.

**Fully-qualified derive** (used inline next to other derives):
```rust
#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
struct MatGlobalUniform { materialize_mix: f32, _pad: [f32; 3] }
```
See `crates/render-3d/src/lib.rs:82-87`.

**Padding rules:** explicit `_pad: [u32; N]` / `[f32; N]` fields keep
16-byte alignment for uniforms (e.g. `BuildParams._pad: [u32; 2]`,
`MatGlobalUniform._pad: [f32; 3]`). Always pad rather than trust the
compiler to insert it for `Pod`.

## Math Types (`glam`)

`glam = "0.32"` is the only math vector library. Common imports:
```rust
use glam::{Mat4, Vec3, Vec4};
```
- `crates/render-3d/src/lib.rs:17` (also UVec/IVec elsewhere).
- 11 files use glam directly per workspace-wide grep, including
  `crates/render-shared/src/lib.rs`, `crates/render-3d/src/geometry.rs`,
  `crates/pt-core/src/bvh.rs`, `crates/pt-megakernel/src/compute.rs`,
  `src/app/mod.rs`, `src/app/treemap_view.rs`.

Use `Mat4::IDENTITY`, `Vec3::ZERO`, `Vec4::splat(...)` constants for
default fields (see `PtState::default()` in
`crates/render-3d/src/lib.rs:58-77`).

## Shader (WGSL) Organization

WGSL files live next to their Rust pipeline owner and are embedded at
compile time with `include_str!`:

**Co-location pattern**:
- `crates/bvh-gpu/src/bvh_gpu/{morton,radix_sort,lbvh_build,aabb_compute}.wgsl`
  + `mod.rs` (lines 131-134 declare `const *_WGSL: &str = include_str!`).
- `crates/pt-megakernel/src/{bvh_traverse,blit,pick}.wgsl` plus
  subdirectories `wavefront/gbuffer.wgsl`, `pathguide/{sample,update}.wgsl`,
  `restir/{spatial,temporal,initial,shade}.wgsl`,
  `adaptive/{variance,allocate}.wgsl`.
- `crates/pt-wavefront/src/wavefront/{raygen,intersect,shade,finalize,count_swap}.wgsl`
  loaded via `crates/pt-wavefront/src/wavefront/pipeline.rs:22-26`.

**Render-3D exception:** `crates/render-3d/shaders/` (top-level under
the crate root, not `src/`) holds the rasterization shaders
`cube_pbr.wgsl`, `cube_object_id.wgsl`, `outline.wgsl`, `skybox.wgsl`.

**Idiom for loading:**
```rust
const RAYGEN_WGSL: &str = include_str!("raygen.wgsl");
// ...
let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
    label: Some("bvh_morton_shader"),
    source: wgpu::ShaderSource::Wgsl(RAYGEN_WGSL.into()),
});
```
Always pass `label: Some("snake_case_pipeline_name")` for wgpu
diagnostics — present on every shader module, bind-group layout,
pipeline layout, buffer, and bind group across the codebase.

## Common Derives

Domain / serializable types:
```rust
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
```
This shape is the workhorse for option enums and small value types:
`crates/render-shared/src/lib.rs:8, 29, 51, 80, 112, 138, 167, 186,
209, 374, 446` and `crates/pt-mats/src/lib.rs:13, 55, 88`.

Tree nodes use `Cell<[f32; 4]>` for interior mutability so layout passes
can write rects without `&mut DirEntry` (avoids cloning the whole tree).
See doc comment in `crates/dirstat-core/src/lib.rs:23-26, 49-51`.

## Comment Style

**Module-level doc comments** (`//!`) on every crate root and major
module — concise summary plus a bullet list of submodule purposes.
Examples:
- `crates/render-3d/src/lib.rs:1-8` (modules summary).
- `crates/dirstat-core/src/lib.rs:6, 13, 22-26` (per-type docs).
- `crates/treemap/src/lib.rs:1-2`.
- `crates/pt-mats/src/lib.rs:1`.

**Item-level `///` rustdoc** appears on most public functions and
fields; brief one-liners are preferred over long paragraphs. See
`DirEntry::sort_children_by_size_desc` and field docs on
`DirEntry.own_size`, `rect`, `lod_expand`
(`crates/dirstat-core/src/lib.rs:30-46, 54-55`).

**ASCII art diagrams** are used in `AGENTS.md` to record control-flow
(scan / startup / render-tick). Match this style when extending
`AGENTS.md`.

**Inline `//` comments** explain GPU layout binding indices and padding
intent (e.g. `// aabbs`, `// morton out`, `// bounds`, `// params` in
`crates/bvh-gpu/src/bvh_gpu/mod.rs:145-148`).

**Single Source of Truth note:** `AGENTS.md` maintains an SSOT table
(tree node shape → `crates/dirstat-core/src/lib.rs`, 3D options →
`render_shared::Render3DOptions`, scan progress → `app::state::ScanProgress`,
cache keys → `src/path_key.rs`, ignore rules → `src/exclusions.rs`).
Honor it when adding fields.

## Function Design

- **Builders return owned values**, often via `Default::default()` then
  field overrides (see `GpuBvhConfig::default()` and
  `PtState::default()`).
- **Recursive tree walks** take `&DirEntry` and return owned `DirEntry`
  rather than mutating in place — see `filter_tree`, `filter_by_mask`,
  `filter_by_extension`, `filter_excluded_recursive`
  (`src/app/filters.rs`).
- **GPU constructors** (`Renderer3D::new`, `GpuBvhBuilder::new`) take
  `&wgpu::Device` and create all pipelines / bind-group layouts up
  front; runtime methods then accept `&mut self` plus `&wgpu::Queue`.
- Prefer `Arc<wgpu::Device>` / `Arc<wgpu::Queue>` for sharing across
  subsystems — `GpuContext` (`crates/render-core/src/lib.rs:76-86`).

## Module Re-export Pattern

Public API is exposed at the crate root via explicit `pub use`. Avoid
glob re-exports. See `crates/pt-core/src/lib.rs:7-9`,
`crates/pt-megakernel/src/lib.rs:8`, `crates/bvh-gpu/src/lib.rs:5`.

## Platform-Specific Code

Per-OS deps live in target-conditional `[target.'cfg(...)']` blocks
(`Cargo.toml:61-72`):
- `cfg(unix)` → `libc`.
- `cfg(windows)` → narrow `windows` features (`Win32_Storage_FileSystem`,
  `Win32_System_IO`, `Win32_System_Ioctl`, `Win32_Foundation`,
  `Win32_Security`).
NTFS-specific scanner code lives in `src/scanner_ntfs.rs` and is
gated/dispatched at runtime by `is_ntfs_available?` checks
(`AGENTS.md` scan codepath).

## Build / Lint Commands

```bash
cargo build -p dirstat-rs --message-format short
cargo check --workspace
cargo clippy --workspace --all-targets
cargo test
```

---

*Convention analysis: 2026-05-09*
