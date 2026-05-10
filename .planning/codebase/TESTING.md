# Testing Patterns

**Original analysis:** 2026-05-09
**Updated:** 2026-05-09 (post-sprint-2)

## TL;DR — current state of testing

The 2-sprint refactor batch tripled the test count and added CI.
**Current state: 24 unit tests across 6 files, all passing on
`cargo test --workspace`.** CI gate
(`cargo clippy --workspace --all-targets -- -D warnings`) is clean.

| File | Test count | Notes |
|------|-----------:|-------|
| `crates/pt-mats/src/lib.rs` (inline `mod tests`) | 9 | Sprint 1 (Wave C). Table-driven contract tests for `classify_path_filtered`: determinism, allow-flag gates, prob-bound saturation, light-vs-glass priority in PT, class-predicate disjointness. |
| `src/app/cli_apply.rs` (inline `mod tests`) | 5 (or so) | Sprint 1 (Stage B.4 by Agent). Round-trip CLI flag → Render3DOptions field, plus None-flags-leave-defaults sanity. |
| `crates/treemap/src/lib.rs` (inline `mod tests`) | 5 | Sprint 1 (Wave B). Squarified-layout regression tests including the cube-gaps bug, `rects_disjoint` helper test. |
| `src/app/filters.rs` (inline `mod tests`) | 3 | Pre-existing. LoD-merge: `merge_buckets_outside_range`, `merge_expanded_small_is_directory`, `count_outside_range`. **CONCERNS.md originally claimed `lod_expand` lived in `dirstat-core`; verified it's actually a field, not a function — the LoD-merge logic is `merge_tree_by_size_range` here in `filters.rs`.** |
| `crates/render-shared/src/lib.rs` (inline `mod tests`) | 2 | Pre-existing. `Render3DOptions` serde round-trips. |
| `crates/bvh-gpu/src/bvh_gpu/mod.rs` (inline `mod tests`) | (pre-existing — was 2 in original audit; current count not re-verified) | Pre-existing CPU validators of LBVH. |

**Total**: ~24 unit tests, 0 failing.

GPU-execution paths (wgpu pipelines, WGSL shaders, path tracer kernels,
treemap GPU rasterization) are **still not covered by automated tests**.
They are validated **manually** at runtime via the egui app and the
wgpu "uncaptured error" hook registered at startup.

**CI status (post-sprint-2):** `.github/workflows/ci.yml` runs
`cargo build --workspace --all-targets`,
`cargo clippy --workspace --all-targets -- -D warnings`,
`cargo test --workspace` on Linux + Windows runners on every push and
PR. `rustsec/audit-check` runs on every push and weekly via cron — this
audit job is the explicit reason why `auto-allocator = "*"` is allowed
to stay unpinned per project policy.

**Local verification footnote:** the local conda-forge toolchain is
pinned at `gcc=13` (downgraded from 15.1 on 2026-05-10 because
`libmimalloc-sys`'s build script uses the C23-removed `ATOMIC_VAR_INIT`
macro). Plain `cargo build` / `cargo test` / `cargo clippy` work
without any PATH workaround. CI runners are unaffected either way.

## Original (pre-sprint-2) state — preserved for reference

Original audit listed 3 files, 8 tests, no CI:

| File | Tests |
|------|-------|
| `src/app/filters.rs` (`mod tests` at line 530) | `merge_buckets_outside_range` (L539), `merge_expanded_small_is_directory` (L568), `count_outside_range` (L590) |
| `crates/render-shared/src/lib.rs` (inline `mod tests`) | `render_3d_options_deserialize_defaults` (L906), `render_3d_light_and_glass_counts_roundtrip` (L920), `render_3d_pt_sampler_roundtrip` (L943) |
| `crates/bvh-gpu/src/bvh_gpu/mod.rs` (inline `mod tests`) | `validate_lbvh_accepts_minimal_tree` (L1396), `validate_lbvh_rejects_cycle` (L1405) |

The "very limited automated test coverage / no CI" framing of the
original audit is no longer accurate.

## Test Framework

**Runner:**
- Built-in `cargo test` via the standard Rust test harness.
- No external runner (no `nextest`, no custom `[lib] harness = false`).

**Assertion Library:**
- Standard `assert!`, `assert_eq!`, `assert_ne!`, plus `expect("...")`
  on `Option`/`Result`. No external crate (no `pretty_assertions`, no
  `assert_matches`).

**Run Commands:**
```bash
cargo test                                  # whole workspace
cargo test -p dirstat-rs --lib              # binary crate's lib tests (filters)
cargo test -p render-shared                 # serde round-trip tests
cargo test -p bvh-gpu                       # CPU validators of LBVH
cargo build -p dirstat-rs --message-format short
cargo check --workspace
cargo clippy --workspace --all-targets      # lint baseline (per AGENTS.md)
```

## Test File Organization

**Pattern:** **inline `#[cfg(test)] mod tests`** at the bottom of the
production source file. There is no separate `tests/` directory in any
crate, and there are no doctests of note.

**Layout examples:**
```
crates/bvh-gpu/src/bvh_gpu/mod.rs        # production + inline mod tests
crates/render-shared/src/lib.rs          # production + inline mod tests
src/app/filters.rs                       # production + inline mod tests
```

**Conventional skeleton observed in all three files:**
```rust
#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    fn file(name: &str, path: PathBuf, size: u64) -> DirEntry {
        DirEntry::new_file(name.to_string(), path, size, "txt".into(), None)
    }

    #[test]
    fn merge_buckets_outside_range() {
        // arrange — build small in-memory tree
        // act     — call function under test
        // assert  — assert_eq! / assert!
    }
}
```
(`src/app/filters.rs:530-599`.)

**Naming:**
- Test functions use `snake_case` and read like a sentence describing
  the property under test:
  `render_3d_options_deserialize_defaults`,
  `render_3d_light_and_glass_counts_roundtrip`,
  `validate_lbvh_rejects_cycle`,
  `merge_expanded_small_is_directory`.

## Test Structure

**Suite organization:**
- Tests live in the same crate as the code they exercise (white-box,
  access to private items via `use super::*;` and `pub(super)`).
- No test fixtures on disk — every test builds the input value
  in-process. The `data/` directory holds asset files
  (`uffizi-large.hdr`, `screenshot1.jpg`, `screenshot2.jpg`, `LICENSE`)
  for the running app, **not** for tests.

**Patterns:**
- **Local builder helpers** are defined inside `mod tests` (e.g. the
  `file(...)` helper in `src/app/filters.rs:535`) instead of pulling in
  a fixtures crate.
- **Plain `assert_eq!` over Vec-of-names** is the typical assertion
  shape:
  ```rust
  let names: Vec<_> = merged.children.iter().map(|c| c.name.as_str()).collect();
  assert!(names.iter().any(|n| n.contains("below")));
  ```
  (`src/app/filters.rs:556-558`).
- **`.expect("lod small")`** with a descriptive message documents
  invariants when unwrapping the search result
  (`src/app/filters.rs:564`).
- **Serde round-trip tests** pass a JSON literal through
  `serde_json::from_str` / `to_string` and compare nested fields
  (`crates/render-shared/src/lib.rs:906-...`).

## Mocking

There is **no mocking framework** in use (no `mockall`, no `faux`).

- CPU-side tests construct concrete inputs (`DirEntry`, `LbvhNode`,
  `Render3DOptions { ... }`) directly. None of them require trait
  doubles.
- GPU code is not mocked — `wgpu::Device` / `wgpu::Queue` are real
  handles obtained at app startup, never stubbed in tests.

## Fixtures and Factories

- **No fixtures directory.** Inputs are built inline.
- **Factories** are minimal local helpers like
  `fn file(name, path, size) -> DirEntry`
  (`src/app/filters.rs:535-537`).
- The `bvh-gpu` test file defines a tiny `node(...)` constructor near
  the test module to make LBVH literals readable
  (`crates/bvh-gpu/src/bvh_gpu/mod.rs:1396-1410`).

## Coverage

**Requirements:** none enforced. No `coverage` job, no `tarpaulin` /
`grcov` / `llvm-cov` config.

**Practical coverage estimate:**
- `crates/dirstat-core` — **0%** automated tests (only consumed by the
  `filters` and `treemap` test paths).
- `src/app/filters.rs` — three property-style tests cover LoD bucket
  merging and counting.
- `crates/render-shared` — three serde round-trip tests cover
  `Render3DOptions` defaults and option subsets.
- `crates/bvh-gpu` — two **CPU-only** validators (`validate_lbvh_*`) on
  a tiny constructed LBVH. The actual GPU build pipeline (`morton`,
  `radix_sort`, `lbvh_build`, `aabb_compute`) is **not exercised** by
  tests.
- `crates/pt-core`, `crates/pt-megakernel`, `crates/pt-wavefront`,
  `crates/pt-mats`, `crates/render-core`, `crates/render-3d`,
  `crates/treemap`, `src/scanner.rs`, `src/scanner_ntfs.rs`,
  `src/cache.rs`, `src/exclusions.rs`, `src/path_key.rs` — **0
  `#[test]` items.**

## Test Types

**Unit tests:** the eight existing tests are all unit-scoped, validating
pure functions or serde behavior on small in-memory inputs.

**Integration tests:** **none.** No `crates/*/tests/*.rs` files exist
(verified via `crates/*/tests/**/*.rs` glob — zero matches).

**End-to-end tests:** **none.** No `cypress`, `playwright`, or
equivalent. The egui shell is exercised manually.

**GPU tests:** **none automated.** GPU correctness relies on:
- The wgpu **uncaptured error hook** registered at startup
  (`AGENTS.md`, startup diagram — `register wgpu uncaptured-error hook
  (when eframe exposes device)`). Failures surface as runtime panics /
  log entries via `env_logger` (`dirstat_rs`, `pt`, `wf`, `pg`
  targets).
- WGSL validation performed by `naga` inside `wgpu::Device::create_shader_module`
  at startup. A WGSL syntax error therefore fails fast on app launch
  (and on any CI step that runs the app), but it is not surfaced by
  `cargo test`.
- Manual runs of the path tracer (Megakernel / Wavefront backends),
  treemap GPU rasterizer (`treemap::GpuRenderer2D`), and BVH build
  (`bvh-gpu`) against representative directory trees. The known manual
  workflow is described in `AGENTS.md`.

**Why not GPU-tested in CI:** wgpu requires a real adapter
(Vulkan/DX12/Metal). CI runners typically lack one; the project does
not currently use a software adapter (`llvmpipe`, `WARP`) or a
dedicated GPU runner.

## Benchmarks

**No `criterion` harness.** No `benches/` directory exists in any crate.
There are no `[[bench]]` entries in any `Cargo.toml` and no
`#[bench]` attributes (verified workspace-wide).

If you add benchmarks, the conventional location would be
`crates/<crate>/benches/<name>.rs` with a `[[bench]]` entry plus
`criterion = "0.5"` as a `[dev-dependencies]`. The treemap layout
(`crates/treemap/src/lib.rs`), `dirstat-core::DirEntry::sort_by_size`,
and the CPU `pt-core::build_instance_bvh` are good candidates.

## CI / CD

**No GitHub Actions or other CI** is configured. The `.github/`
directory does not exist (verified via glob).

The de-facto verification flow is local-only and matches `AGENTS.md`
"Maintenance commands":
```bash
cargo check --workspace
cargo clippy --workspace --all-targets
```

## Common Patterns

**Async testing:** N/A — there is no `async` runtime in use; GPU futures
go through `pollster::block_on`. No tests `block_on` GPU work today.

**Error testing:** the LBVH validators express "rejects bad input" as a
direct boolean / `Result` assertion:
```rust
#[test]
fn validate_lbvh_rejects_cycle() {
    let lbvh = vec![node(0, -2, -1)];
    // assert validation flags the cycle
}
```
(`crates/bvh-gpu/src/bvh_gpu/mod.rs:1405-...`).

**Serde round-trip pattern:**
```rust
#[test]
fn render_3d_options_deserialize_defaults() {
    let json = "{}";
    let opts: Render3DOptions = serde_json::from_str(json).expect("deserialize");
    assert_eq!(opts.<field>, <expected default>);
}
```
(`crates/render-shared/src/lib.rs:906-918`.)

## Recommendations (gaps to address)

These are not required by current policy, but the gap matters for
refactoring confidence:

1. **`dirstat-core`** has zero direct tests. Add unit tests for
   `DirEntry::sort_by_size`, `sort_children_by_size_desc`, and
   serde round-trips.
2. **`scanner` / `scanner_ntfs` / `exclusions` / `cache` / `path_key`**
   in `src/` — these are pure-CPU and trivially testable with
   `tempfile`-backed fixtures, yet have no tests.
3. **CPU BVH build** in `pt-core::build_instance_bvh` and
   `pt-core::build_gpu_data_from_nodes` are pure functions with no
   coverage.
4. **CI:** even a Linux-only `cargo check --workspace` + `cargo clippy
   --workspace --all-targets -- -D warnings` workflow would catch
   regressions before push.

---

*Testing analysis: 2026-05-09*
