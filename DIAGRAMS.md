# Application diagrams

Updated: 2026-09-23. These diagrams describe inspected source paths. See [AGENTS.md](AGENTS.md) for operating constraints, [plan11.md](plan11.md) for original findings, [plan12.md](plan12.md) for systemic repairs, [plan13.md](plan13.md) for the camera and CPU-picking follow-up, and [plan15.md](plan15.md) for the typed NTFS outcome repair.

## Scan, cache, and display dataflow

```mermaid
flowchart TD
    CLI["CLI Result: src/main.rs:19-28"] --> GPU["Shared GpuContext: src/main.rs:145-165"]
    GPU --> Eframe["eframe WgpuSetup::Existing"]
    Eframe --> App["App::new"]
    App --> Start["start_scan: scan_orchestration.rs:299-362"]
    Start --> Root["ScanRoot: display + canonical path + id"]
    Root --> Exclusions["exclusions::load: canonical ID + member validation"]
    Exclusions -->|load error| Warning["Warning shown; exclusion edits blocked"]
    Root --> CacheCmd["CacheService::load(generation)"]
    Root --> Select{"Backend"}
    Select --> Standard["run_standard -> scan_dir -> fscan_rs::scan_standard"]
    Select --> NTFS["scanner_ntfs::run_ntfs -> fscan_rs::scan_ntfs_tree_with_progress"]
    NTFS -->|BackendUnavailable; terminal warning| Standard
    Standard --> Ancestors["scan_dir: reconstruct omitted ancestor directories"]
    Ancestors -->|build returned| Build["finish_build: sort + stats"]
    NTFS -->|build returned| Build
    Standard -->|cancelled or failed| Terminal["Terminal outcome"]
    NTFS -->|Cancelled or Failed| Terminal
    Build -->|complete tree| Serialize["serialize_cache_ref in scanner worker"]
    Build -->|partial tree| Terminal
    Serialize --> Terminal
    Standard --> Progress["Progress channel"]
    NTFS --> Progress
    CacheCmd --> CacheEvent["CacheEvent::Loaded"]
    CacheEvent --> Gate["generation and root-id gate"]
    Progress --> UIStatus["Generation-gated status bar progress"]
    Terminal --> Gate
    Gate -->|preview or live result| Install["install_tree / rebuild_display_tree"]
    Gate -->|complete with serialized bytes| Store["CacheService::store"]
    Store --> Atomic["atomic_file::write"]
    Install --> UI["ui_treemap"]
```

Cache I/O is owned by an ordered worker with per-root generation watermarks (`src/cache.rs:107-124,174-245`). Only complete scans queue a cache store (`src/app/scan_orchestration.rs:237-250`). Standard traversal reconstructs parents missing from walker callbacks before assembling the partial tree (`src/scanner.rs:302,315-366,432-440`); the NTFS adapter converts the `fscan-rs` tree into owned `DirEntry` nodes (`src/scanner_ntfs.rs:62-150`).

## Scan and cache sequence

```mermaid
sequenceDiagram
    participant UI as App UI
    participant CS as CacheService
    participant SW as Scan worker
    participant FS as Filesystem
    UI->>UI: Retire old scan, advance generation, clear presentation
    UI->>UI: Load exclusions; show warning and block edits on error
    UI->>CS: Load(generation, ScanRoot)
    UI->>SW: spawn(generation, root, backend)
    CS->>FS: Read and validate flat cache
    CS-->>UI: Loaded(generation, root_id, result)
    UI->>UI: Gate by generation/root_id; optional preview
    SW->>FS: fscan-rs standard or NTFS MFT
    opt NTFS BackendUnavailable
        SW-->>UI: NtfsFallback on terminal channel
        SW->>FS: Standard scanner fallback
    end
    opt NTFS Cancelled or Failed
        Note over SW: No standard fallback
    end
    SW-->>UI: Progress (bounded channel)
    opt A build returned
        SW->>SW: Sort tree and compute stats
        opt Complete tree
            SW->>SW: Serialize cache bytes
        end
    end
    SW-->>UI: Terminal(Completed/Partial/Cancelled/Failed)
    UI->>UI: Gate and install live tree
    opt Completed with serialized bytes
        UI->>CS: Store(generation, root, bytes)
        CS->>FS: Atomic cache replacement
        CS-->>UI: Stored(result)
    end
```

## Rendering and readback branches

```mermaid
flowchart LR
    UI["ui_treemap: treemap_view.rs:20-98"] --> Select{"Shared wgpu render state?"}
    Select -->|2D CPU or no callback| Legacy["render_treemap: app/mod.rs:569"]
    Select -->|2D GPU| GPU2D["render_2d_callback: treemap_view.rs:1373"]
    Select -->|3D| GPU3D["render_3d_callback: treemap_view.rs:1034"]
    Legacy --> CPU["renderer::cpu::render or GPU readback"]
    CPU --> Upload["egui ColorImage upload"]
    GPU2D --> Native2D["GpuRenderer2D::render_to_texture"]
    Native2D --> EguiNative["egui_wgpu native texture"]
    GPU3D --> View["Renderer3D::render_to_view"]
    View --> Prepare["prepare_scene: shared native/readback setup + selection remap"]
    Prepare --> Empty{"No instances?"}
    Empty -->|yes| Clear["Clear color/object ID; reset picking/UI selection; skip OIDN"]
    Empty -->|no| Raster{"Raster or PT"}
    Clear --> EguiNative
    Raster -->|raster| Pass["raster + active object-ID picking"]
    Raster -->|PT| Trace["path tracing; current Object ID for outline; optional OIDN"]
    Pass --> EguiNative
    Trace --> EguiNative
```

Shift-drag selection keeps path identity across instance rebuilds (`src/app/treemap_view.rs:412-458,729-767`; `src/app/scan_orchestration.rs:91-111`). CPU fallback picking calls `Renderer3D::screen_ray`, which applies the camera's reversed-Z near/far convention (`crates/render-3d/src/renderer3d/cpu_pick.rs:29-58`; `crates/render-3d/src/lib.rs:1343-1378`).

```mermaid
flowchart LR
    Drag["Shift-drag starts"] --> Baseline["Capture selected paths"]
    Baseline --> Remap["Each preview/commit: paths to active instance IDs"]
    Remap --> Inside["Add instances inside current rectangle"]
    Inside --> Overlay["Refresh selection overlay"]
    NewScan["New scan"] --> Reset["Clear drag start and path baseline"]
```

```mermaid
flowchart LR
    Cursor["App fallback cursor pick"] --> Pick["Renderer3D::cpu_pick"]
    Pick --> Ray["Renderer3D::screen_ray"]
    Ray --> Near["NDC near Z = 1; far Z = 0.001"]
    Near --> Tree["pick_tree against current layout"]
```

```mermaid
flowchart TD
    T2D["2D legacy GpuRenderer2D::render"] --> Copy["render_core::gpu::readback_texture"]
    R3D["3D legacy Renderer3D::render via prepare_scene"] --> Copy
    PT["PT readback path"] --> Copy
    Copy --> Layout["TextureReadbackLayout::new: checked dimensions"]
    Layout --> Stage["TextureReadback staging buffer"]
    Stage --> Map["map_readback -> map_buffer_read"]
    Map --> Callback["map_async callback Result + channel"]
    Callback --> Reserve["try_reserve_exact output bytes"]
    Reserve --> Error{"Map, channel, or allocation failed?"}
    Error -->|yes| ResultErr["ReadbackError"]
    Error -->|no| Pixels["Strip row padding -> Vec of pixels"]
```

The older double-`unwrap` panic diagram is obsolete: `map_buffer_read` returns typed errors, and output allocation is fallible (`crates/render-core/src/lib.rs:740-805,807-839`).

## Screenshot and media codepaths

```mermaid
flowchart TD
    Args["--screenshot and delay: cli.rs"] --> Main["main.rs:19-24"]
    Main --> App["App::new / CLI apply"]
    App --> Timer["screenshot_start_time after complete scan"]
    Timer --> Maybe["handle_screenshot: app/screenshot.rs:12-77"]
    Maybe --> Capture["capture_viewport: screenshot.rs:79-157"]
    Capture -->|validated pixels| Save["save_png: screenshot.rs:162-173"]
    Capture -->|error| Failure["Retry or dismiss; no exit"]
    Save -->|error| Failure
    Save -->|success| Done["Mark screenshot taken"]
    Done --> Exit["optional viewport close"]
```

```mermaid
flowchart TD
    Comp["Comp::get_frame"] --> Choice{"Export type"}
    Choice -->|video| Video["encode_sequence_from_comp: encode.rs:1148-1280"]
    Video --> Crop["crop and conditional tonemap"]
    Crop --> Encoder["VideoEncoder::open / push / finish: video.rs"]
    Encoder --> Convert["codec conversion / encoded samples"]
    Convert --> Mux["MovWriter temporary sibling file"]
    Mux --> Commit["finalize and publish output"]
    Choice -->|images| Sequence["encode_image_sequence: encode.rs:1769"]
    Sequence --> Tone["Tonemap to target precision; U16 retains float until quantization"]
    Tone --> Writer{"PNG / TIFF / TGA / EXR / JPEG"}
    Writer -->|TIFF| Tiff["Selected TIFF compression"]
    Writer -->|TGA| Tga["Selected RLE mode"]
    Writer -->|PNG, EXR, JPEG| Frames["numbered image files"]
    Tiff --> Frames
    Tga --> Frames
```

The image-sequence writer now routes selected TIFF and TGA compression to their encoders (`crates/media-encoder/src/dialogs/encode/encode.rs:1664-1680,1718-1762`). App capture supplies RGBA8 (`src/app/image_sequence.rs:257-267`), so its U16 export cannot recover higher source precision.
