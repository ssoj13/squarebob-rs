# Roadmap: dirstat-rs

## Overview

Закрепить LoD по размеру, затем масштабировать производительность (CPU/GPU) и стабильность на огромных деревьях.

## Phases

- [x] **Phase 1: LoD по размеру** — merge вне [Min,Max], метаданные раскрытия, секция UI «LoD»
- [x] **Phase 2: Performance & GPU at scale** — профилирование, wgpu/shaders, исправления узких мест, QA (2026-04-19)

## Phase Details

### Phase 1: LoD по размеру
**Goal**: Свести число листьев тремэпа для мелких/крупных файлов с раскрытием по запросу.  
**Depends on**: Nothing  
**Requirements**: (delivered in tree; traceability в `01-lod.md`)  
**Success Criteria**:
  1. Ползунки Min/Max задают полосу «индивидуальных» файлов; вне полосы — merge в два ведра на каталог (если включено).
  2. На свёрнутом ведре хранится `LodExpandInfo`; double-click/scroll раскрывает файлы.
  3. Секция настроек названа «LoD».

**Plans**: Done (implementation + `.planning/phases/01-lod.md`)

### Phase 2: Performance & GPU at scale
**Goal**: Уверенная работа на миллионах файлов: измерить, оптимизировать hot path и wgpu, найти и исправить регрессии.  
**Depends on**: Phase 1  
**Requirements**: PERF-01 — PERF-04, QA-01  
**Success Criteria**:
  1. Есть отчёт профилирования с 1–3 главными узкими местами и следующими шагами.
  2. Внесены целевые оптимизации (CPU и/или GPU) с измеримым эффектом или обоснованным «no-op».
  3. `cargo test` и ручной смоук 2D/3D/PT/кэш проходят на целевой машине.

**Plans**: Complete — `.planning/phases/02-performance-gpu-at-scale/` (`PERF-BASELINE.md`, `PERF-CHANGES.md`, plans 02-01 / 02-02)

## Progress

| Phase | Plans Complete | Status |
|-------|----------------|--------|
| 1. LoD | — | Complete |
| 2. Performance | 2/2 | Complete |
