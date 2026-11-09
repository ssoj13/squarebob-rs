// Blit shader: copy path tracer output texture to screen.
// Uses a fullscreen triangle with tone mapping.
// Uses textureLoad instead of textureSample (Rgba32Float is non-filterable).
//
// Assembled by `compute.rs::make_blit_pipeline` behind `display_encode` and the OCIO lane's
// `ocio_transform` (vfx-ocio's exact WGSL for the live processor, bound at groups 1/2, or the
// encode-only stand-in when none is installed). Group 0 is this shader's own.

@group(0) @binding(0) var pt_texture: texture_2d<f32>;
@group(0) @binding(1) var pt_sampler: sampler;

// Display-pipeline parameters pushed each frame from CPU. Layout
// mirrors `BlitParamsGpu` on the Rust side (compute.rs).
//
// exposure.x = physical-camera exposure multiplier (1.0 = passthrough)
// exposure.y = legacy ACES ODT tag. Retained in ABI for preset parity;
//              SDR composition never applies transfer encoding here.
// exposure.z = ACES RRT tag (`AcesRrt::gpu_tag`).
//              0 = Standard (Stephen Hill ACES RRT+ODT fit).
//              1 = A1.1     (tighter highlight rolloff).
//              2 = Off      (skip the filmic curve — debug).
// exposure.w = 1.0 when pt_texture already holds canvas-encoded values (the OCIO CPU
//              codepath): they are passed through as they are, no exposure/tonemap/encode.
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

// Bit-exact IEEE-754 HDR input sanitizer (immune to WGSL fast-math NaN/Inf folding).
// Maps NaN and negative values to 0.0, and caps +Inf / extreme finite highlights at 65504.0
// so rational polynomial evaluation and downstream ODT/gamut matrices never leak NaN.
fn sanitize_hdr(v: vec3<f32>) -> vec3<f32> {
    let bits = bitcast<vec3<u32>>(v);
    let is_nan = (bits & vec3<u32>(0x7fffffffu)) > vec3<u32>(0x7f800000u);
    let is_neg = (bits & vec3<u32>(0x80000000u)) != vec3<u32>(0u);
    let is_pos_inf = bits == vec3<u32>(0x7f800000u);
    let clean = select(select(v, vec3<f32>(65504.0), is_pos_inf), vec3<f32>(0.0), is_nan | is_neg);
    return min(clean, vec3<f32>(65504.0));
}

// Stephen Hill's ACES RRT+ODT rational polynomial fit in AP1 space.
fn aces_rrt_fit(v: vec3<f32>) -> vec3<f32> {
    let x = sanitize_hdr(v);
    let a = x * (x + 0.0245786) - vec3<f32>(0.000090537);
    let b = x * (0.983729 * x + 0.4329510) + vec3<f32>(0.238081);
    return max(a / b, vec3<f32>(0.0));
}

// ACES 1.1-ish variant in AP1 space: input gained +10 % and output trimmed
// -7 % for a slightly tighter highlight shoulder and marginally darker midtone.
fn aces_rrt_fit_a11(v: vec3<f32>) -> vec3<f32> {
    return aces_rrt_fit(sanitize_hdr(v) * 1.10) * 0.93;
}

// ACES filmic tone mapping (Stephen Hill's canonical 3x3 matrix ACES RRT+ODT fit).
fn aces_filmic(color: vec3<f32>) -> vec3<f32> {
    let c = sanitize_hdr(color);
    // ACESInputMat (sRGB / Rec.709 -> AP1 + RRT saturation)
    let v = vec3<f32>(
        dot(vec3<f32>(0.59719, 0.35458, 0.04823), c),
        dot(vec3<f32>(0.07600, 0.90834, 0.01566), c),
        dot(vec3<f32>(0.02840, 0.13383, 0.83777), c),
    );
    // RRTAndODTFit
    let curved = aces_rrt_fit(v);
    // ACESOutputMat (AP1 + ODT saturation -> sRGB / Rec.709)
    let out = vec3<f32>(
        dot(vec3<f32>( 1.60475, -0.53108, -0.07367), curved),
        dot(vec3<f32>(-0.10208,  1.10813, -0.00605), curved),
        dot(vec3<f32>(-0.00327, -0.07276,  1.07602), curved),
    );
    return saturate(out);
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

// PQ encoding belongs to the shared egui-display final presentation pass.

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
    if blit_params.exposure.w > 0.5 {
        return vec4<f32>(raw, 1.0);
    }

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
    // AP1 RRT+ODT rational fit → display (post ODT). Uses `aces_rrt_fit`
    // directly so caller-supplied `aces_pre` / `aces_post` matrices are
    // never double-applied alongside `aces_filmic`'s built-in Rec.709↔AP1 matrices.
    let kind = u32(blit_params.color.x);
    var mapped: vec3<f32>;
    switch kind {
        case 0u: { mapped = scene; }                  // None
        case 1u: { mapped = scene; }                  // Linear (curve-less)
        case 2u: { mapped = reinhard(scene); }                  // Reinhard
        case 4u: {                                              // AcesFull (legacy)
            let working = blit_params.aces_pre * scene;
            // RRT switch — Standard / A1.1 / Off. Read in WGSL from
            // the same uniform slot the CPU writes via
            // `set_blit_rrt_tag` (see `compute.rs`).
            let rrt_tag = u32(blit_params.exposure.z);
            var curved: vec3<f32>;
            switch rrt_tag {
                case 0u: { curved = aces_rrt_fit(working);     }   // Standard
                case 1u: { curved = aces_rrt_fit_a11(working); }   // A1.1
                case 2u: { curved = saturate(sanitize_hdr(working)); }   // Off — bypass curve
                default: { curved = aces_rrt_fit(working);     }
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
            // The exact OCIO processor, already adapted to the window canvas
            // (`ColorPipeline::canvas_processor`): its output IS canvas-encoded,
            // so it is returned as is — no second encode.
            return vec4<f32>(ocio_transform(vec4<f32>(scene, 1.0)).rgb, 1.0);
        }
        default: { mapped = aces_filmic(scene); }               // AcesFilmic (default)
    }

    // Egui samples non-sRGB textures as extended-sRGB code values.
    // Float targets retain values above reference white; egui-display owns
    // the final SDR/scRGB/HDR10 transfer and primary conversion.
    return vec4<f32>(display_encode(mapped), 1.0);
}
