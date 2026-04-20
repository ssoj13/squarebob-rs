# Requirements: dirstat-rs

**Defined:** 2026-04-19  
**Core Value:** Fast visualization at scale without dropping features.

## v1 (active roadmap)

### Scale & performance

- [ ] **PERF-01**: При деревьях с миллионами файлов UI остаётся отзывчивым (layout/рендер не блокируют event loop без необходимости).
- [ ] **PERF-02**: Профилирование end-to-end: scanner → tree → filters → treemap → wgpu; узкие места задокументированы.
- [ ] **PERF-03**: Оптимизация путей wgpu (буферы, батчи, лимиты адаптера) и шейдеров без удаления режимов.
- [ ] **PERF-04**: Регрессионная проверка: 2D/3D, path tracer, кэш сканов, LoD merge/expand.

### Quality

- [ ] **QA-01**: Систематический поиск глюков и нелогичного поведения при больших данных; исправления с тестами где уместно.

## Out of Scope (this milestone)

| Item | Reason |
|------|--------|
| Новый UI-фреймворк | Остаётся egui |
| Облачная синхронизация | Вне продукта |

## Traceability

| Requirement | Phase | Status |
|-------------|-------|--------|
| PERF-* | Phase 2 | Pending |
| QA-01 | Phase 2 | Pending |
