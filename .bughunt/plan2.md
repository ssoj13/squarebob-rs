# Bug Hunt Plan 2 — Remediation and Verification Report

Date: 2026-07-15
Repository: `squarebob-rs`
Branch: `main`
Disposition: implementation complete for approved classes; no commit created

## Approval gate

This report records implemented changes and current verification limits.

Do not commit or broaden dependency changes until the user approves this report. The full workspace build is blocked by a missing local `vfx-rs` checkout referenced at `Cargo.toml:238`–`Cargo.toml:243`. Changing that override is a separate dependency-policy decision and was not inferred.

## Executive status

| Area | Status | Evidence |
|---|---|---|
| Directory-tree recursion/depth class | VERIFIED | 43 retained isolated production tests; deep chains up to 20,000 nodes |
| NTFS parser/tree construction | VERIFIED IN ISOLATION | 14 production tests; live privileged-volume scan still required |
| Cache validation/round trip | VERIFIED IN ISOLATION | 13 production tests; artificial depth cap removed |
| Treemap traversal/memory safety | VERIFIED IN ISOLATION | 6 production tests; no unsafe pixel writes |
| GPU readback/layout error propagation | IMPLEMENTED, WORKSPACE BUILD BLOCKED | centralized checked layout and typed errors |
| PT/GPU contracts | IMPLEMENTED, WORKSPACE BUILD BLOCKED | shared frame path, checked sizing, shader contract consolidation |
| Encode/frame lifecycle | IMPLEMENTED, PARTIALLY VERIFIED | frame harness passed; full app build blocked |
| Full workspace | BLOCKED | missing `../vfx-rs/crates/foundation/vfx-core/Cargo.toml` |

## Closed problem classes

### R01 — Recursive `DirEntry` operations could overflow the process stack

Problem sites included cloning, dropping, sorting, formatting, filtering, cache conversion, UI flattening, picking, instance collection, and treemap queries.

Systemic resolution:

- `DirEntry` now exposes reusable iterative pre-order and post-order traversal at `crates/squarebob-core/src/lib.rs:110` and `crates/squarebob-core/src/lib.rs:125`.
- Clone and destruction are iterative at `crates/squarebob-core/src/lib.rs:145` and `crates/squarebob-core/src/lib.rs:195`.
- Mutable traversal and sort share the same explicit-stack model at `crates/squarebob-core/src/lib.rs:237` and `crates/squarebob-core/src/lib.rs:326`.
- Recursive derive-based formatting/serialization was removed from the owned tree. Debug formatting is iterative; cache persistence uses a flat DTO.
- App statistics and lookup use shared traversal at `src/app/helpers.rs:127`, `src/app/helpers.rs:150`, and `src/app/helpers.rs:158`.
- Tree-panel flattening is iterative at `src/app/tree_panel.rs:246`.
- Filter rebuild logic is consolidated into one iterative engine at `src/app/filters.rs:47`; exclusion and size merging use it at `src/app/filters.rs:315` and `src/app/filters.rs:349`.
- CPU picking and 3D instance collection use explicit stacks at `crates/render-3d/src/renderer3d/cpu_pick.rs:86` and `crates/render-3d/src/renderer3d/instance_collect.rs:113`.
- CPU/GPU treemap collection and queries are iterative at `crates/treemap/src/lib.rs:610`, `crates/treemap/src/wgpu.rs:378`, `crates/treemap/src/lib.rs:803`, and `crates/treemap/src/lib.rs:836`.

Regression coverage:

- Core deep-tree tests start at `crates/squarebob-core/src/lib.rs:335`; 20,000-node clone/traverse/sort/drop and 4,096-node debug coverage.
- Treemap query regression uses depth 4,096 at `crates/treemap/src/lib.rs:983`.

### R02 — Cache depth policy truncated valid trees and disagreed with runtime ownership

Problem: cache validation imposed a 1,024-level policy while runtime trees had no equivalent semantic limit. This rejected valid scan data and created a second tree-depth contract.

Systemic resolution:

- Flat persistence remains the only cache representation at `src/cache.rs:50`.
- Flat-node validation is iterative and bounded by node/byte/allocation invariants, not arbitrary depth, at `src/cache.rs:650`.
- Owned-tree validation is iterative at `src/cache.rs:814`.
- Deep round trip exceeds the removed limit at `src/cache.rs:1084` and `src/cache.rs:1086`.

### R03 — NTFS scanner selected invalid volumes and lost correctness on deep/cyclic trees

Problem:

- Ad hoc drive-letter extraction could interpret relative or UNC paths as unrelated local volumes.
- Recursive MFT tree construction and size aggregation could overflow.
- Cycle/depth behavior could silently lose data.
- Backend/cancellation/partial-result states were not distinct.

Systemic resolution:

- One normalized `VolumeTarget` parser owns volume selection at `src/scanner_ntfs.rs:29` and `src/scanner_ntfs.rs:35`.
- USN record headers and record versions are parsed through typed functions at `src/scanner_ntfs.rs:539` and `src/scanner_ntfs.rs:633`.
- MFT subtree construction is iterative at `src/scanner_ntfs.rs:1342`.
- Size aggregation is iterative post-order at `src/scanner_ntfs.rs:1542`.
- Cycle detection is ancestry-local: the cycle branch is isolated and diagnosed without suppressing valid siblings.
- Shared typed outcomes and diagnostics live at `src/scanner.rs:15` and `src/scanner.rs:46`.

Regression coverage:

- Depth 4,096 subtree preservation: `src/scanner_ntfs.rs:1933`.
- Localized cycle handling: `src/scanner_ntfs.rs:1969`.
- Depth 4,096 size aggregation: `src/scanner_ntfs.rs:2006`.

Runtime caveat: these tests validate production parser/tree code. They do not prove a real privileged raw-volume open on this machine. A live NTFS scan remains mandatory after the workspace can build.

### R04 — Scan replacement, terminal state, cache work, and path identity were fragmented

Problem: replacing a scan could orphan the previous worker; stale generations could update UI; partial scans could be cached; cache work could block UI or race detached writers; path identity varied by caller.

Systemic resolution:

- `ScanSession` owns generation, root, cancellation, receiver, and worker at `src/scanner.rs:90`; spawn is fallible at `src/scanner.rs:134`.
- Replacement cancels and retires the prior generation at `src/app/scan_orchestration.rs:47`; finished workers are reaped at `src/app/scan_orchestration.rs:54`.
- New scans clear presentation state before launch at `src/app/scan_orchestration.rs:299`.
- Partial results are surfaced and not cached at `src/app/scan_orchestration.rs:253`.
- Cache work runs through one service at `src/app/scan_orchestration.rs:71`.
- Writes use same-directory temporary files, file flush, atomic replacement, and directory flush at `src/atomic_file.rs:33`, `src/atomic_file.rs:36`, `src/atomic_file.rs:160`, and `src/atomic_file.rs:168`.
- `ScanRoot` centralizes canonical operational path, display path, and stable identity at `src/path_key.rs:12` and `src/path_key.rs:19`.

### R05 — Treemap solid rendering relied on unchecked unsafe parallel writes

Problem: release safety depended on rectangles being disjoint; a layout regression could produce aliasing or out-of-bounds writes.

Systemic resolution:

- Both cushion and solid paths partition the image into Rayon-owned disjoint rows at `crates/treemap/src/lib.rs:533` and `crates/treemap/src/lib.rs:546`.
- Rectangles within each mutable row are processed sequentially at `crates/treemap/src/lib.rs:549`.
- No unsafe block remains in `crates/treemap/src/lib.rs`.
- Layout-disjointness remains a diagnostic invariant, not a memory-safety precondition, at `crates/treemap/src/lib.rs:522`.

### R06 — GPU readback arithmetic and failure handling were duplicated and panic-prone

Problem: width/height/bytes-per-pixel arithmetic could wrap before conversion; map/poll/channel errors lost identity; picking and PT CPU color duplicated mapper logic.

Systemic resolution:

- Checked 2D count/byte/alignment helpers are centralized at `crates/render-core/src/lib.rs:240`, `crates/render-core/src/lib.rs:273`, `crates/render-core/src/lib.rs:287`, and `crates/render-core/src/lib.rs:319`.
- One `TextureReadbackLayout` owns row pitch and total byte layout at `crates/render-core/src/lib.rs:541`.
- Readback copy/unpack paths delegate to shared helpers at `crates/render-core/src/lib.rs:643`, `crates/render-core/src/lib.rs:694`, and `crates/render-core/src/lib.rs:715`.
- Typed failure identity is preserved by `ReadbackError` at `crates/render-core/src/lib.rs:751`.
- Callback send, poll, receive, mapped-range validation, and unmap are centralized at `crates/render-core/src/lib.rs:806`.
- Object-ID picking propagates checked layout and mapping errors at `crates/render-3d/src/picking.rs:125` and `crates/render-3d/src/picking.rs:191`.

### R07 — PT entrypoints and shader contracts had diverged

Implemented resolution:

- CPU-readback and GPU-only wrappers delegate to one frame state machine at `crates/render-3d/src/pt/megakernel/render.rs:31` and `crates/render-3d/src/pt/megakernel/render_no_readback.rs:13`.
- Output transport is an explicit `PtOutput` policy at `crates/render-3d/src/pt/megakernel/render.rs:6`.
- ReSTIR receiver/material/target logic is shared at `crates/pt-megakernel/src/restir/common.wgsl:1` and `crates/pt-megakernel/src/restir/common.wgsl:127`.
- ReSTIR visibility traversal is shared at `crates/pt-megakernel/src/restir/visibility.wgsl:1`.
- Fixed traversal stacks no longer silently drop subtrees: main, shadow, pick, and ReSTIR overflow paths fall back to exhaustive instance traversal at `crates/pt-megakernel/src/bvh_traverse.wgsl:453`, `crates/pt-megakernel/src/bvh_traverse.wgsl:502`, `crates/pt-megakernel/src/pick.wgsl:158`, and `crates/pt-megakernel/src/restir/visibility.wgsl:96`.
- Shared half-open RNG seeding/conversion lives at `crates/pt-core/src/rng.wgsl:7`, `crates/pt-core/src/rng.wgsl:13`, and `crates/pt-core/src/rng.wgsl:32`.
- Material IOR is sanitized once at the GPU boundary at `crates/standard-surface/src/params.rs:64` and `crates/standard-surface/src/params.rs:76`.
- GPU BVH counts, allocations, and topology validation use checked helpers at `crates/bvh-gpu/src/bvh_gpu/mod.rs:64`, `crates/bvh-gpu/src/bvh_gpu/mod.rs:77`, and `crates/bvh-gpu/src/bvh_gpu/mod.rs:1345`.

Status: source audit complete. Full Rust/WGSL compilation remains blocked by the missing local dependency.

### R08 — Encode lifecycle and frame layout accepted invalid or overlapping state

Systemic resolution:

- `PixelLayout` owns checked dimensions, channels, stride, and byte length at `crates/media-encoder/src/frame.rs:78`.
- Frame construction is fallible at `crates/media-encoder/src/frame.rs:270`.
- Cropping consumes validated layout at `crates/media-encoder/src/frame.rs:305` and `crates/media-encoder/src/frame.rs:473`.
- Encode ownership is explicit `Idle | Running | Cancelling | Finishing` at `crates/media-encoder/src/dialogs/encode/encode_ui.rs:55` and `crates/media-encoder/src/dialogs/encode/encode_ui.rs:61`.
- Generation-owned cancellation token is defined at `crates/media-encoder/src/dialogs/encode/encode_ui.rs:25`.

## Verification performed

| Command | Result |
|---|---|
| `cargo test --manifest-path .bughunt/core-harness/Cargo.toml` | 3 passed |
| `cargo test --manifest-path .bughunt/filters-harness/Cargo.toml` | 7 passed |
| `cargo test --manifest-path .bughunt/ntfs-harness/Cargo.toml` | 14 passed |
| `cargo test --manifest-path .bughunt/cache-harness/Cargo.toml` | 13 passed |
| `cargo test --manifest-path .bughunt/treemap-harness/Cargo.toml` | 6 passed |
| `rustfmt crates/treemap/src/lib.rs` | passed |
| Residual Rust search for removed recursive helpers/depth cap | no matches |
| Residual search for readback `recv().unwrap` / callback `send(...).unwrap` | no matches |
| `cargo check --workspace --all-targets` | blocked before compilation |

Total isolated tests: 43 passed, 0 failed.

The NTFS harness emits one expected dead-code warning for `decode_error_to_scan` because the isolated harness does not compile the full Windows entrypoint. No deletion was made.

## Workspace blocker

`cargo check --workspace --all-targets` fails before compiling workspace code:

```text
error: failed to load source for dependency `vfx-core`
failed to read C:\projects\projects.rust.cg\cglibs\vfx-rs\crates\foundation\vfx-core\Cargo.toml
```

Cause: local overrides at `Cargo.toml:238`–`Cargo.toml:243`.

Proper options require user choice:

1. Restore the expected sibling `vfx-rs` checkout.
2. Remove the local patch only if the referenced upstream changes have landed and the git dependencies are now authoritative.
3. Point the patch at another verified local checkout.

No automatic edit is safe here.

## GitNexus change detection

Required `gitnexus_detect_changes(scope: "all")` completed.

- Changed files: 83.
- Changed symbols: 934.
- Affected processes/symbols: 260.
- Aggregate risk: CRITICAL.

This is an aggregate over the already-dirty worktree, including broad pre-existing GPU/PT/application edits. It is not evidence that every listed file was changed by this final recursion pass. No commit was created.

## Remaining acceptance work

1. Resolve the `vfx-rs` dependency policy.
2. Run `cargo check --workspace --all-targets`.
3. Run full workspace tests and shader compilation/validation.
4. Run a real NTFS scan against a known directory from an elevated process; verify raw-volume open, selected subtree, progress, cancellation, partial-error UI, and cache round trip.
5. Run GPU smoke tests for raster, PT GPU-only, PT readback, picking, and screenshots.
6. Re-run GitNexus analysis and `detect_changes` after any dependency or follow-up edits.
7. Review final scoped diff before commit.

## Approval requested

Approve this report to proceed with dependency resolution and full end-to-end verification. No source deletion, compatibility shim, or commit is authorized by this report.
