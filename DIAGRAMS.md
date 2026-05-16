# DIAGRAMS.md

## Application Dataflow

```mermaid
flowchart TD
    CLI[CLI args] --> Main[src/main.rs]
    Main --> GpuContext[render_core::gpu::GpuContext::new]
    GpuContext --> Eframe[eframe WgpuSetup::Existing]
    GpuContext --> App[App::new]
    App --> StartScan[App::start_scan]
    StartScan --> CacheLoad[cache::load_cache]
    StartScan --> ScannerChoice{Scanner mode}
    ScannerChoice --> Jwalk[scanner::scan_bg]
    ScannerChoice --> Ntfs[scanner_ntfs::scan_ntfs_bg]
    Jwalk --> ScanMsg[ScanMsg channel]
    Ntfs --> ScanMsg
    ScanMsg --> Poll[App::poll_scan]
    Poll --> Tree[DirEntry tree]
    Tree --> CacheSave[cache::serialize_cache/write_cache_bytes]
    Tree --> Display[rebuild_display_tree]
    Display --> TreemapUI[App::ui_treemap]
    TreemapUI --> CPU2D[treemap::render CPU]
    TreemapUI --> GPU2D[GpuRenderer2D]
    TreemapUI --> Renderer3D[render_3d::Renderer3D]
    CPU2D --> EguiTexture[egui texture]
    GPU2D --> EguiTexture
    Renderer3D --> EguiTexture
```

## Shared GPU Readback Blast Radius

```mermaid
flowchart TD
    TreemapLegacy[treemap::GpuRenderer2D::render] --> ReadbackTexture[render_core::gpu::readback_texture]
    Render3DLegacy[Renderer3D::render] --> ReadbackTexture
    PTReadback[pt::megakernel::render_path_traced] --> ReadbackTexture
    Screenshot[src/app/screenshot.rs] --> Render3DLegacy
    ReadbackTexture --> MapReadback[render_core::gpu::map_readback]
    MapReadback --> MapAsync[BufferSlice::map_async]
    MapAsync --> CallbackResult[callback Result]
    CallbackResult --> Channel[std::sync::mpsc channel]
    Channel --> DoubleUnwrap[rx.recv().unwrap().unwrap]
    DoubleUnwrap --> Panic[panic on sender drop or BufferAsyncError]
```

## Scan And Cache Sequence

```mermaid
sequenceDiagram
    participant UI as App UI
    participant Cache as cache.rs
    participant Scan as scanner.rs/scanner_ntfs.rs
    participant Core as squarebob_core::DirEntry

    UI->>Cache: load_cache(scan_path)
    alt cache hit
        Cache-->>UI: CachedScan { tree }
        UI->>UI: compute stats + rebuild_display_tree
    else cache miss
        UI->>Scan: scan_bg(path, tx)
        Scan->>Core: DirEntry::new_file/new_dir
        Scan->>Scan: aggregate bottom-up + sort_by_size
        Scan-->>UI: ScanMsg::Done(tree)
        UI->>Cache: serialize_cache(scan_path, &tree)
        UI->>Cache: write_cache_bytes on background thread
        UI->>UI: rebuild_display_tree
    end
```

## Render Path Split

```mermaid
flowchart LR
    UI[App::ui_treemap] --> Callback{wgpu_render_state && gpu_context?}
    Callback -->|yes, Mode2D GPU| R2D[render_2d_callback]
    Callback -->|yes, Mode3D| R3D[render_3d_callback]
    Callback -->|no| Legacy[render_treemap legacy]

    R2D --> Gpu2D[GpuRenderer2D::render_to_texture]
    Gpu2D --> Native2D[egui_wgpu native texture]

    R3D --> RTView[Renderer3D::render_to_view]
    RTView --> Raster[Raster passes]
    RTView --> PTNoReadback[PT no-readback path]
    Raster --> Native3D[egui_wgpu native texture]
    PTNoReadback --> Native3D

    Legacy --> CPUBuffer[Vec<u8> pixels]
    CPUBuffer --> EguiUpload[egui ColorImage upload]
```

## Remaining Panic Surface

```mermaid
flowchart TD
    PanicSurface[Remaining runtime panic surface] --> MapReadback[render_core map_readback double unwrap]
    PanicSurface --> RenderState[render_state/cached_instances expects]
    PanicSurface --> LazyPT[path_tracer as_mut unwrap after lazy init]
    PanicSurface --> TreemapSlots[2D render_texture/render_view/instance_buffer unwraps]

    MapReadback --> FixA[Return Result<Vec<u8>, ReadbackError>]
    RenderState --> FixB[Central require_render_state or result-returning API]
    LazyPT --> FixC[Single ensure_path_tracer helper]
    TreemapSlots --> FixD[Bundle render target and instance resources]
```
