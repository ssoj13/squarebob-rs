# PT megakernel / path tracing — audit (2026-05-11)

Краткий обзор узких мест и идей оптимизации (код `pt-megakernel`, `render-3d` megakernel path).

## 1. Host / CPU — `set_adaptive_enabled`

**Было:** `render_path_traced` вызывал `set_adaptive_enabled` каждый кадр; в `pt-megakernel` при любом вызове в конце шли `fill_sample_map` (аллокация + полный `write_buffer`) и `rebuild_bind_group`.

**Сейчас:** ранний выход, если adaptive уже в нужном состоянии; тяжёлая работа только на переходах (вкл/выкл, смена пайплайна). См. `pt-megakernel/src/compute.rs`.

## 2. Загрузка сцены и материалы

**GPU-буферы:** `upload_scene` / `upload_scene_smart` переиспользуют STORAGE для nodes / instances / materials при неизменном размере (`write_buffer`), ребилд главного bind group и спутников (ReSTIR, pathguide, wavefront при adaptive) — только когда меняются размеры или появляются новые ресурсы.

**CPU — расширенные PT-материалы:** ветка `materialize_mode`, варианты огней и glass-mix при глобальной прозрачности раздувает список материалов и ID на сцену. Это кэшируется между кадрами: `Renderer3D.pt_expand_cache`, ключ в `crates/render-3d/src/renderer3d/material_cache.rs` (`pt_expand_cache_key` / `prepare_pt_expanded_materials`). При промахе кэша пересчёт как раньше; при попадании — переиспользуется `Arc<Vec<GpuMaterial>>` и готовый `material_ids`.

## 3. Частые обновления (обычно оправдано)

- `update_camera` + `update_view_proj` каждый кадр — нормально; для ReSTIR нужны матрицы / motion.
- `write_emissive_light_uniform` внутри `dispatch()` каждый dispatch — маленький uniform; при желании писать только при изменении полей.
- `mark_history_dirty` + сброс накопления при движении камеры — ожидаемо для temporal.

## 4. Megakernel на GPU

Один полноэкранный dispatch на сэмпл — упор в пропускную способность памяти и регистры / occupancy в `bvh_traverse.wgsl`; дальше — профилирование и WGSL.

Wavefront: батчинг `write_buffer` для тайлов уже есть; узкие места — число тайлов, `prepare_tiles`, редкие `rebuild_wavefront_bind_groups` при смене размеров.

## 5. Политика `pt_scene_dirty` vs сброс накопления

**`pt_scene_dirty`** по-прежнему означает полную перезаливку сцены (инстансы, BVH, таблица материалов на GPU), когда это необходимо.

**`pt_accum_reset`:** для изменений, которые не требуют нового upload (например, слайдер Mix в режиме path tracing — PT не использует PBR `materialize_mix`, достаточно обнулить progressive accumulation), UI вызывает `Renderer3D::mark_pt_accum_reset()` вместо `mark_pt_scene_dirty()`. Перед dispatch megakernel вызывается `reset_accumulation()` без принудительного `upload_scene`.

Дальнейшее разграничение остальных рычагов (что именно требует upload vs uniform) — по мере профилирования.

---

## Статус правок

- [x] `set_adaptive_enabled`: не ребилдить BG / не заливать sample map без изменения состояния; не вызывать `rebuild_wavefront_bind_groups` каждый кадр при выключенном adaptive (`compute.rs`).
- [x] `upload_scene` / `upload_scene_smart`: рост и переиспользование `nodes` / `instances` / `materials` STORAGE с `write_buffer`, ребилд главного BG + ReSTIR + pathguide только при смене GPU-ресурсов; эмиссив — `write_texture` / `write_buffer` при достаточной ёмкости текстуры и alias-буфера; uniform эмиссии больше не пересоздаётся в `rebuild_emissive_lights` (через `write_emissive_light_uniform`, в т.ч. ReSTIR-DI поля); полный путь `upload_scene_smart` вызывает `upload_scene`.
- [x] Кэш расширенных PT-материалов (`pt_expand_cache` + `prepare_pt_expanded_materials`) — срез CPU на повторяющихся кадрах с тем же ключом.
- [x] `pt_accum_reset` / `mark_pt_accum_reset` — отдельный путь сброса накопления без полного scene dirty (в т.ч. Mix в PT из `settings/renderer.rs`).
