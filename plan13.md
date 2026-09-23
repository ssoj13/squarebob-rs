# plan13 — Camera API warning cleanup

Updated: 2026-09-23. This report follows [plan12.md](plan12.md). It covers the four `glam` deprecation warnings left by that pass. The camera conventions and rendering dataflow remain unchanged; [AGENTS.md](AGENTS.md) and [DIAGRAMS.md](DIAGRAMS.md) still describe the paths.

## Work checklist

- [x] Read plan12 and inspect all four changed source sites and their diff against `main`.
- [x] Check the workspace Rust source for the three superseded `Mat4` calls.
- [x] Record the camera-space and depth-range contracts at each replacement.
- [x] Resolve the connected CPU fallback ray-picking depth-order defect and review its callers.
- [x] Re-run the workspace compiler and focused formatting/diff checks after that fix.
- [x] Record the final GitNexus reindex and change-detection scope.
- [ ] Publish the authorized main-branch commit; SHA and remote confirmation belong in the final chat.

## What changed

The previous `cargo check -p squarebob-rs --locked` passed but reported four deprecations: two in `OrbitCamera` and two in the GPU path-tracing smoke path ([plan12.md](plan12.md#verification-record)). The current diff replaces the deprecated constructors with the explicit `glam::camera` equivalents:

| Source | Previous API | Replacement | Contract |
| --- | --- | --- | --- |
| `crates/render-shared/src/lib.rs:1303-1305` | `Mat4::look_at_rh` | `glam::camera::rh::view::look_at_mat4` | Right-handed view matrix from camera position, target, and world up. |
| `crates/render-shared/src/lib.rs:1307-1326` | `Mat4::perspective_infinite_reverse_rh` | `glam::camera::rh::proj::directx::perspective_infinite_reverse` | Right-handed, WebGPU/DirectX depth in `[0, 1]`, infinite far plane, reversed Z. |
| `src/cli_test.rs:161-162` | `glam::Mat4::look_at_rh` | `glam::camera::rh::view::look_at_mat4` | Same right-handed view convention in the smoke scene. |
| `src/cli_test.rs:163-168` | `glam::Mat4::perspective_rh` | `glam::camera::rh::proj::directx::perspective` | Right-handed, WebGPU/DirectX depth in `[0, 1]`, finite near and far planes. |

These two projection choices are intentional. `OrbitCamera::projection_matrix` uses reversed Z with an infinite far plane. The raster path clears depth to `0.0` and uses a greater comparison (`crates/render-3d/src/renderer3d/render.rs:251`; `crates/render-3d/src/pipelines.rs:313-388`). The camera contract requires near NDC Z `1.0` and far NDC Z `0.0` (`crates/render-shared/src/lib.rs:1316-1320`); CPU fallback picking now uses the shared screen-ray conversion. `gpu_pt_smoke` uses a finite `0.1..100.0` projection only to construct the inverse projection matrix passed to `PtCameraUniform` (`src/cli_test.rs:163-173`). Replacing both with one projection helper would change a camera contract.

## Connected CPU picking defect

Before this pass, active `Renderer3D::cpu_pick` used `OrbitCamera::projection_matrix` but independently unprojected NDC Z `0.0` as `near` and `1.0` as `far`. That reversed the ray direction under this camera's reversed-Z contract. The App calls the method for CPU fallback picking (`src/app/treemap_view.rs:275,365`). It now calls the existing `Renderer3D::screen_ray` and retains its layout and tree traversal (`crates/render-3d/src/renderer3d/cpu_pick.rs:29-58`). `screen_ray` unprojects near at Z `1.0` and far at Z `0.001`; exact Z `0.0` would place the far point at infinity and invalidate the perspective divide (`crates/render-3d/src/lib.rs:1343-1378`). The old duplicate ray construction and unused `Vec4` import were removed. The `screen_ray` comment was updated to name the new projection API (`crates/render-3d/src/lib.rs:1358-1364`).

## Codepaths and impact

```text
OrbitCamera::position -> OrbitCamera::view_matrix ─┐
                                                    ├─> view_projection_matrix -> raster/PT camera consumers
OrbitCamera::fov, near -> projection_matrix ────────┘
    right-handed, reversed-Z, infinite far, WebGPU [0, 1]

App fallback pick -> cpu_pick -> screen_ray -> reversed-Z ray -> pick_tree

gpu_pt_smoke position -> right-handed view -> inverse view ─┐
finite near/far projection -> inverse projection ──────────┴─> PtCameraUniform -> smoke render
    right-handed, finite far, WebGPU [0, 1]
```

Before editing, GitNexus reported `OrbitCamera::view_matrix` as CRITICAL (five direct callers, eight affected symbols, 16 flows), `OrbitCamera::projection_matrix` as CRITICAL (four direct callers, eight affected symbols, 16 flows), and `gpu_pt_smoke` as LOW (no callers). For the follow-up, GitNexus reported `cpu_pick` and `screen_ray` as LOW; its zero direct-caller result missed the two source-visible App calls to `cpu_pick` (`src/app/treemap_view.rs:275,365`), so the source callpath was also reviewed. The high-risk scope was reported to the user before the source change. The substitutions preserve inputs, return type, handedness, and the two distinct depth mappings. This is a static/API equivalence claim; it does not establish visual parity.

A workspace Rust-source search on the edited tree found zero remaining `Mat4::look_at_rh`, `Mat4::perspective_rh`, or `Mat4::perspective_infinite_reverse_rh` occurrences across 126 searched Rust files. This search covers those three names, not every deprecated API in the workspace.

## Verification record

- Source and diff review: the four camera call sites and the CPU picking diff were checked against their prior forms. The render agent checked right-handed and WebGPU `[0, 1]` semantics. `src/cli_test.rs` was formatted; targeted `rustfmt --check` and `git diff --check` passed after the picking edit.
- Compiler warnings: `cargo check --workspace --locked` exited 0 in 1m35s after the API replacements, with no `warning:` or `error:` in complete stderr. After the CPU picking fix, the final incremental workspace check exited 0 in 4.29s; stderr contained only `Checking render-3d`, `Checking squarebob-rs`, and `Finished`, with no warnings or errors. No tests or application run were performed.
- Runtime or visual verification: not performed in this focused pass.
- GitNexus: final reindex reported 5,728 nodes and 13,176 relationships. `detect-changes --scope all` exited 0 with 12 changed symbols, three affected symbols, five files, and MEDIUM risk. The staged change gate runs after this report is added.
- Commit and push: authorized for `main`; the final chat records the SHA and remote confirmation.

## Follow-up boundary

The wider scan, export, rendering-parity, and workspace-audit work remains in [plan12.md](plan12.md#remaining-review-and-implementation-work) and [plan11.md](plan11.md#coverage-boundary-and-remaining-audit). This pass addresses the four confirmed `glam` deprecations without changing the camera projection contracts and corrects CPU fallback picking to follow the reversed-Z contract.
