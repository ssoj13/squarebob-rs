# BUG

Known defects and follow-ups found from other repos and this workspace. Newest first; `[ ]` open, `[x]` done.

## 2026-09-23 NTFS failure classification after scanner extraction

- [x] **The adapter distinguishes cancellation, unavailable backend, and fatal failure.** Published `fscan-rs` commit `c33d447` returns typed `ScanFailure` from its public NTFS tree API (`../fscan-rs/src/scanner.rs:99-105`; `../fscan-rs/src/ntfs.rs:394-433`). Squarebob maps only `BackendUnavailable` to standard fallback, maps cancellation to `Cancelled`, and reports `Failed` terminally (`src/scanner_ntfs.rs:139-154`). `Cargo.toml:179` and `Cargo.lock:3339` pin the fix. The source repair compiles on Windows; no focused NTFS outcome test or runtime scan was run ([plan15.md](plan15.md)).

## 2026-09-23 Partial scan with an omitted parent

- [x] **The standard scanner now reconstructs parents omitted after metadata errors.** `src/scanner.rs:302,315-366` validates yielded paths and inserts every missing ancestor once before assembly (`src/scanner.rs:433-441`). Walker errors still flow into `ScanDiagnostics` (`src/scanner.rs:313-314,412-421`), so `finish_build` can return `Partial` (`src/scanner.rs:204-236`). The source-level loss is repaired; no regression test or runtime scan was run for this case.

## 2026-09-23 Portable package binary name

- [x] **The portable archive binary name now matches the Cargo target.** `Cargo.toml:11-13` declares `squarebob`; `.github/workflows/ci.yml:37-40` now sets `BINARY_NAME: squarebob`. The Linux and Windows portable steps use that variable (`ci.yml:243-254,257-274`). The source-level mismatch is resolved; neither packaging branch has been run in this review.

## 2026-09-23 Vendored BSDF copies

Found 2026-09-23 by the ofx-rs Standard Surface survey (`ofx-rs/.superpowers/sdd/ss-existing-survey.md`). The canonical shared BSDF is the new render-rs crate `standard-surface-bsdf` (plan: `ofx-rs/docs/superpowers/plans/2026-09-23-standard-surface-bsdf.md`); fidelity reference is MaterialX 1.39.5 in `vfx.ref`.

- [ ] **`crates/standard-surface` is a stripped vendored copy** of render-rs `standard-surface` (shaders byte-identical,
  no wgpu/pipeline/`material_ext.rs`, wgpu-29-era deps). Re-vendor from, or depend on, render-rs
  (`standard-surface-bsdf`) once it lands.
- [ ] **Diverged PT BSDF forks**: `crates/pt-megakernel/src/bvh_traverse.wgsl:523-543,766`,
  `restir/common.wgsl:155-176`, `render-3d/shaders/cube_pbr.wgsl:127-146` (Schlick/NDF sampling copies). Rebase onto
  the shared BSDF with render-rs's megakernel (render-rs BUG.md).
