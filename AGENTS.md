# AGENTS.md — dirstat-rs agent context

Audience: humans and tooling resuming after context compaction.

## Crate layout

- **Binary** (`src/`): egui shell, scanners, filters, renderer glue, CLI.
- **dirstat-core**: canonical tree model (`DirEntry`, sizes, rects, serde, LoD hints).
- **treemap**: 2D layout + CPU/GPU (wgpu) treemap rasterization.
- **render-core**, **render-shared**, **render-3d**: GPU context and 3D path tracing stacks.
- **pt-***, **bvh-gpu**: path tracer pipelines (megakernel, wavefront, materials, etc.).

## ASCII dataflow — startup

```
main.rs
  ├─ parse_args() → CliOptions (optional overrides)
  ├─ env_logger filters (dirstat_rs, optional pt/wf/pg)
  └─ eframe::run_native → app::App::new(cc, cli)
           ├─ load PersistState from egui storage
           ├─ reconcile scan_path existence
           ├─ apply CLI mode/backend/render knobs
           ├─ register wgpu uncaptured-error hook (when eframe exposes device)
           └─ GpuContext::from_eframe(...) if POLYGON_MODE_LINE supported
```

## ASCII codepath — scan

```
start_scan(app/scan_orchestration.rs)         [Stage B.3 extraction]
  ├─ exclusions::load(scan_path)
  ├─ cache hit? → restore tree, rebuild display, optionally skip thread scan
  └─ miss → crossbeam channel + background thread:

      ScannerMode::Standard ──► scanner::scan_bg → jwalk walk → DirEntry tree → Done

      ScannerMode::Ntfs (Windows)
          ├─ is_ntfs_available? no  → scanner::scan_bg (jwalk)
          └─ yes → scanner_ntfs::scan_ntfs_bg
                     ├─ OK → Done
                     └─ Err → ScanMsg::NtfsFallback(reason)
                               → scanner::scan_dir_public (same thread)
                               → Done or Error
```

UI note (post-sprint-2): handling `NtfsFallback` does NOT mutate
`scanner_mode`. The user's NTFS preference is preserved across
fallback. Existing UI feedback flows through `progress.error` and
`progress.scan_engine_label`. (Fix in commit `ce6ae3c`.)

## ASCII codepath — display / render tick

```
update() / run_frame() (app/render_loop.rs)   [Stage B.3 extraction]
  └─ poll_scan() (app/scan_orchestration.rs) → rebuild display tree when Done
  └─ handle_events() → Navigate/Select/LayoutDirty/RenderTick3D
  └─ rebuild_display_tree() when filters/size/exclusions change

ui_treemap_pane(app/treemap_view.rs)        [main display dispatch]
  └─ if eframe device + gpu_context: zero-copy via register_native_texture
  │     ├─ Mode3D → render_3d_callback (3D PBR + PT, denoiser when it lands)
  │     └─ Mode2D + Gpu → render_2d_callback                  [Stage D.1]
  └─ else CPU-readback fallback via render_treemap(app/mod.rs)
       ├─ ensure GpuContext (failures logged via log::error! per Stage C.2)
       ├─ Mode2D + Cpu → renderer::cpu::render
       ├─ Mode2D + Gpu fallback → treemap::GpuRenderer2D::render (readback RGBA)
       └─ Mode3D fallback → Renderer3D::render (readback RGBA)
            └─ ColorImage → ctx.load_texture (only on the fallback path now)
```

## Single sources of truth (SSOT)

| Concern              | Canonical location                                      |
|----------------------|---------------------------------------------------------|
| Tree node shape      | `crates/dirstat-core/src/lib.rs` (`DirEntry`)           |
| 3D options / camera  | `render_shared::Render3DOptions` (+ `App` persisted state slices) |
| Scan progress        | `app::state::ScanProgress`                              |
| Path-derived cache keys | `src/path_key.rs` (`scan_path_id_hex`)               |
| Ignore rules on disk | `src/exclusions.rs` + `.dirstat-exclusions.json`        |

## Open engineering items (tracked in code, see CHANGELOG.md / TODO4.md)

- **PT denoiser** (Stage D.2): deferred per user. The
  `register_native_texture` path used by both 3D and 2D-GPU display
  is the natural integration point — denoiser will produce an RGBA
  wgpu texture on eframe's device, register with egui via
  `render_texture_id`, no new display infrastructure needed. Just
  preserve G-buffer extension points (normal/depth/albedo) inside
  the PT pipeline so the denoiser can sample them.
- **Two PT backends policy** (Stage C.6): documented as intentional
  orchestrator pattern (`pt-megakernel` depends on `pt-wavefront` via
  single import in `compute.rs:16`). Final canonical-vs-fast-path
  policy text awaits user decision.
- **Runtime-verification items** (user, not code): Stage 0.1 slider
  toggle vs `instance_rebuild_count()`, Stage A.1 visual diff,
  Stage D.3 BVH refit fast-path trace, Stage D.4 allocator benchmark,
  Stage E.3 `npx gitnexus analyze --embeddings`.

(Items resolved earlier this session: zero-copy 3D + 2D-GPU via
`register_native_texture`, the only two `TODO` markers replaced with
explanatory comments, all four blanket `#![allow(dead_code)]` removed
because nothing was actually dead — see CHANGELOG.md.)

## Maintenance commands

```text
cargo build --workspace --all-targets
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace                     # 24 unit tests, all passing
```

Local toolchain: conda-forge `gcc=13` (downgraded from 15.1 on
2026-05-10 to avoid the C23-removed `ATOMIC_VAR_INIT` issue in
auto-allocator's build script). Plain `cargo build` works without
PATH workarounds.

Last bughunt pass (sprint-2): clippy clean — 0 warnings with
`-D warnings`. CI workflow at `.github/workflows/ci.yml` enforces
this on Linux + Windows.

<!-- gitnexus:start -->
# GitNexus — Code Intelligence

This project is indexed by GitNexus as **dirstat-rs** (2651 symbols, 6575 relationships, 230 execution flows). Use the GitNexus MCP tools to understand code, assess impact, and navigate safely.

> If any GitNexus tool warns the index is stale, run `npx gitnexus analyze` in terminal first.

## Always Do

- **MUST run impact analysis before editing any symbol.** Before modifying a function, class, or method, run `gitnexus_impact({target: "symbolName", direction: "upstream"})` and report the blast radius (direct callers, affected processes, risk level) to the user.
- **MUST run `gitnexus_detect_changes()` before committing** to verify your changes only affect expected symbols and execution flows.
- **MUST warn the user** if impact analysis returns HIGH or CRITICAL risk before proceeding with edits.
- When exploring unfamiliar code, use `gitnexus_query({query: "concept"})` to find execution flows instead of grepping. It returns process-grouped results ranked by relevance.
- When you need full context on a specific symbol — callers, callees, which execution flows it participates in — use `gitnexus_context({name: "symbolName"})`.

## Never Do

- NEVER edit a function, class, or method without first running `gitnexus_impact` on it.
- NEVER ignore HIGH or CRITICAL risk warnings from impact analysis.
- NEVER rename symbols with find-and-replace — use `gitnexus_rename` which understands the call graph.
- NEVER commit changes without running `gitnexus_detect_changes()` to check affected scope.

## Resources

| Resource | Use for |
|----------|---------|
| `gitnexus://repo/dirstat-rs/context` | Codebase overview, check index freshness |
| `gitnexus://repo/dirstat-rs/clusters` | All functional areas |
| `gitnexus://repo/dirstat-rs/processes` | All execution flows |
| `gitnexus://repo/dirstat-rs/process/{name}` | Step-by-step execution trace |

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
