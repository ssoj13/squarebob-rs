# PT megakernel / path tracing — audit (2026-05-11)

Краткий обзор узких мест и идей оптимизации (код `pt-megakernel`, `render-3d` megakernel path).

## 1. Host / CPU — критично

### `set_adaptive_enabled` каждый кадр

`render_path_traced` (`crates/render-3d/src/pt/megakernel/render.rs`, ~460) вызывает `set_adaptive_enabled` **на каждом кадре**. В `pt-megakernel/src/compute.rs` при **любом** вызове в конце выполняется:

- `fill_sample_map(queue)` — аллокация `Vec` на весь кадр и полный `write_buffer` sample map;
- `rebuild_bind_group(device)` — пересборка главного bind group megakernel.

Даже когда adaptive уже включён и состояние не менялось, каждый кадр платим полным ребилдом BG и лишней заливкой буфера.

**Направление:** ранний выход, если `enabled` и наличие пайплайна не изменились; `fill_sample_map` / `rebuild_bind_group` только на переходах (вкл/выкл, первое создание пайплайна).

## 2. Загрузка сцены и материалы

Каждый `upload_scene` / ветка `upload_scene_smart` заново делает `create_buffer_init` для nodes / instances / materials и затем `rebuild_bind_group` + wavefront / ReSTIR / pathguide BG. Даже при удачном BVH refit итог всё равно идёт через `upload_scene` с **новыми** буферами — нет устойчивых GPU-буферов с `write_buffer` при неизменном размере; много аллокаций и ребиндинга.

**Материалы** (`render.rs`): при `materialize_mode` и источниках света раздувается `materials` (варианты огней, смешивание glass при глобальной прозрачности) — чистый CPU на каждый upload. Кэш стабильных `(key → material_id)` между кадрами при неизменном скане мог бы срезать работу.

## 3. Частые обновления (обычно оправдано)

- `update_camera` + `update_view_proj` каждый кадр — нормально; для ReSTIR нужны матрицы / motion.
- `write_emissive_light_uniform` внутри `dispatch()` каждый dispatch — маленький uniform; при желании писать только при изменении полей.
- `mark_history_dirty` + сброс накопления при движении камеры — ожидаемо для temporal.

## 4. Megakernel на GPU

Один полноэкранный dispatch на сэмпл — упор в пропускную способность памяти и регистры / occupancy в `bvh_traverse.wgsl`; дальше — профилирование и WGSL.

Wavefront: батчинг `write_buffer` для тайлов уже есть; узкие места — число тайлов, `prepare_tiles`, редкие `rebuild_wavefront_bind_groups` при смене размеров.

## 5. Политика `pt_scene_dirty`

Флаг помечается очень широко из UI (`renderer.rs` и др.) — любое касание рычага может триггерить полный upload. Имеет смысл различать: что требует перезаливки инстансов / BVH vs что достаточно пробросить uniform-ами.

---

## Статус правок

- [x] `set_adaptive_enabled`: не ребилдить BG / не заливать sample map без изменения состояния; не вызывать `rebuild_wavefront_bind_groups` каждый кадр при выключенном adaptive (`compute.rs`).
- [x] `upload_scene` / `upload_scene_smart`: рост и переиспользование `nodes` / `instances` / `materials` STORAGE с `write_buffer`, ребилд главного BG + ReSTIR + pathguide только при смене GPU-ресурсов; эмиссив — `write_texture` / `write_buffer` при достаточной ёмкости текстуры и alias-буфера; uniform эмиссии больше не пересоздаётся в `rebuild_emissive_lights` (через `write_emissive_light_uniform`, в т.ч. ReSTIR-DI поля); полный путь `upload_scene_smart` вызывает `upload_scene`.
