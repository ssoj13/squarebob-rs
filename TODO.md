1. get rid of separate "denoise" tab and move it to Settings tab
2. Finish the denoiser / add Intel OID
   - [x] Vendor `oidn-rs` (Burn-wgpu port), bundle 5 TZA weights (~9 MB).
   - [x] Shared `wgpu::{Instance,Adapter,Device,Queue}` across PT + Burn + eframe.
   - [x] Wavefront and megakernel both write primary-hit `albedo`/`normal` AOVs.
   - [x] `OidnDenoiser` with lazy build, graceful AOV downgrade, manual + auto triggers.
   - [x] Render-loop wire: display switches raw↔denoised, latency shown in UI.
   - [ ] **oidn-rs Phase I (deferred)**: lift `unet_runner` pre/post-process
         and `autoexposure` from CPU loops into Burn ops on shared wgpu.
         Saves ~80 ms/1080p and ~250 ms/4K of host roundtrip. Work happens
         in the `oidn-rs` repo, not here.
3. **Adaptive sampling — verify behaviour.** Current code has `sample_map`
   + `adaptive_config` plumbing (per-pixel SPP limits), but it's unclear
   whether it actually concentrates samples on noisy pixels the way V-Ray's
   noise-threshold DMC sampler does. Audit the variance feedback loop:
   `accum[pixel_id].w` (per-pixel SPP) and `variance` buffer should drive
   `sample_map` updates that early-terminate clean pixels. Probable issues
   to look at: variance reset on accumulation reset, threshold scaling vs
   `pt_samples`, and whether the WGSL `current_samples >= spp_limit` guard
   fires at the right rate.
4. **Global samples knob (unification)** — `Render3DOptions::pt_samples` is
   now the single V-Ray-style top-level number; everything else (per-dispatch
   batch, adaptive caps, OIDN trigger) is derived. Consider linking
   `adaptive_config.{min,max}_spp` to `pt_samples` proportionally so the
   user only ever touches one slider.
