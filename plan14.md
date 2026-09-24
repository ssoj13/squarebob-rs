# plan14 — Documentation synchronization and packaging audit

Initial documentation checkpoint: 2026-09-23, based on committed `main` at `6562a5280c42195345dee2fef36c59254ee7d894`. The observations below describe that checkpoint. Publication update: `fscan-rs` was published privately at `aa0185f8a45dcf3ad949d97b88b5950009444cb2`, Squarebob pinned that Git revision, and its scanner unit tests and `cargo check` passed on Windows. Large-scan runtime parity and packaging remain unverified.

## Work checklist

- [x] Read the root Markdown files and the four `docs/` records before editing.
- [x] Compare active README claims with `Cargo.toml`, `rust-toolchain.toml`, `.github/workflows/ci.yml`, `bootstrap.py`, and the committed bug-hunt reports.
- [x] Update active architecture and dataflow pointers; retain historical plans with clear status notices.
- [x] Record the package/binary-name mismatch with source references in `BUG.md`.
- [x] Recheck the visible standard and NTFS scanner adapters, update active dataflows, and confirm current local-path compilation. Runtime and publication remain open.
- [x] Replace the local `../fscan-rs` path with a published Git SHA and regenerate `Cargo.lock` before pushing.
- [x] Complete full-file reread, link review, and diff checks for this documentation set.
- [x] Include the reviewed documentation in the authorized `main` push.

## Documentation changes

- `README.md`: current dependency versions from `Cargo.toml:52-74,105-116`; the workspace member list from `Cargo.toml:15-37`; CI cancellation behavior from `.github/workflows/ci.yml:31-35`; the visible local `../fscan-rs` dependency; and links to architecture, bug reports, and historical design records.
- `AGENTS.md` and `DIAGRAMS.md`: identify plan13 as the latest completed repair, retain the plan11/plan12 evidence trail, and show the shared reversed-Z CPU picking ray (`crates/render-3d/src/renderer3d/cpu_pick.rs:29-58`; `crates/render-3d/src/lib.rs:1343-1378`).
- `CHANGELOG.md`: summarize the committed 2026-09-23 dependency, scan, CLI, render, export, and camera changes. The warning-free workspace check is a result recorded in plan13, not a new check from this pass.
- `PLAN.md`, `plan11.md`, `plan12.md`, and `plan13.md`: distinguish historical checklists from current status and record the published plan12/plan13 commits. Earlier line references and warning reports remain as evidence of their original review states.
- `docs/aces-color-pipeline-plan.md` and the three OIDN plan/survey files: retain the original content and add notices that their API versions and proposals are historical. Each is linked from the README.
- `BUG.md`: record the portable archive name mismatch and its source-level repair.

## Portable package binary name

`Cargo.toml:11-13` declares the binary target `squarebob`, and `.github/workflows/ci.yml:37-40` now sets `BINARY_NAME: squarebob`. The Linux and Windows portable steps use that variable (`ci.yml:243-254,257-274`). The previously confirmed source-level mismatch is resolved. Neither packaging branch has been run, so archive contents and CI success remain unverified.

## Concurrent source boundary

At the initial checkpoint, the worktree had uncommitted scanner and dependency edits. The standard path calls `fscan_rs::scan_standard` (`src/scanner.rs:308-421`); the Windows adapter delegates NTFS probing, diagnosis, and tree scanning to `fscan-rs`, then converts the returned tree to owned `DirEntry` nodes (`src/scanner_ntfs.rs:22-150`). The initial local-path dependency has since been replaced with the Git revision pinned in `Cargo.toml` and `Cargo.lock`. The earlier workspace compilation check passed; subsequent scanner unit tests also passed on Windows.

## Standard-scanner partial-tree regression

Source review found that `fscan-rs` can omit a directory callback after a metadata error while still yielding descendants. The previous adapter could lose those descendants. The current `scan_dir` validates callback paths and reconstructs each missing ancestor once before tree assembly (`src/scanner.rs:302,315-366,432-440`). Walker errors remain in `ScanDiagnostics`, so `finish_build` can return `Partial` (`src/scanner.rs:313-314,412-421,204-236`). The source-level repair is present and compiles; no regression test or runtime scan was run for this case.

## Remaining verification

- Review the omitted-parent repair and scanner behavior at a justified runtime gate, including standard traversal, NTFS fallback, cancellation, diagnostics, and cache identity. The source repair and compilation do not establish these behaviors.
- Compare large-scan throughput before and after the `jwalk` to `fscan-rs` migration. The current `scan_standard` iterates a serial `WalkDir` in the pinned `fscan-rs` crate, while the earlier standard walker used a Rayon-backed `jwalk` pool. No benchmark has measured the effect.
- Review the staged scope before committing. Run the Linux and Windows portable packaging branches at a justified CI gate; the source-level name repair alone does not establish packaging success.

## Documentation verification

Every changed Markdown file was reread after editing. All 16 unique local link targets referenced by those files exist; Markdown fences are balanced. `git diff --check` exited 0. No Markdown linter or Prettier executable is installed in PATH, so no linter command was run. The root agent ran `cargo check --workspace --locked -q` on the current local-path scanner worktree; it exited 0 with empty stderr. No tests, application run, visual comparison, or packaging workflow was performed for this documentation pass.
