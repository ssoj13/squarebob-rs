---
plan_id: 02-02
phase: 2
status: complete
---

# Plan 02-02 — Summary

## Outcome

Целевые оптимизации по baseline:

1. **CPU:** `matches_any_mask` — стековый ASCII-lowercase для типичных имён файлов, меньше аллокаций в фильтрах.
2. **GPU / wgpu:** переиспользование instance buffer в `GpuRenderer2D::render` с `write_buffer` и перевыделением при росте.

Добавлен `PERF-CHANGES.md` с описанием и чеклистом смоука.

## Key files

- `src/app/filters.rs`
- `crates/treemap/src/wgpu.rs`
- `.planning/phases/02-performance-gpu-at-scale/PERF-CHANGES.md`

## Verification

- `cargo test --workspace` — exit 0
- `cargo build --release` — exit 0

## Self-Check: PASSED
