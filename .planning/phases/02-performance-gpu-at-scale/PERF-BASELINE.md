# Performance baseline — Phase 2 (02-01)

**Date:** 2026-04-19  
**Toolchain:** Rust `rustc 1.95.0 (59807616e 2026-04-14)` via `rust-toolchain.toml` channel `1.95.0`  
**Cargo:** `cargo 1.95.0 (f2d3ce0bd 2026-03-21)`  
**Profile for measurements:** `--release` (`opt-level = 3` in root `Cargo.toml`)

## Environment

| Field | Value |
|--------|--------|
| OS | Windows 10/11 (primary dev target for this repo) |
| GPU | Vendor-specific (integrated or discrete — record in your ETW / GPU capture) |
| Scenario | **(A)** Load a **large cached scan** from the app cache (`directories` project cache dir) after a full scan, or **(B)** point the app at a tree with ~10⁶+ small files on a fast local disk |

В репозитории **нет** crate `tracing`; временные `tracing::span` в код не добавлялись. Измерения CPU — внешний sampling (ниже).

## Measurement commands

1. **Сборка release**
   ```bash
   cargo build --release
   ```
2. **Запуск бинарника**
   ```bash
   cargo run --release
   ```
3. **CPU sampling (Windows)**  
   - Performance Recorder / WPA (ETW) с профилем CPU Sampling, процесс `dirstat-rs`, 30–60 с интерактива (pan/zoom treemap, смена фильтров).  
   - Либо **Visual Studio** → Performance Profiler → CPU Usage.  
   - На Linux/macOS: `cargo flamegraph --bin dirstat-rs` (при наличии `perf` / `dtrace`) — см. `02-RESEARCH.md`.
4. **GPU**  
   - При узком месте в 2D: **RenderDoc** или тайминги кадра (present / queue submit) для `wgpu`.  
   - Для 3D/PT: те же инструменты на проходе `render-3d` / `pt-*`.

5. **Регрессия до изменений**
   ```bash
   cargo test
   ```

## Hotspots

| Компонент | Метод измерения | Вывод (гипотезы для проверки flamegraph/ETW) |
|-----------|-----------------|-----------------------------------------------|
| **filters + `App::rebuild_filtered_tree` / `rebuild_display_tree`** | ETW CPU stacks при смене Min/Max, LoD merge, масок расширений | Рекурсивные обходы `DirEntry`, `merge_tree_by_size_range`, `filter_tree`; лишние аллокации строк при матчах масок (см. оптимизацию ASCII-пути в `matches_any_mask`). |
| **treemap layout (`treemap::layout`, squarify / KDirStat)** | CPU samples в том же окне после фильтра | Плотные проходы по дереву и сортировки детей по площади; на огромных каталогах доминирует работа после подготовки дерева. |
| **wgpu 2D (`GpuRenderer2D::render`)** | GPU/CPU boundary, частота `create_buffer` / submit | Каждый кадр создавался новый instance buffer; целевое улучшение — переиспользование буфера и `queue::write_buffer` (план 02-02). |

Дополнительно (по архитектуре): **3D / path trace** (`render-3d`, `pt-*`, `bvh-gpu`) стоят отдельного профиля при включённом 3D/PT — не смешивать с чистым 2D treemap без смены режима.

## Next steps

1. **PERF-03 / план 02-02 — CPU:** взять подтверждённый hotspot по фильтрам/layout и внести одну измеримую правку (например снижение аллокаций в hot path маски).  
2. **PERF-03 / план 02-02 — GPU:** переиспользование instance buffer в `crates/treemap/src/wgpu.rs` при непустом списке rects.  
3. **QA-01:** после правок — `cargo test`, `cargo build --release`, ручной смоук по чеклисту в `PERF-CHANGES.md`.
