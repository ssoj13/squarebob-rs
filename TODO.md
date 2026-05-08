# Path Tracing Quality TODO

## High impact

- [x] Emissive direct light sampling for cube lights
  - Build a GPU light list from emissive cube instances.
  - Sample one emissive cube per surface hit.
  - Suppress duplicate non-transmission BSDF light hits after NEE.
  - Add right-panel controls for enable, samples per hit, and minimum light weight.

- [ ] Full MIS for emissive cube hits
  - Track previous BSDF PDF and light PDF for emissive-hit weighting.
  - Re-enable weighted BSDF light hits once the PDF bookkeeping is correct.

- [ ] ReSTIR DI for many emissive cubes
  - Reuse the emissive light list as candidate source input.
  - Add temporal/spatial reuse only after the unbiased direct sampler is stable.

- [ ] Denoising
  - Add normal/depth/albedo buffers.
  - Implement a-trous/SVGF style filtering with variance guidance.

## Medium impact

- [ ] Low-discrepancy sampling
  - Replace pure PCG pixel jitter with blue-noise or Owen-scrambled Sobol.
  - Keep a fallback PCG mode for debugging.

- [ ] Adaptive sampling polish
  - Use luminance variance or confidence interval instead of raw RGB average variance.
  - Keep burn-in and rolling refinement budget so pixels do not freeze too early.
  - Show effective adaptive cap in the UI.

- [ ] Firefly filtering
  - Tune per-sample luminance clamp instead of clamping accumulated radiance.
  - Consider percentile-based clamping once readback/debug tooling exists.

## Audit

- [ ] Environment MIS/PDF audit
  - Verify lat-long PDF normalization and MIS weights.

- [ ] Material lobe PDF audit
  - Verify diffuse/specular/transmission lobe probabilities, weights, and glass edge cases.

- [ ] Wavefront parity
  - Port megakernel fixes to wavefront or clearly mark unsupported controls.
