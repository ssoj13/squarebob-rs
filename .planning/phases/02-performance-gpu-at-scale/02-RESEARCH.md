# Phase 2 — Research

**Phase:** Performance & GPU at scale  
**Date:** 2026-04-19

## Summary

На больших деревьях доминируют: (1) построение/клонирование дерева и фильтров, (2) squarified layout в `treemap`, (3) заполнение GPU-буферов и draw calls в 2D/3D, (4) path tracing / BVH обновления. Профилирование должно быть **поэтапным**: сначала один большой каталог/кэш, затем изоляция layout vs render.

## Рекомендуемый стек измерений

| Слой | Инструмент |
|------|------------|
| CPU Rust | `cargo flamegraph`, `cargo build --release` + sampling, `tracing` spans вокруг `rebuild_filtered_tree` / layout |
| GPU | wgpu timestamp queries / `RenderDoc` / vendor tools |
| Память | `dhat-rs` или системный монитор при повторном скане |

## Риски

- **Преждевременная оптимизация** без baseline — исправляется планом 02-01.
- **Регрессия визуала** при смене батчинга — обязательный смоук 2D/3D/PT.

## Validation Architecture

Фаза не добавляет сетевых границ; приёмка — **автоматические тесты** (`cargo test`, при необходимости `cargo test --release`) и **ручной смоук** (чеклист в плане 02-02). Регрессии производительности отслеживаются **относительно baseline-отчёта** из плана 02-01, а не абсолютным числом FPS.

---

## RESEARCH COMPLETE

Достаточно для планирования исполнения фазы 2.
