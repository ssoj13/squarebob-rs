# Technical Concerns & Debt

**Analysis Date:** 2026-04-20

## Supply Chain / Build

- **`auto-allocator = { version = "*" }`** (`Cargo.toml`) — Unpinned wildcard dependency weakens reproducibility; pin a semver range when policy allows.

## Performance / GPU

- **Zero-copy / shared device** — Comments in `src/app/mod.rs` (~784, ~817) note **TODO** for sharing **eframe’s wgpu device** and **double-buffering** the 3D path to avoid stalling/blocking; current approach may use readback paths that cost latency.
- **`render-3d`** — Heavy use of **`unwrap()`** on `Option` cache fields (e.g. `cached_instances`, `targets`, `dyn_bgs`) assumes strict init ordering; failure modes are panics if invariants break (`crates/render-3d/src/lib.rs` — multiple sites).

## Robustness

- **Sparse automated tests** — Most behavior verified by running the desktop app; regressions in layout/filter math may slip through (see `TESTING.md`).
- **Cache key migration** — Stable path keys (`src/path_key.rs`) may **not match** older cache filenames if users relied on previous hash formatting; may appear as “missing cache” until rescan or clear (operational note, not necessarily a code bug).

## Platform / Security

- **NTFS scanner** — Elevated capabilities on Windows; fallback paths must remain robust when permissions deny MFT access (`src/scanner.rs` / `src/scanner_ntfs.rs`).
- **Trash / shell** — User-initiated destructive actions; rely on OS dialogs; keep path validation consistent when extending remote FS support.

## Dependency Drift

- **egui / wgpu** — Frequent API migrations (panels `show_inside`, `global_style`, etc.); budget time for upgrades across `eframe`, `egui_dock`, and custom wgpu code.

## Documentation Debt

- **`plan1.md`** — Audit list may be partially **stale** vs current code (several “open” items already addressed in later sessions); reconcile before treating as backlog of record.

---

*Concerns analysis: 2026-04-20*
