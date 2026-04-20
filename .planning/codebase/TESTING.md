# Testing

**Analysis Date:** 2026-04-20

## Framework

- **Built-in `#[test]`** — Standard Rust tests, no separate harness crate at workspace root.

## Where Tests Exist

Sparse unit tests embedded in library crates, for example:

- `crates/bvh-gpu/src/bvh_gpu/mod.rs` — `mod tests` with `#[test]` cases.
- `crates/render-shared/src/lib.rs` — `mod tests` with at least one `#[test]`.

**Grep snapshot:** Very few `#[test]` occurrences workspace-wide compared to codebase size — most logic is validated manually or via running the app.

## Gaps

- **No `.github/workflows/`** in repo at mapping time — CI may live elsewhere or be absent.
- **Integration / E2E** — No automated headless GUI or screenshot diff pipeline visible in-tree.
- **Golden images** — Optional CLI screenshot flags in `src/main.rs` support manual/regression capture but are not wired to a test runner by default.

## Recommendations (Planning)

- Add **`cargo test --workspace`** to any future CI.
- Prefer **deterministic pure tests** on `dirstat-core` / `treemap` layout math before GPU-heavy crates.
- For renderer invariants, consider **wgpu validation** + small buffer tests in `bvh-gpu` / `render-shared` first.

---

*Testing analysis: 2026-04-20*
