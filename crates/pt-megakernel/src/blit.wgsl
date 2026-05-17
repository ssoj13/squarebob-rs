// Blit shader: copy path tracer output texture to screen.
// Uses a fullscreen triangle with tone mapping.
// Uses textureLoad instead of textureSample (Rgba32Float is non-filterable).

@group(0) @binding(0) var pt_texture: texture_2d<f32>;
@group(0) @binding(1) var pt_sampler: sampler;

// Exposure / display parameters pushed each frame from CPU.
//   x = scene-linear exposure multiplier (1.0 = passthrough)
//   y..w = reserved for future display-pipeline knobs (vignette,
//   chromatic aberration radius, tonemap selector index, etc.)
struct BlitParams {
    exposure: vec4<f32>,
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

// ACES filmic tone mapping.
fn aces_tonemap(color: vec3<f32>) -> vec3<f32> {
    let a = 2.51;
    let b = 0.03;
    let c = 2.43;
    let d = 0.59;
    let e = 0.14;
    return saturate((color * (a * color + b)) / (color * (c * color + d) + e));
}

@fragment
fn fs_main(in: VsOut) -> @location(0) vec4<f32> {
    let dims = textureDimensions(pt_texture);
    let px = vec2<i32>(vec2<f32>(f32(dims.x), f32(dims.y)) * in.uv);
    let color = textureLoad(pt_texture, px, 0);

    // Physical-camera exposure: scale scene-linear radiance before
    // tonemap. `exposure.x == 1.0` (Manual mode default) is bit-exact
    // passthrough.
    let exposed = color.rgb * blit_params.exposure.x;

    // Tone map HDR -> LDR
    let mapped = aces_tonemap(exposed);

    // Gamma correction (linear -> sRGB)
    let gamma = pow(mapped, vec3<f32>(1.0 / 2.2));

    return vec4<f32>(gamma, 1.0);
}
