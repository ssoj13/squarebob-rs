---
phase: 2
slug: performance-gpu-at-scale
status: draft
nyquist_compliant: true
wave_0_complete: true
created: 2026-04-19
---

# Phase 2 — Validation Strategy

## Test infrastructure

| Property | Value |
|----------|-------|
| **Framework** | Rust built-in + cargo |
| **Quick run** | `cargo test` |
| **Full suite** | `cargo test` + `cargo build --release` |
| **Manual** | Smokey checklist in plan 02-02 |

## Sampling

- После каждой значимой задачи: `cargo test`
- Перед merge: `cargo build --release`, смоук 2D/3D/кэш/LoD

## Per-task verification

| Task | Plan | Requirement | Automated | Status |
|------|------|-------------|-----------|--------|
| Baseline report | 02-01 | PERF-02 | doc + commands | pending |
| GPU/CPU fixes | 02-02 | PERF-03, QA-01 | `cargo test` | pending |

## Manual-only

| Behavior | Why |
|----------|-----|
| Плавность UI на 1M+ файлов | Нужен локальный диск и визуальная оценка |

## Sign-off

- [ ] Все задачи имеют проверяемые критерии в PLAN.md
- [ ] `nyquist_compliant: true` после исполнения
