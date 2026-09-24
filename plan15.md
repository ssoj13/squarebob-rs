# plan15 — Typed NTFS outcomes and dependency verification

Updated: 2026-09-23 PDT. This report follows [plan14.md](plan14.md). It describes the current uncommitted Squarebob source and lockfile changes against published `main` at `e662a71`, plus the published `fscan-rs` commit `c33d4474b5004ec047ecd065f96f6a53addbc97e`. Recheck the worktree and commit state before using it as a release record.

## Work checklist

- [x] Verify that `fscan-rs` preserves typed NTFS failures at its public tree API.
- [x] Verify that Squarebob maps cancellation, unavailable backend, and fatal failure to distinct outcomes.
- [x] Pin the published `fscan-rs` commit in `Cargo.toml` and `Cargo.lock`.
- [x] Remove the incidental Windows dependency changes introduced by `cargo update`; retain `gpu-allocator` on `windows 0.62.2`.
- [x] Run `cargo metadata --locked --no-deps` and `cargo check --workspace --locked -q` on Windows.
- [ ] Run focused NTFS outcome tests and a representative app scan when justified.
- [ ] Measure large standard scans against the earlier `jwalk` path.
- [ ] Restore CI authentication for private `oiio-rs`, then evaluate Clippy, tests, and packaging.
- [ ] Review, commit, and push the current Squarebob source, lockfile, and documentation changes.

## Typed NTFS outcome

The published `fscan-rs` commit `c33d447` exports `ScanFailure::{Cancelled, BackendUnavailable, Failed}` (`../fscan-rs/src/scanner.rs:99-105`; `../fscan-rs/src/lib.rs:15-21`). Its public NTFS tree functions return that type and preserve the worker result (`../fscan-rs/src/ntfs.rs:394-433`). Squarebob pins the exact commit in `Cargo.toml:179` and `Cargo.lock:3337-3345`.

Squarebob's NTFS adapter now maps `Cancelled` to `ScanOutcome::Cancelled`, sends `NtfsFallback` and runs the standard scanner only for `BackendUnavailable`, and maps `Failed` to a terminal failure (`src/scanner_ntfs.rs:112-154`). This restores the typed distinction that existed before the scanner extraction. The earlier catch-all error branch was recorded in [plan14.md](plan14.md). No focused outcome test or runtime NTFS scan was run for this change.

```text
fscan-rs NTFS worker -> Result<Tree, ScanFailure>
  |-- Cancelled          -> Squarebob Cancelled
  |-- BackendUnavailable -> NtfsFallback notice -> standard scan
  |-- Failed             -> Squarebob Failed
  `-- Ok(tree)           -> owned DirEntry tree -> finish_build
```

## Lockfile and compilation

An intermediate `cargo update` resolution selected `windows 0.56.0` for `gpu-allocator 0.28.0` and broke the Windows `wgpu-hal 30.0.1` build. The root agent removed all 11 incidental Windows dependency changes. The current `Cargo.lock` still records `gpu-allocator` depending on `windows 0.62.2` (`Cargo.lock:3725-3737`), consistent with `wgpu-hal` (`Cargo.lock:8632-8684`). The current lockfile diff against `e662a71` changes only the `fscan-rs` Git source SHA (`Cargo.lock:3337-3339`).

The root agent reports that `cargo metadata --locked --no-deps` and a repeat `cargo check --workspace --locked -q` both exited 0 on Windows after the lockfile cleanup, with empty check stderr. These commands establish manifest resolution and compilation for that environment. No tests or app runtime scan were run for this revision.

## CI and remaining verification

Earlier CI jobs failed while authenticating to the private `oiio-rs` Git dependency, before Clippy. That blocker predates the typed NTFS and lockfile changes. No new CI run was started for this revision to conserve CI budget. Therefore Clippy, workspace tests, portable packaging, and cross-platform behavior remain unverified.

The standard `fscan-rs` walker uses serial `WalkDir` (`../fscan-rs/src/lib.rs:142-154`); the earlier Squarebob walker used parallel `jwalk`. No throughput benchmark has established the magnitude of any change. The omitted-parent repair remains source-reviewed and compiled, but its dedicated regression test and runtime behavior are still open ([plan14.md](plan14.md); `src/scanner.rs:302,315-366`).

## Documentation verification

This report cites the current source and lockfile. `README.md`, `AGENTS.md`, `DIAGRAMS.md`, `BUG.md`, `plan14.md`, and this report were reread after editing. `git diff --check` exited 0 for tracked changes; this new report remains untracked until publication. Documentation review did not add a build or test result.
