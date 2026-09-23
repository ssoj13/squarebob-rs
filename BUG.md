# BUG

Known defects and follow-ups found from other repos. Newest first; `[ ]` open, `[x]` done.

## 2026-09-23 Vendored BSDF copies

Found 2026-09-23 by the ofx-rs Standard Surface survey (`ofx-rs/.superpowers/sdd/ss-existing-survey.md`). The canonical shared BSDF is the new render-rs crate `standard-surface-bsdf` (plan: `ofx-rs/docs/superpowers/plans/2026-09-23-standard-surface-bsdf.md`); fidelity reference is MaterialX 1.39.5 in `vfx.ref`.

- [ ] **`crates/standard-surface` is a stripped vendored copy** of render-rs `standard-surface` (shaders byte-identical,
  no wgpu/pipeline/`material_ext.rs`, wgpu-29-era deps). Re-vendor from, or depend on, render-rs
  (`standard-surface-bsdf`) once it lands.
- [ ] **Diverged PT BSDF forks**: `crates/pt-megakernel/src/bvh_traverse.wgsl:523-543,766`,
  `restir/common.wgsl:155-176`, `render-3d/shaders/cube_pbr.wgsl:127-146` (Schlick/NDF sampling copies). Rebase onto
  the shared BSDF with render-rs's megakernel (render-rs BUG.md).
