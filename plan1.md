# Bug hunt report — plan1.md

Date: 2026-05-07  
Scope: workspace `dirstat-rs` (binary + all workspace members).

## Verification run

| Command | Result |
|---------|--------|
| `cargo check --workspace` | OK |
| `cargo clippy --workspace --all-targets` | OK (warnings addressed in-session) |
| `cargo test -p render-shared` | 2 tests OK |

---

## Implemented fixes (this pass)

Aligned with Rust API ergonomics / clippy hygiene; behavior unchanged except clearer idioms.

| File | Lines (approx.) | Change |
|------|-----------------|--------|
| `crates/treemap/src/wgpu.rs` | ~468 | `map_or(false, \|b\| ...)` → `is_some_and(\|b\| ...)` (clippy::unnecessary_map_or) |
| `crates/render-3d/src/lib.rs` | ~644 | Removed unused `_buf_size` binding (manual slice size lint) |
| `src/app/settings/renderer.rs` | ~819, ~893 | `.min(..).max(1)` → `.clamp(1, MAX_PT_MAT_CUBE_COUNT)` (clippy::manual_clamp) |
| `crates/render-shared/src/lib.rs` | tests ~897–914 | Struct literal + `..Default::default()` in test (clippy::field_reassign_with_default) |

---

## Findings (no code change unless noted)

### F1 — NTFS fallback mutates user preference permanently

**Where:** `src/app/mod.rs` ~L619–L623 (`ScanMsg::NtfsFallback`).  
**What:** On MFT failure, the app forces `scanner_mode = ScannerMode::Standard`. That flows into persisted `PersistState` on next save — the user loses **NTFS** selection without opting in.  
**Suggestion:** Track fallback as a transient flag (`ntfs_last_error: Option<String>`) while keeping UI mode as **NTFS**; or prompt once. **Do not** silently rewrite persisted mode without UX agreement.

### F2 — `auto-allocator` version policy

**Where:** root `Cargo.toml` L23, `auto-allocator = { version = "*", ... }`.  
**What:** Floating major versions break reproducible builds ([C‑API-STABLE recommendation](https://rust-lang.github.io/api-guidelines/): predictable dependency pins).  
**Suggestion:** Pin a minimum semver range (e.g. `"0.x"` or exact) per team policy.

### F3 — Monolithic UI core

**Where:** `src/app/mod.rs` (~1500+ lines).  
**What:** Scan orchestration, render loop, docking, screenshots, events interleave — harder reviews and regressions.  
**Suggestion:** Extract `scan_orchestration` / `treemap_pipeline` modules **without** behavior change (mechanical moves + `pub(super)` API).

### F4 — Duplication surface: CLI vs persisted options

**Where:** `src/main.rs` `CliOptions` vs `PersistState` / `render_shared::Render3DOptions` application in `app::App::new`.  
**What:** Hundreds of mirrored fields increases drift risk when adding a render knob (CLI forgets persistence or inverse).  
**Suggestion:** Introduce `impl Render3DOptions { fn apply_cli_overrides(&mut self, cli: &CliOptions) }` in one module; keep struct fields but single application site (already partially inlined in `App::new` — consolidate).

### F5 — `dead_code` / `unused_imports` belts in GPU PT crates

**Where:** `pt-megakernel`, `pt-wavefront` pipelines (`#![allow(dead_code)]`, module-level `unused_imports`).  
**What:** Masks unfinished integration vs truly dead helpers.  
**Suggestion:** Prefer feature gates or targeted `#[allow]` on symbols with a one-line rationale; periodically `cargo rustc -- -W dead_code` without allows on a staging branch.

### F6 — Rendering architecture (known limitation, documented TODOs)

**Where:** `src/app/mod.rs` ~L1035–L1074.  
**What:** Stable path allocates `ColorImage` + `load_texture` per frame — correct but CPU/GPU-bound. Comments reference shared eframe device and double buffering.  
**Suggestion:** Schedule as a milestone: one `wgpu::Device` for egui + custom passes, texture pool / ping-pong for PT output.

### F7 — Minimal `FIXME`/`TODO` in application Rust

**Grep:** `TODO` only in `src/app/mod.rs` (zero-copy). No `FIXME`/`unimplemented!` in `src/` at scan time.  
**Residual risk:** Larger unfinished areas are guarded by `#![allow(dead_code)]` in crates (see F5).

### F8 — SSOT clarity

**Good:** `dirstat_core::DirEntry` is the authoritative scan tree shape; scanners both materialize compatible trees.  
**Watch:** Filters (`app/filters.rs`) and LoD merging must stay consistent with `DirEntry::lod_expand` semantics (`dirstat-core`).

---

## Deferred work (approval gate)

| ID | Task | Risk |
|----|------|------|
| D1 | Redesign NTFS fallback vs persisted `scanner_mode` | UX + Windows-only QA |
| D2 | Pin `auto-allocator` | Cargo.lock already pins; Cargo.toml ergonomics |
| D3 | Split `app/mod.rs` | Mechanical; merge conflicts |
| D4 | Single `CliOptions → Render3DOptions` applicator | Medium refactor |
| D5 | Zero-copy / texture reuse with eframe | High; correctness + GPU sync |

---

## References

- `src/main.rs` — CLI parsing, logging, `eframe::run_native`
- `src/scanner.rs` — jwalk aggregator, `ScanMsg`, `scan_dir_public` for NTFS fallback
- `src/scanner_ntfs.rs` — MFT/USN enumeration, fallback chain L109–L120
- `src/app/state.rs` — `ScannerMode`, `PersistState`
- `src/events.rs` — lightweight event queue
- `src/path_key.rs` — SHA-256 id for caches
- `DIAGRAMS.md` — Mermaid overview (updated in same session)
- `AGENTS.md` — ASCII diagrams for agents (new)

---

**Status:** Waiting for approval on deferred items **D1–D5**. Immediate clippy-clean workspace achieved.
