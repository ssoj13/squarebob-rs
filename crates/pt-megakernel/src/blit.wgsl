// Blit shader: copy path tracer output texture to screen.
// Uses a fullscreen triangle with tone mapping.
// Uses textureLoad instead of textureSample (Rgba32Float is non-filterable).

@group(0) @binding(0) var pt_texture: texture_2d<f32>;
@group(0) @binding(1) var pt_sampler: sampler;

// Display-pipeline parameters pushed each frame from CPU. Layout
// mirrors `BlitParamsGpu` on the Rust side (compute.rs).
//
// exposure.x = physical-camera exposure multiplier (1.0 = passthrough)
// exposure.yzw = reserved
//
// color.x = tonemap kind (u32 bitcast into f32; see TonemapKind::gpu_tag):
//             0=None, 1=Linear, 2=Reinhard, 3=AcesFilmic, 4=AcesFull.
//           Default branch of the switch is AcesFilmic so the legacy
//           code path is bit-exact when CPU writes 3.
// color.y = display-side exposure in EV stops (additive, on top of x)
// color.z = white-balance Kelvin / 6500 (normalised so 1.0 = neutral)
// color.w = gamut-compress strength [0,1] (0.0 = bypass)
struct BlitParams {
    exposure: vec4<f32>,
    color:    vec4<f32>,
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
    let kind = u32(blit_params.color.x);
    var mapped: vec3<f32>;
    switch kind {
        case 0u: { mapped = saturate(scene); }                  // None
        case 1u: { mapped = saturate(scene); }                  // Linear (curve-less)
        case 2u: { mapped = reinhard(scene); }                  // Reinhard
        case 4u: { mapped = aces_filmic(scene); }               // AcesFull (C-3 will swap)
        default: { mapped = aces_filmic(scene); }               // AcesFilmic (default)
    }

    // Stage 5 — sRGB OETF (linear -> display). Skip when tonemap is
    // `None` (clamp-only debug mode); keep the legacy 1/2.2 approximation
    // everywhere else for behavioural continuity.
    if kind == 0u {
        return vec4<f32>(mapped, 1.0);
    }
    let display = pow(mapped, vec3<f32>(1.0 / 2.2));
    return vec4<f32>(display, 1.0);
}
