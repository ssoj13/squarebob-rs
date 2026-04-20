# Phase 2: Performance & GPU at scale — Context

**Gathered:** 2026-04-19  
**Status:** Ready for planning  
**Source:** Roadmap + prior LoD work (`.planning/phases/01-lod.md`)

## Phase boundary

Измерить производительность на больших деревьях, задокументировать узкие места, внести целевые правки в CPU-путь (дерево, фильтры, layout) и GPU (wgpu, шейдеры, батчи), затем прогнать тесты и смоук без удаления фич.

## Implementation decisions

- Оставить **egui** и текущую архитектуру (`src/app/`, `crates/treemap`, `crates/render-3d`).
- Профилирование: **Windows**: ETW / встроенный sampling; **кросс-платформа**: `cargo flamegraph` / `perf` где доступно; для GPU — встроенные тайминги wgpu / GPU profiler по возможности.
- Любые оптимизации должны сохранять корректность **LoD** (`lod_expand`, `merge_tree_by_size_range`).
- Кэш сканов: при изменении `DirEntry` уже поднят `CACHE_VERSION`; новые поля — только с миграцией версии.

## Canonical references

- `.planning/codebase/ARCHITECTURE.md`
- `.planning/codebase/STACK.md`
- `.planning/phases/01-lod.md`
- `src/app/mod.rs` — `rebuild_filtered_tree`, `display_root`, render loop
- `crates/treemap/` — layout + GPU 2D
- `crates/render-3d/` — 3D + PT pipelines

## Deferred

- Распределённый скан по сети
- Облачная аналитика
