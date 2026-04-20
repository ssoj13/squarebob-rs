# Phase 2 — Performance changes (02-02)

## CPU (`filters`)

- **`matches_any_mask`:** для ASCII-имён файлов (типичные расширения) обходится без выделения `String` на полный `to_lowercase()` — нижний регистр в буфер на стеке до 512 байт, затем существующий `glob_match`.

## GPU (`treemap` / wgpu)

- **`GpuRenderer2D::render`:** вместо каждого кадра `create_buffer_init` для instance data — переиспользование буфера с `VERTEX | COPY_DST` и `queue.write_buffer`, при росте числа rects — перевыделение с запасом (`next_power_of_two`).

## Expect effect

- Меньше аллокаций в hot path при фильтрации по маскам на больших деревьях.
- Меньше работы драйвера/аллокатора GPU при интерактивном pan/zoom 2D treemap с неизменным порядком величины числа прямоугольников.

## Manual smoke (operator checklist)

| Step | Result |
|------|--------|
| (1) Открыть скан (в т.ч. из кэша) | Pass |
| (2) 2D treemap pan/zoom | Pass |
| (3) 3D mode если доступен | Pass |
| (4) LoD merge + раскрыть ведро | Pass |
| (5) Загрузка из кэша | Pass |

Автоматические проверки: `cargo test` — Pass; `cargo build --release` — Pass.
