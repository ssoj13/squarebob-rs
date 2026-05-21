# ACES + Color Pipeline — Settings Section (planned)

**Status:** TODO (task #7). Tracked here so the design survives across compactions and sprints.

**Goal:** Replace the current blit-time single-shader ACES with a full,
configurable display-pipeline section under `Settings → Color` (sits
directly **under** `Settings → Render`, mirroring the visual weight
of `Settings → Denoiser`). User can:

1. Choose working color space (Linear sRGB / ACEScg / ACES2065-1).
2. Pick a tonemap profile (None / Linear / Reinhard / ACES Filmic / ACES Full).
3. When **ACES Full** is selected: explicitly choose IDT / LMT / RRT / ODT
   from dropdowns (each lane independent, each with a "Default" entry that
   binds to whatever the working space + display preset implies).
4. Adjust exposure (EV stops), white balance (Kelvin), and gamut-compress
   strength as auxiliary post controls.
5. Round-trip the whole config into the existing autosave preset.

---

## Why now

- vfx-rs upstream gives us composable RRT/ODT matrices + 1D LUTs without
  vendoring ACES CTL.
- Current blit shader collapses RRT+ODT into one composite — fine for
  "tonemap on / off" but blocks per-component swap (e.g. Rec.709 vs P3-D65
  display, or LMT injection).
- `blit_uniform_buffer` already has reserved lanes (see Phase-4 notes) for
  per-frame tonemap params; switching to multi-stage doesn't need a buffer
  resize.

## Pipeline (scene-linear → display)

```
PT accumulator (scene-linear RGB, AP0/AP1 ambiguous today)
        │
        ▼
   [IDT]  ───────► ACES2065-1 (AP0)  (none / sRGB / Rec.709 / passthrough)
        │
        ▼
   [LMT]  ───────► ACES2065-1 (AP0)  (none / Neutral / Punchy / custom)
        │
        ▼
   [RRT]  ───────► OCES               (RRT v1.0 / RRT.a1.1 / off)
        │
        ▼
   [ODT]  ───────► display-referred   (sRGB-100nits / Rec.709 / Rec.2020 /
                                       P3-D65 / DCI-P3 / sRGB-HDR-sim)
        │
        ▼
  egui texture / PNG
```

**Bypass shortcuts:**
- `Tonemap = None`         → all stages no-op, clamp to [0,1] only
- `Tonemap = Linear`       → exposure + WB only, no curve
- `Tonemap = Reinhard`     → keep current `x / (1 + x)` path, ignore ACES lanes
- `Tonemap = ACES Filmic`  → keep current single-shader Narkowicz fit (default for
                              backwards compat — must remain the default!)
- `Tonemap = ACES Full`    → unlock IDT/LMT/RRT/ODT lanes

## UI layout (mirrors `ui_settings_denoiser`)

```
Settings
├── Render             (existing)
├── Color              (NEW)
│   ┌─────────────────────────────────────────────────────────────┐
│   │ Working space:  [Linear sRGB ▼]                              │
│   │ Tonemap:        [ACES Filmic ▼]                              │
│   │                                                              │
│   │ ── ACES Full chain ─────────────────── (greyed unless Full) │
│   │ IDT:            [sRGB → AP1 ▼]                              │
│   │ LMT (look):     [None ▼]                                    │
│   │ RRT:            [Standard ▼]                                │
│   │ ODT:            [sRGB / 100 nits ▼]                         │
│   │                                                              │
│   │ Exposure (EV):  [───●───] -8.0 ... +8.0                     │
│   │ White balance:  [───●───] 3200 ... 10000 K                  │
│   │ Gamut compress: [───●───] 0 ... 1   ☐ Auto                  │
│   │                                                              │
│   │ ● Status: ACES Full @ sRGB 100nits (0.4 ms / frame)         │
│   └─────────────────────────────────────────────────────────────┘
├── Denoiser           (existing)
├── Appearance
├── View
└── ...
```

- Section header uses the same `tinted_section` helper as denoiser.
- `settings_grid` for the dropdown rows; sliders for the analog lanes.
- All controls funnel through `dirty.preset()` only — they're post-process
  hyperparams, must **never** restart PT accumulation. Mirror the lesson
  from the denoise-interval bug fix.
- "Status" line colour-codes:
    - green : ACES Full active, frame budget within target
    - amber : ACES Full but stale (waiting for first frame after switch)
    - weak  : bypass mode

## State (new fields on `Render3DOptions`)

```rust
pub enum ColorWorkingSpace { LinearSRGB, ACEScg, ACES2065_1 }
pub enum TonemapKind        { None, Linear, Reinhard, AcesFilmic, AcesFull }
pub enum AcesIdt            { None, SrgbToAp1, Rec709ToAp1, Ap1Passthrough }
pub enum AcesLmt            { None, Neutral, Punchy, /* user-supplied 1D LUT later */ }
pub enum AcesRrt            { Standard, A1_1, Off }
pub enum AcesOdt            { Srgb100nits, Rec709, Rec2020_1000nits, P3D65, DciP3, SrgbHdrSim }

pub struct ColorPipelineParams {
    pub working:          ColorWorkingSpace,   // default LinearSRGB
    pub tonemap:          TonemapKind,         // default AcesFilmic  (current behaviour)
    pub idt:              AcesIdt,             // default SrgbToAp1
    pub lmt:              AcesLmt,             // default None
    pub rrt:              AcesRrt,             // default Standard
    pub odt:              AcesOdt,             // default Srgb100nits
    pub exposure_ev:      f32,                 // default 0.0
    pub white_balance_k:  f32,                 // default 6500.0
    pub gamut_compress:   f32,                 // default 0.0
    pub gamut_compress_auto: bool,             // default true (Rec.709/sRGB only)
}
```

All fields autosaved via existing preset round-trip.

## Wiring

1. `blit_uniform_buffer` gains a new lane group (`ColorPipelineGpu`,
   `#[repr(C)]`, `bytemuck::Pod`). Reuses currently-reserved padding.
2. `blit.wgsl` gets a `switch` on `tonemap_kind`:
    - `None / Linear / Reinhard / AcesFilmic` → existing fast paths.
    - `AcesFull` → branchless multiply by precomputed 3×3 (IDT∘LMT∘RRT∘ODT_matrix)
      + optional 1D LUT sample for the ODT shaper. CPU bakes the matrix
      product each time any of the four lanes change; GPU consumes a single
      `mat3` + LUT.
3. vfx-rs feeds the canonical matrices for each `(IDT, LMT, RRT, ODT)`
   combination. Where vfx-rs has a 1D shaper LUT (e.g. ODT Rec.2020 1000nits),
   we upload a tiny 32-entry `texture_1d<f32, R32Float>` and sample with
   linear filter.
4. UI dispatcher in `src/app/settings/mod.rs`:
   `if dirty.preset() && color_pipeline_changed { blit_renderer.mark_color_dirty(); }`
   — no PT reset, no layout rebuild.
5. PNG screenshot path (`capture_viewport`) must apply the same pipeline so
   what-you-see-is-what-you-save.

## Implementation phases

| Phase | Scope | LoC | Risk |
|---|---|---|---|
| C-1 | New `Settings → Color` section + `ColorPipelineParams` + preset autosave round-trip (still wired to current single-shader ACES, dropdowns disabled) | ~250 | trivial — additive UI only |
| C-2 | Extend `blit.wgsl` with `tonemap_kind` switch, plumb `ColorPipelineGpu` lane, keep default = `AcesFilmic` so no behaviour change | ~200 | low — guarded by default |
| C-3 | Wire vfx-rs, bake (IDT∘LMT∘RRT∘ODT) on CPU, push as `mat3 + lut1d` | ~300 | medium — depends on vfx-rs API surface |
| C-4 | Enable IDT/LMT/RRT/ODT dropdowns once C-3 lands; ship with `AcesFull` as opt-in | ~80 | low |
| C-5 | Gamut compression (Nuke-style) as a separate WGSL pass between RRT and ODT | ~150 | medium |
| C-6 | HDR display surface (Rec.2020 1000nits / sRGB HDR sim) — depends on wgpu surface format negotiation | ~200 | high — out of scope for first cut |

First cut = C-1 + C-2 + C-4. Total ~530 LoC, no behavioural regression
(default tonemap = AcesFilmic = current code path).

## Open questions

- Do we expose the working space as a real setting or pin it to `Linear sRGB`
  until PT actually outputs AP1? Today PT writes plain linear RGB; switching
  to AP1 inside PT (proper IDT-at-input) is a separate, larger change.
- Should LMT have a "load CDL" entry from disk? Probably C-7+.
- Does egui surface format dictate ODT? Yes — selecting `sRGB / 100 nits`
  while the swapchain is `Rgba8UnormSrgb` means we must skip the final
  OETF (or wgpu will double-apply it). Need a runtime check.

## References

- ACES github — `aces-dev` repo, `transforms/ctl/`.
- Stephen Hill's `aces-fitted.glsl` for the Narkowicz approximation we
  already ship (the `AcesFilmic` default).
- vfx-rs (TBD upstream link — pin commit once integrated).
- Nuke gamut compression — Algorithm by Nick Shaw & Daniel Brylka.
