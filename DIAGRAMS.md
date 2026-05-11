# DIAGRAMS.md — dirstat-rs (Mermaid)

## Application layer

```mermaid
flowchart TB
  subgraph Entry["Binary entry"]
    MAIN["main.rs: CLI, logging, eframe"]
  end

  subgraph UI["app::App"]
    STATE["State: tree, filters, viewport, render mode"]
    CHANNEL["crossbeam ScanMsg queue"]
  end

  subgraph Data["Data sources"]
    CACHE["cache.rs load/save"]
    SCAN_J["scanner.rs jwalk"]
    SCAN_N["scanner_ntfs.rs optional"]
  end

  subgraph Model["dirstat_core"]
    DIR["DirEntry"]
  end

  MAIN --> STATE
  CACHE --> DIR
  SCAN_J --> DIR
  SCAN_N --> DIR
  SCAN_N -.->|fallback| SCAN_J
  STATE --> CHANNEL
  CHANNEL --> DIR
```

## NTFS scan fallback (Windows)

```mermaid
sequenceDiagram
    participant UI as poll_scan(scan_orchestration)
    participant NT as scanner_ntfs thread
    participant JW as scanner::scan_dir_public

    NT->>UI: Progress (zeros)
    alt MFT OK
        NT->>UI: Done(DirEntry)
    else MFT Err
        NT->>UI: NtfsFallback(reason)
        Note over UI: Updates progress.scan_engine_label + progress.error only.
        Note over UI: Does NOT mutate scanner_mode (persisted pref stays NTFS).
        NT->>JW: jwalk scan_dir_public same thread after NtfsFallback
        alt jwalk OK
            JW->>UI: Done(DirEntry)
        else jwalk Err
            JW->>UI: Error
        end
    end
```

## Display pipeline

```mermaid
flowchart LR
  T["Raw tree (scan)"]
  F["filters.rs: size / exclusion / mask / ext"]
  D["display_tree_cache"]
  L["treemap layout"]
  R2["2D CPU/GPU render"]
  R3["Renderer3D"]

  T --> F --> D --> L
  L --> R2
  L --> R3
  R2 --> E["egui texture"]
  R3 --> E
```

## Directory entity (logical)

```mermaid
classDiagram
  class DirEntry {
    +String name
    +PathBuf path
    +u64 size
    +u64 own_size
    +Vec~DirEntry~ children
    +bool is_dir
    +String ext
    +u64 file_count
    +u64 dir_count
    +Option~u64~ modified_time
    +Cell~rect~ rect
    +sort_by_size()
    +sort_children_by_size_desc()
  }
```

## Display GPU paths (actual vs fallback vs roadmap)

Treemap pane chooses paths in `src/app/treemap_view.rs` (`use_callback`:
`wgpu_render_state.is_some()` and `gpu_context.is_some()` and mode/backend).

```mermaid
flowchart TB
  subgraph ZeroCopy["Current: zero-copy (eframe-compatible device)"]
    ZC3["render_3d_callback<br/>Renderer3D + register_native_texture"]
    ZC2["render_2d_callback<br/>GpuRenderer2D + register_native_texture"]
    EG_NAT["egui textures native wgpu sampling"]
    ZC3 --> EG_NAT
    ZC2 --> EG_NAT
  end

  subgraph Fallback["Current: fallback (foreign device / CPU / screenshots)"]
    RB["Cpu readback path render_treemap()"]
    EGI["ctx.load_texture treemap_tex"]
    RB --> EGI
  end

  subgraph Roadmap["Roadmap — Stage D.2 etc."]
    DEN["PT denoiser output texture"]
    TEMP["Temporal / ReSTIR polish"]
  end

  DEN -.->|"same register_native_texture hook"| EG_NAT
  TEMP -.-> DEN
```

Readback fallback remains required when `GpuContext` is **not**
shareable with egui (`AGENTS.md` / `render_treemap` comment block).
