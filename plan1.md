# Bug hunt report — plan1 (bughunt 2026-05-11)

**Scope:** Static audit + periodic `cargo clippy` / `cargo test`. **2026-05-12 follow-up:** roadmap + changelog + README + `HANDOFF` synced; `ramp_widget` module `dead_code` belt removed (see §4).

---

## Executive summary

- **Toolchain health:** Clippy passes with `-D warnings`. All workspace unit tests pass (44 tests total across crates; see §8).
- **No literal `TODO` / `FIXME` tokens** in `src/**/*.rs`; roadmap text in markdown still references obsolete line numbers and “two TODO markers in `mod.rs`” (removed in sprint 3).
- **Main actionable themes:** documentation drift (`TODO4.md`, `DIAGRAMS.md`, test count in `AGENTS.md`), optional hardening of `unwrap()`/`as_ref().unwrap()` invariants with `.expect("…")`, and phased removal of `#![allow(dead_code)]` where symbols are actually wired (`ramp_widget.rs`).

---

## 1. Documentation drift (high value, low risk)

| Location | Issue | Verified fact |
|---------|-------|----------------|
| `TODO4.md` §0.2 (approx. lines 232–237) | Suggests `NtfsFallback` handler in `src/app/mod.rs:619–623` and optional `ntfs_last_error`. | **Fixed in rev 6** — handler documented at `scan_orchestration.rs::poll_scan`; optional banner still open. |
| `TODO4.md` rev 5 / §D | States “only two `TODO` markers in `src/app/mod.rs`”. | **Fixed in rev 6** — codebase has no literal `TODO` in `src`; D.1 marked done. |
| `DIAGRAMS.md` NTFS sequence | Note said UI “forces `scanner_mode` Standard`. | Contradicts `scan_orchestration.rs` and `AGENTS.md`. **Diagrams updated** in bughunt pass. |
| `DIAGRAMS.md` § GPU / display | Implied zero-copy still “planned”. | **Updated** — Mermaid shows zero-copy vs readback vs roadmap. |
| `AGENTS.md` maintenance | Claims “24 unit tests”. | **Updated** — 44 tests + `plan1.md` §8. |

---

## 2. Stale / misleading comments in code

| File | Notes |
|------|------|
| `src/app/treemap_view.rs` | **Verified:** header imports contain no stale “Zero-copy disabled” comment in the current tree; keep it that way. If resurrected commented-out `render_callback` imports, delete — zero-copy paths are `render_*_callback` + `register_native_texture`. |

---

## 3. `unwrap()` and internal invariants

**Pattern:** Several sites use `.as_ref().unwrap()` after a predicate guarantees `Some` (same function). Rust API guidelines prefer **`expect` with invariant message** where the branch is logically unreachable but regression would panic anyway.

Examples (not exhaustive):

- `src/app/treemap_view.rs:808` — after `ctx_menu_path.is_none()` early return; `clone()` + `unwrap()` could be `if let Some(path) = …` / `take()` pattern for idiomatic Rust (minor).
- `src/app/treemap_view.rs:976`, `:1170` — `wgpu_render_state.as_ref().unwrap()` only reached when ```32:36:src/app/treemap_view.rs``` (`use_callback`); invariant holds. Prefer `expect("ui_treemap: wgpu_render_state set when use_callback")` if touched for other reasons.

GPU / PT internals (`pt-megakernel`, `bvh-gpu`, `treemap::wgpu`, etc.) use extensive `unwrap()` on optional GPU resources — typical for pipelines where `resize`/`init` establishes buffers before draw; upgrading to `expect` is a consistency pass, not urgent if init order tests hold.

---

## 4. `#[allow(dead_code)]` inventory

Broadly justified categories:

- **`scanner_ntfs.rs`**: non-Windows stubs / parity (`cfg` + comment at e.g. 70, 626, 752).
- **`targets.rs`**, **GPU crates**: fields kept alive for view lifetimes (`depth_view` comment).
- **`pt-megakernel` / `pt-mats` / `bvh-gpu`**: large shader pipelines; unused helpers retained for experimentation.

**`ramp_widget.rs` (verified 2026-05-12):** module-level `#![allow(dead_code)]` **removed**.
Clippy then flagged only `RampUiCtx::compact` as unused — kept as **reserved API** with
item-level `#[allow(dead_code)]` + doc comment. Full workspace `cargo clippy … -D warnings` **passes**.

---

## 5. Deprecated egui API

- `treemap_view.rs` uses **`#[allow(deprecated)]`** around `egui::show_tooltip_at_pointer` (~line 185). Track egui migration to the non-deprecated tooltip API when upgrading egui.

---

## 6. Duplication / SSOT

- Scan engine label + error UX for NTFS fallback is centralized in **`ScanProgress`** via `scan_orchestration.rs` — good SSOT boundary.
- `render_treemap` in `mod.rs` documents when CPU readback path runs vs zero-copy callbacks — **keep** this as canonical comment block (already aligns with ```586:597:src/app/mod.rs```).
- **`TODO4.md`** (roadmap / stage numbers) vs **`HANDOFF.md`** (sprint narrative) vs **`plan1.md`** (bughunt forensics): **reconciled 2026-05-12** — `TODO4` is the single source for *numbered* stage backlog; `HANDOFF` points to it explicitly; `plan1` explicitly defers to `TODO4` for priorities (see `TODO4` header + `HANDOFF` documentation map).

### Verification notes (2026-05-12)

- Grep `ScanMsg::NtfsFallback` → only `scanner.rs` (enum), `scanner_ntfs.rs` (send),
  `scan_orchestration.rs` (handle) — **no** duplicate handler in `mod.rs`.
- Settings ramp UI: **`TODO4` no longer claims “only two TODO in mod.rs”**; `src/**/*.rs` grep for `\\bTODO\\b` is empty.

---

## 7. Verification log (machine)

Commands (Windows PowerShell):

```powershell
Set-Location "C:\projects\projects.rust.cg\dirstat-rs"
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
```

Results: **clippy OK** (2026-05-11); **all tests OK**.

---

## 8. Unit test counts by crate (workspace)

From latest `cargo test --workspace`:

- `dirstat-rs` (binary): 6  
- `bvh-gpu`: 2  
- `pt-mats`: 13  
- `pt-wavefront`: 3  
- `render-3d`: 8  
- `render-shared`: 7  
- `treemap`: 5  

**Total: 44** (crates with 0 tests omitted).

---

## 9. Proposed next steps (remaining)

1. ~~**`TODO4.md` NTFS / D.1 / roadmap hygiene**~~ — Done in **rev 6** (2026-05-12).
2. **unwrap → expect/if-let hygiene:** Narrow pass on `treemap_view` callbacks and context menu (`handle_context_menu`).
3. **`ramp_widget` `dead_code`:** ✅ module-level allow removed; `RampUiCtx::compact` carries item-level allow (see §4).
4. **egui migration:** Replace deprecated tooltip helper when adopting newer egui.
5. **Optional tests:** `TODO4.md` Stage 0.3 — `classify_path_filtered` table-driven tests; treemap golden layout tests.

---

## 10. Artifacts touched in this session

| File | Change |
|------|--------|
| `TODO4.md` | **Rev 6** — §0.2, D.1, execution table Wave A, test inventory, roadmap hygiene. |
| `CHANGELOG.md` | Sprint-3 test footnote **44**; NTFS changelog clarification; unreleased docs-sync block **2026-05-12**. |
| `README.md` | **Testing / CI parity** section; Stack table bumped to egui **0.34** / egui_dock **0.19** / wgpu **29**. |
| `HANDOFF.md` | **Documentation map** (TODO4 vs plan1 vs CHANGELOG). |
| `AGENTS.md` | Open-engineering backlog bullet — no literal `TODO` in `src`. |
| `src/app/settings/ramp_widget.rs` | Module `#![allow(dead_code)]` removed; `RampUiCtx::compact` item-level allow. |

---

*End of plan1.md — awaiting user approval before deeper refactors.*
