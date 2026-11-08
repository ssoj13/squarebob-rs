<!-- gitnexus-rs:start -->
# GitNexus-rs — Code Intelligence

This project is indexed by gitnexus-rs as **squarebob-rs** (5697 symbols, 13022 relationships, 300 execution flows). Use the gitnexus-rs MCP tools to understand code, assess impact, and navigate safely.

> Call `graph_status` when freshness matters. Use `reanalyze` (incremental) or `gitnexus-rs analyze` (full). `detect_changes` does **not** re-index.

## Always Do

- **MUST run impact analysis before editing any symbol.** Before modifying a function, class, or method, run `impact({target: "symbolName", direction: "upstream"})` and report the blast radius (direct callers, affected processes, risk level) to the user.
- **MUST run `detect_changes({scope: "all"})` before committing** to verify your changes only affect expected symbols and execution flows.
- **MUST warn the user** if impact analysis returns HIGH or CRITICAL risk before proceeding with edits.
- When exploring unfamiliar code, use `query({query: "concept"})` to find execution flows instead of grepping. It returns process-grouped results ranked by relevance.
- When you need full context on a specific symbol — callers, callees, which execution flows it participates in — use `context({name: "symbolName"})`.
- **Check graph freshness** with `graph_status` before trusting query/impact results on a repo you have been editing.
- **Update the graph** with `reanalyze` (incremental) or `gitnexus-rs analyze` (full). `detect_changes` does **not** re-index.
- **Uncommitted edits** while commit is fresh: `reanalyze` with `scope: "unstaged"`, or run `gitnexus-rs watch` in a terminal.

## Never Do

- NEVER edit a function, class, or method without first running `impact` on it.
- NEVER ignore HIGH or CRITICAL risk warnings from impact analysis.
- NEVER rename symbols with find-and-replace — use `rename`, which understands the call graph (it previews by default).
- NEVER commit changes without running `detect_changes` to check affected scope.

## Resources

| Resource | Use for |
|----------|---------|
| `gitnexus://repo/squarebob-rs/context` | Codebase overview |
| `gitnexus://repo/squarebob-rs/clusters` | All functional areas |
| `gitnexus://repo/squarebob-rs/processes` | All execution flows |
| `gitnexus://repo/squarebob-rs/process/{name}` | Step-by-step execution trace |

## Skills

| Task | Read this skill file |
|------|---------------------|
| Understand architecture / "How does X work?" | `.claude/skills/gitnexus-rs-exploring/SKILL.md` |
| Blast radius / "What breaks if I change X?" | `.claude/skills/gitnexus-rs-impact-analysis/SKILL.md` |
| Trace bugs / "Why is X failing?" | `.claude/skills/gitnexus-rs-debugging/SKILL.md` |
| Rename / extract / split / refactor | `.claude/skills/gitnexus-rs-refactoring/SKILL.md` |
| Tools, resources, schema reference | `.claude/skills/gitnexus-rs-guide/SKILL.md` |
| Index, status, clean, wiki CLI commands | `.claude/skills/gitnexus-rs-cli/SKILL.md` |

<!-- gitnexus-rs:end -->

## Icons (chore/widgets-icons)
- UI icons come from `egui_widgets_config::icons`; the font is installed in `src/app/mod.rs` with
  `egui_widgets_config::add_icon_font`. `tests/icons.rs` scans the app + widget crates for symbol-block glyphs.
- TODO [ ] missing in `icons::*`, still raw `egui_phosphor::regular` in `src/app/toolbar.rs` (MOON, SUN theme toggle)
  and `src/app/settings/mod.rs` (FLOPPY_DISK); drop the `egui-phosphor` dependency once they exist.

## 2026-11-08 — `render-rs@23c01af` dual-backend PT, SSOT & CUDA+OIDN interop
- `crates/render-3d/src/env_map.rs` delegates equirectangular format detection, sRGB linearization, and spherical `sin(theta)` importance-sampling CDFs to `render_env`.
- `crates/render-shared` re-exports `PtComputeBackend`, `PtSamplerMode`, `AdaptivePreset`, `SpectralMode`, `OidnModeOption`, `OidnQualityOption`, and `DenoiserSettings` from `render_settings` (`pt_compute_backend` defaults to `Auto`).
- `crates/render-3d` enforces strict no-silent-fallback for `PtComputeBackend::Cuda` (reporting actual backend via `Renderer3D::active_backend()`), bridges CUDA linear beauty + on-demand primary-hit albedo/normal AOVs (`Renderer3D::sync_oidn_guides`) into `pt_denoise_oidn::OidnDenoiser` (`DenoiseInput::TexturesUntracked`), tracks full normalized `CudaAccumKey`, wires lazy `set_aovs_enabled`, zero-alloc `read_linear_into` / `read_pt_linear_into` / `readback_render_texture_into` (into `FrozenRenderSession` scratch buffers), GPU TLAS refit (`refit_scene_two_level_gpu` / `refit_two_level_instances_gpu`) on transform-only updates, `MaterialExt`, `set_emissive_sampling`, `OcioPass::new_with_adapter` (`gpu-mem` pipeline cache), and `pt_is_converged()`.
