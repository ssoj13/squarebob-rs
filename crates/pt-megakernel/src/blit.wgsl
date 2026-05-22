// Blit shader: copy path tracer output texture to screen.
// Uses a fullscreen triangle with tone mapping.
// Uses textureLoad instead of textureSample (Rgba32Float is non-filterable).

@group(0) @binding(0) var pt_texture: texture_2d<f32>;
@group(0) @binding(1) var pt_sampler: sampler;

// Baked OCIO display 3D LUT (33³ Rgba16Float) + filtering sampler.
// Populated by `PathTraceCompute::set_blit_lut_3d`. Default content is
// an identity LUT so a passthrough sample of the bind slot never
// corrupts the image even when ColorMode == BuiltIn (case 6u is the
// only branch that samples it).
@group(0) @binding(3) var lut_3d:   texture_3d<f32>;
@group(0) @binding(4) var lut_samp: sampler;

// Display-pipeline parameters pushed each frame from CPU. Layout
// mirrors `BlitParamsGpu` on the Rust side (compute.rs).
//
// exposure.x = physical-camera exposure multiplier (1.0 = passthrough)
// exposure.y = ACES ODT tag (`AcesOdt::gpu_tag`). 2 = Rec2020 1000nits
//              → PQ OETF; everything else → sRGB 1/2.2 OETF.
// exposure.z = ACES RRT tag (`AcesRrt::gpu_tag`).
//              0 = Standard (Narkowicz ACES 1.0 fit).
//              1 = A1.1     (tighter highlight rolloff).
//              2 = Off      (skip the filmic curve — debug).
// exposure.w = reserved
//
// color.x = tonemap kind (u32 bitcast into f32; see TonemapKind::gpu_tag):
//             0=None, 1=Linear, 2=Reinhard, 3=AcesFilmic, 4=AcesFull.
//           Default branch of the switch is AcesFilmic so the legacy
//           code path is bit-exact when CPU writes 3.
// color.y = display-side exposure in EV stops (additive, on top of x)
// color.z = white-balance Kelvin / 6500 (normalised so 1.0 = neutral)
// color.w = gamut-compress strength [0,1] (0.0 = bypass)
//
// aces_pre  = IDT (scene-linear → ACEScg) matrix, column-major.
// aces_post = ODT (ACEScg → display) matrix, column-major.
// Both consulted only when color.x == 4 (AcesFull). For other tonemap
// kinds the matrices are unused — CPU still writes identity at init so
// reading them in a placeholder branch is well-defined.
struct BlitParams {
    exposure:  vec4<f32>,
    color:     vec4<f32>,
    aces_pre:  mat3x3<f32>,
    aces_post: mat3x3<f32>,
}
@group(0) @binding(2) var<uniform> blit_params: BlitParams;

struct VsOut {
    @builtin(position) position: vec4<f32>,
    @location(0) uv: vec2<f32>,
};

// Fullscreen triangle (3 vertices cover entire screen).
@vertex
fn vs_main(@builtin(vertex_index) idx: u32) -> VsOut {
    var out: VsOut;
    let x = f32(i32(idx & 1u)) * 4.0 - 1.0;
    let y = f32(i32(idx >> 1u)) * 4.0 - 1.0;
    out.position = vec4<f32>(x, y, 0.0, 1.0);
    out.uv = vec2<f32>((x + 1.0) * 0.5, (1.0 - y) * 0.5);
    return out;
}

// ACES filmic tone mapping (Narkowicz 2015 fit). Matches the pre-C-2
// behaviour exactly when called with the same input.
fn aces_filmic(color: vec3<f32>) -> vec3<f32> {
    let a = 2.51;
    let b = 0.03;
    let c = 2.43;
    let d = 0.59;
    let e = 0.14;
    return saturate((color * (a * color + b)) / (color * (c * color + d) + e));
}

// ACES 1.1-ish variant: same Narkowicz fit but with input gained +10 %
// and output trimmed −7 %. Net effect is a slightly tighter highlight
// shoulder and a marginally darker midtone — the visual delta the
// ACES 1.1 RRT introduced over 1.0. Not a literal port of the Hable
// blue-light-artifact patch, but distinguishable enough that flipping
// the dropdown is a meaningful operation.
fn aces_filmic_a11(color: vec3<f32>) -> vec3<f32> {
    return aces_filmic(color * 1.10) * 0.93;
}

// AgX (Eary Chow). Modern open-source filmic transform that
// preserves hue better than ACES at the cost of some saturation.
// Three-stage pipeline:
//   1. inset matrix — Rec.709 → AgX working primaries.
//   2. log encode  — clamp + linear → [0,1] via log2 mapping.
//   3. 6th-order   — Eary Chow's "default contrast" polynomial
//      curve, then the outset matrix back to Rec.709.
//
// Visually distinct from ACES in three places: skin tones drift
// less, saturated blues stay blue (the famous ACES "blue-light
// artifact"), and highlights desaturate more gently. Coefficients
// from the published shader; matrices match the open-source AgX
// reference (Blender 4.x baseline).
fn agx_default_contrast(x: vec3<f32>) -> vec3<f32> {
    let x2 = x * x;
    let x4 = x2 * x2;
    return 15.5 * x4 * x2
        - 40.14 * x4 * x
        + 31.96 * x4
        - 6.868 * x2 * x
        + 0.4298 * x2
        + 0.1191 * x
        - vec3<f32>(0.00232);
}

fn agx_filmic(color: vec3<f32>) -> vec3<f32> {
    // Rec.709 → AgX inset primaries.
    let inset_r = vec3<f32>(0.842479, 0.042328, 0.042394);
    let inset_g = vec3<f32>(0.078398, 0.878894, 0.078399);
    let inset_b = vec3<f32>(0.079131, 0.078778, 0.879183);
    // AgX → Rec.709 outset (inverse-ish, matches the published mat).
    let outset_r = vec3<f32>( 1.196879, -0.052960, -0.052931);
    let outset_g = vec3<f32>(-0.098000,  1.151474, -0.098158);
    let outset_b = vec3<f32>(-0.099160, -0.090520,  1.151207);

    let safe = max(color, vec3<f32>(0.0));
    var c = vec3<f32>(
        dot(inset_r, safe),
        dot(inset_g, safe),
        dot(inset_b, safe),
    );

    // Log encode. min_ev/max_ev are AgX's open exposure window.
    let min_ev = -12.47393;
    let max_ev =   4.026069;
    c = log2(max(c, vec3<f32>(1.0e-10)));
    c = (c - min_ev) / (max_ev - min_ev);
    c = clamp(c, vec3<f32>(0.0), vec3<f32>(1.0));

    c = agx_default_contrast(c);

    // Outset matrix back to display primaries.
    let out = vec3<f32>(
        dot(outset_r, c),
        dot(outset_g, c),
        dot(outset_b, c),
    );
    return max(out, vec3<f32>(0.0));
}

// SMPTE ST 2084 (PQ) inverse-EOTF — encodes display-linear nits to
// the 10-bit PQ-encoded signal expected by HDR10 displays. Input is
// nits normalised to 1.0 = 10_000 nits (so an Rec.2020 1000-nit signal
// peaks at 0.1). Returns `[0,1]`.
//
// Constants are the canonical PQ parameters (m1, m2, c1, c2, c3 from
// SMPTE ST 2084:2014, also called Rec.2100 PQ).
//
// Note: at C-6 first-cut, the eframe surface is always Rgba8UnormSrgb,
// which means PQ output is wasted on an SDR framebuffer. The function
// ships now so the math is in place when a future eframe / wgpu surface
// negotiation lands; until then it's a no-op behind the `kind == 4 &&
// odt == Rec2020` runtime check.
fn pq_inverse_eotf(nits_normalised: vec3<f32>) -> vec3<f32> {
    let m1 = 0.1593017578125;       // 1305 / 8192
    let m2 = 78.84375;              // 2523 / 32 (× 32 = 78.84375)
    let c1 = 0.8359375;             // 3424 / 4096
    let c2 = 18.8515625;            // 2413 / 4096 × 32
    let c3 = 18.6875;               // 2392 / 4096 × 32
    let l  = max(nits_normalised, vec3<f32>(0.0));
    let lm1 = pow(l, vec3<f32>(m1));
    let num = c1 + c2 * lm1;
    let den = vec3<f32>(1.0) + c3 * lm1;
    return pow(num / den, vec3<f32>(m2));
}

// ACES Reference Gamut Compression (cyan/magenta/yellow asymmetric).
// Pulls samples that lie outside the display gamut back inside with a
// soft rolloff in the achromatic-distance domain. Constants are the
// canonical ACES 1.3 values published with the Reference Gamut
// Compressor (Nick Shaw / Daniel Brylka).
//
// `strength` in `[0,1]` lerps from "no compression" to "full". The
// returned RGB is still scene-referred display-gamut linear; OETF is
// applied downstream as for any other tonemap branch.
fn gamut_compress(rgb: vec3<f32>, strength: f32) -> vec3<f32> {
    if (strength <= 0.0) {
        return rgb;
    }
    // Per-channel distance limit and threshold. Limits are the maximum
    // distance from achromatic the published algorithm will compress;
    // thresholds gate where the compression starts (below threshold =
    // untouched).
    let limit     = vec3<f32>(1.147, 1.264, 1.312);
    let threshold = vec3<f32>(0.815, 0.803, 0.880);
    let power     = 1.2;

    // Achromatic = max channel. If everything is negative we have no
    // valid scaling axis — bail out.
    let achromatic = max(max(rgb.r, rgb.g), rgb.b);
    if (achromatic <= 0.0) {
        return rgb;
    }

    // Per-channel distance from achromatic, normalised.
    let dist = (vec3<f32>(achromatic) - rgb) / achromatic;

    // Soft-knee compress each channel independently.
    var compressed = dist;
    for (var i = 0; i < 3; i = i + 1) {
        let d = dist[i];
        let t = threshold[i];
        let l = limit[i];
        if (d < t) {
            continue;
        }
        // Normalised distance into the compressed region.
        let nd = (d - t) / (l - t);
        // Asymptotic compressor: nd/(1 + nd^power)^(1/power).
        let denom = pow(1.0 + pow(nd, power), 1.0 / power);
        compressed[i] = t + (l - t) * (nd / denom);
    }

    // Linear-blend toward the compressed sample by `strength`.
    let final_dist = mix(dist, compressed, vec3<f32>(strength));
    return achromatic - final_dist * achromatic;
}

// Reinhard `x / (1 + x)`. Soft rolloff, washed-out highlights.
fn reinhard(color: vec3<f32>) -> vec3<f32> {
    return color / (vec3<f32>(1.0) + color);
}

// Cheap Kelvin-tint approximation. `wb_norm` is `target_K / 6500`, so
// `1.0` is neutral, `<1` warms (more red, less blue), `>1` cools.
// Sufficient for a preview pipeline — full Planckian locus needs an
// xy → CAT02 chain (C-5 territory).
fn white_balance(color: vec3<f32>, wb_norm: f32) -> vec3<f32> {
    let t = clamp(wb_norm, 0.4, 1.6);
    let r_gain = 1.0 / t;
    let b_gain = t;
    return vec3<f32>(color.r * r_gain, color.g, color.b * b_gain);
}

@fragment
fn fs_main(in: VsOut) -> @location(0) vec4<f32> {
    let dims = textureDimensions(pt_texture);
    let px = vec2<i32>(vec2<f32>(f32(dims.x), f32(dims.y)) * in.uv);
    let raw = textureLoad(pt_texture, px, 0).rgb;

    // Stage 1 — physical-camera exposure (legacy lane, untouched). Manual
    // mode writes 1.0 so this is bit-exact passthrough.
    var scene = raw * blit_params.exposure.x;

    // Stage 2 — display-side EV stops (additive over the camera exposure).
    // Default value 0.0 → 2^0 = 1.0 = passthrough.
    let ev_mul = exp2(blit_params.color.y);
    scene = scene * ev_mul;

    // Stage 3 — white balance. Default `wb_norm = 1.0` → identity.
    scene = white_balance(scene, blit_params.color.z);

    // Stage 4 — tonemap switch. Default branch is AcesFilmic so callers
    // that haven't migrated yet (or that explicitly select kind == 3)
    // produce the same image as the pre-C-2 shader.
    //
    // AcesFull (kind == 4): scene-linear → ACEScg (pre IDT∘LMT) →
    // filmic RRT → display (post ODT). When CPU writes identity for
    // both matrices the result is algebraically equal to AcesFilmic.
    let kind = u32(blit_params.color.x);
    var mapped: vec3<f32>;
    switch kind {
        case 0u: { mapped = saturate(scene); }                  // None
        case 1u: { mapped = saturate(scene); }                  // Linear (curve-less)
        case 2u: { mapped = reinhard(scene); }                  // Reinhard
        case 4u: {                                              // AcesFull (legacy)
            let working = blit_params.aces_pre * scene;
            // RRT switch — Standard / A1.1 / Off. Read in WGSL from
            // the same uniform slot the CPU writes via
            // `set_blit_rrt_tag` (see `compute.rs`).
            let rrt_tag = u32(blit_params.exposure.z);
            var curved: vec3<f32>;
            switch rrt_tag {
                case 0u: { curved = aces_filmic(working);     }   // Standard
                case 1u: { curved = aces_filmic_a11(working); }   // A1.1
                case 2u: { curved = saturate(working);        }   // Off — bypass curve
                default: { curved = aces_filmic(working);     }
            }
            let displayed  = blit_params.aces_post * curved;
            // Gamut compression operates in display gamut, BEFORE
            // saturate — clipping first would defeat the point. The
            // strength lane is already resolved on CPU (Auto checkbox
            // -> per-ODT default).
            let compressed = gamut_compress(displayed, blit_params.color.w);
            mapped         = saturate(compressed);
        }
        case 5u: {                                              // AgX (built-in, new pipeline)
            mapped = saturate(agx_filmic(scene));
        }
        case 6u: {
            // OCIO 3D-LUT sampler. The LUT was baked from the active
            // `vfx_ocio::Processor` over `[0,1]^3` in scene-linear input
            // space and stored RGB in scan order `r + g*N + b*N²`,
            // which maps cleanly to WGPU's texel order (X fastest).
            //
            // Clamp scene to [0,1] — HDR values above 1.0 saturate to
            // the LUT's top corner. A proper HDR shaper LUT would
            // require an extra log encode pre-step; that lives behind
            // a future shaper-aware bake.
            //
            // Half-texel correction: a trilinear sample over an N³ LUT
            // centred on cell midpoints needs `(c*(N-1) + 0.5) / N`,
            // otherwise the edges interpolate against the implicit
            // out-of-bounds clamp value and the result drifts.
            //
            // Output is already display-encoded (the OCIO display
            // processor folds the OETF in) — Stage 5 skips the
            // trailing gamma pow for tag 6.
            let lut_size: f32 = 33.0;
            let lut_uvw = (saturate(scene) * (lut_size - 1.0) + 0.5) / lut_size;
            mapped = textureSample(lut_3d, lut_samp, lut_uvw).rgb;
        }
        default: { mapped = aces_filmic(scene); }               // AcesFilmic (default)
    }

    // Stage 5 — OETF (display-linear → display-encoded). Three paths:
    //
    //  * `kind == 0u` (None): clamp-only debug mode, skip OETF entirely.
    //  * ODT tag == 2 (Rec.2020 1000nits HDR): apply PQ (SMPTE ST 2084)
    //    inverse-EOTF. PT mapped value is treated as nits / 10_000,
    //    so an Rec.2020 1000-nit signal peaks at 0.1 before encoding.
    //  * Otherwise: keep the legacy 1/2.2 sRGB approximation.
    //
    // CAVEAT: today's eframe-managed surface is always Rgba8UnormSrgb.
    // The PQ branch produces mathematically correct HDR10 codewords,
    // but the 8-bit framebuffer destroys them — proper HDR output needs
    // a Rgba16Float / Rgb10a2Unorm surface plus colour-space negotiation
    // through wgpu. That surface plumbing is the remaining open work on
    // this pipeline (TaskList #8).
    if kind == 0u {
        return vec4<f32>(mapped, 1.0);
    }
    // Tag 6 (OCIO LUT): the OCIO display processor's view transform
    // already includes the output-side EOTF^-1 (display encoding), so
    // re-applying the legacy 1/2.2 here would double-encode and crush
    // midtones. Return the LUT sample as-is.
    if kind == 6u {
        return vec4<f32>(mapped, 1.0);
    }
    let odt_tag = u32(blit_params.exposure.y);
    if odt_tag == 2u {
        let pq = pq_inverse_eotf(mapped);
        return vec4<f32>(pq, 1.0);
    }
    let display = pow(mapped, vec3<f32>(1.0 / 2.2));
    return vec4<f32>(display, 1.0);
}
