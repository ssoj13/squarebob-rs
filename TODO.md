# Path Tracing Quality TODO

## High impact

- [x] Emissive direct light sampling for cube lights
  - Build a GPU light list from emissive cube instances.
  - Sample one emissive cube per surface hit.
  - Suppress duplicate non-transmission BSDF light hits after NEE.
  - Add right-panel controls for enable, samples per hit, and minimum light weight.

- [x] Full MIS for emissive cube hits
  - Track previous BSDF PDF and light PDF for emissive-hit weighting.
  - Re-enable weighted BSDF light hits once the PDF bookkeeping is correct.

- [x] ReSTIR DI for many emissive cubes
  - Reuse the emissive light list as candidate source input.
  - Added emissive cube candidates to the ReSTIR initial pass with shadow visibility checks.
  - ReSTIR remains wavefront-only; the UI marks that scope.

- [ ] Denoising (paused)
  - Add normal/depth/albedo buffers.
  - Implement a-trous/SVGF style filtering with variance guidance.
  - Paused for now; finish non-denoiser fixes first.

## Medium impact

- [x] Low-discrepancy sampling
  - Added R2 low-discrepancy pixel jitter with per-pixel scrambling.
  - Kept PCG fallback mode for debugging.
  - Added right-panel sampler control and persisted setting.

- [x] Adaptive sampling polish
  - Use luminance variance instead of raw RGB average variance.
  - Keep burn-in and rolling refinement budget so pixels do not freeze too early.
  - Show effective adaptive cap in the UI.

- [x] Firefly filtering
  - Use per-sample luminance clamp instead of clamping accumulated radiance.
  - Consider percentile-based clamping once readback/debug tooling exists.

## Audit

- [x] Environment MIS/PDF audit
  - Verified lat-long CDF is sin(theta)-weighted and PDF converts pixel PMF to solid angle.
  - Kept environment MIS weight path connected to the actual BSDF PDF.

- [x] Material lobe PDF audit
  - Use actual diffuse/specular lobe probabilities for environment and emissive MIS PDFs.
  - Keep transmission paths out of direct-light MIS for opaque-only NEE.

- [x] Wavefront parity
  - Mark megakernel-only controls in the right panel while wavefront is enabled.
