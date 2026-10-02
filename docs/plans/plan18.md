# plan18 — Published OIDN dependency delivery

Updated: 2026-10-02. Status: **published OIDN dependency verified; Squarebob publication confirmed**. Continues [plan16](plan16.md), [plan17](plan17.md), and [OIDN plan4](../../../oidn-rs/plan4.md#publication-receipt--2026-10-02).

## Authorization and delivered source

After the systematic OIDN repairs and recorded verification, the user instructed “пушни всё в main”. OIDN source repairs were pushed to `main` at `09bdf7e4b7e5c091e262400c7eb7cfcbe9677763`; `git ls-remote` confirmed the remote revision. Squarebob's bridge update and document moves were committed by another contributor as `b1afc7d` and preserved.

The bridge resolves weights through canonical `RtFilter::builder` commit and updates scale at runtime. Its committed cache key retains roles, quality, and geometry. This avoids a separate byte-resolution cache and rebuilding the model for exposure changes. The source and frozen-input verification are recorded in OIDN plan4.

## Lockfile scope and caught regression

`cargo update -p oidn-rs` refreshed `oidn-rs`, `oidn-model`, and `oidn-tza` to the published revision through the actual SSH Git source. It also rebound nine unrelated Windows dependency references, producing incompatible DX12 types in `gpu-allocator`. Those nine references were restored using checked old/new package blocks. The final lockfile diff changes only the three OIDN packages and their dependencies; no local path override is used.

The initial failed check remains in [stdout](../../../oidn-rs/bughunt/native-verification/bob_published_contract_check.stdout.log) and [stderr](../../../oidn-rs/bughunt/native-verification/bob_published_contract_check.stderr.log). The successful retry below covers the corrected lock.

## Verification receipts

| Command | Observed result |
| --- | --- |
| `cargo check --workspace --all-targets --locked --quiet` | Exit0 in8.374s; empty [stdout](../../../oidn-rs/bughunt/native-verification/bob_published_contract_check_retry.stdout.log) and [stderr](../../../oidn-rs/bughunt/native-verification/bob_published_contract_check_retry.stderr.log) |
| `cargo build -p squarebob-rs --bin squarebob --locked --quiet` | Exit0 in39.011s; empty [stdout](../../../oidn-rs/bughunt/native-verification/bob_published_build.stdout.log) and [stderr](../../../oidn-rs/bughunt/native-verification/bob_published_build.stderr.log) |

All nine relocated documents (`BUG.md`, `PLAN.md`, and `plan11.md` through `plan17.md`) match the preceding HEAD content byte for byte. Their move to `docs/plans` is preserved; older relative links inside them remain historical.

## Publication and diagnostic scope

- [x] Publish and remotely verify the OIDN source revision.
- [x] Preserve the existing Squarebob bridge and document-move commit.
- [x] Refresh only the OIDN lock entries and check the final diff.
- [x] Check all workspace targets and link the actual binary against the published dependency.
- [x] Commit and push the Squarebob dependency update; remote `main` confirmed at `e4cd39bebf6cff907264a12b852993f66819a682`.

The dependency update was committed and pushed to Squarebob `main` as `e4cd39bebf6cff907264a12b852993f66819a682`; the remote revision was confirmed independently. Its lock retains the OIDN source revision `09bdf7e4b7e5c091e262400c7eb7cfcbe9677763`. Subsequent publication-receipt documentation does not change that API source pin.

The earlier frozen96x64 GPU diagnostic produced identical repeated and fixed-clamp outputs, including32 repeats. Adaptive SPP1/256 changed the fixture by max8.448264122. These remain bounded input-policy measurements, not a reproduction or explanation of the user's progressive scene noise. Compilation and binary linking do not extend the numerical or physical-display claims in the preceding plans.
