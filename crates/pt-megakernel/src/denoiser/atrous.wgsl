// À-trous edge-aware denoiser for path-tracer output.
//
// Reference: "Edge-Avoiding À-Trous Wavelet Transform for fast Global
// Illumination Filtering" (Dammertz, Sewtz, Hanika, Lensch 2010).
//
// This is the MVP variant: COLOR-only edge stopping (no G-buffer
// guidance). Each iteration runs a 5x5 cubic kernel at increasing
// stride (1, 2, 4, 8, 16). Color similarity controls the edge stop —
// pixels whose colors differ a lot from the centre contribute less.
//
// Future work (Stage D.2.b): add G-buffer guidance (normal + depth)
// to preserve edges that the wavefront/gbuffer.wgsl already produces.
// Hooks for it: extend `Params` with sigma_normal/sigma_depth and add
// extra texture bindings; the kernel structure here stays the same.

struct Params {
    width: u32,
    height: u32,
    stride: u32,            // 1, 2, 4, 8, ... — doubles per iteration
    sigma_color_inv: f32,   // 1.0 / (sigma_color^2) — precomputed
}

@group(0) @binding(0) var input_tex: texture_2d<f32>;
@group(0) @binding(1) var output_tex: texture_storage_2d<rgba32float, write>;
@group(0) @binding(2) var<uniform> params: Params;

// 5-tap cubic B-spline kernel: (1, 4, 6, 4, 1) / 16
fn kernel_weight(i: i32) -> f32 {
    if (i == 0) { return 6.0 / 16.0; }
    if (i == 1 || i == -1) { return 4.0 / 16.0; }
    return 1.0 / 16.0;  // i == ±2
}

@compute @workgroup_size(8, 8)
fn atrous(@builtin(global_invocation_id) gid: vec3<u32>) {
    if (gid.x >= params.width || gid.y >= params.height) {
        return;
    }

    let center_pos = vec2<i32>(i32(gid.x), i32(gid.y));
    let center = textureLoad(input_tex, center_pos, 0);
    let center_color = center.rgb;

    var sum = vec3<f32>(0.0);
    var weight_sum = 0.0;

    let stride = i32(params.stride);
    let max_x = i32(params.width) - 1;
    let max_y = i32(params.height) - 1;

    for (var dy: i32 = -2; dy <= 2; dy = dy + 1) {
        for (var dx: i32 = -2; dx <= 2; dx = dx + 1) {
            // Sample at distance `stride * (dx, dy)` — the à-trous trick
            // that exponentially expands the filter footprint per iteration
            // while the kernel weight stays the same.
            var sample_pos = center_pos + vec2<i32>(dx * stride, dy * stride);
            // Edge clamp (replicate). Mirror would also work; clamp is cheaper.
            sample_pos.x = clamp(sample_pos.x, 0, max_x);
            sample_pos.y = clamp(sample_pos.y, 0, max_y);

            let sample_color = textureLoad(input_tex, sample_pos, 0).rgb;

            // Edge-stop: gaussian on color difference.
            let diff = sample_color - center_color;
            let dist_sq = dot(diff, diff);
            let w_color = exp(-dist_sq * params.sigma_color_inv);

            // Spatial à-trous kernel weight.
            let w_spatial = kernel_weight(dx) * kernel_weight(dy);

            let w = w_spatial * w_color;
            sum = sum + sample_color * w;
            weight_sum = weight_sum + w;
        }
    }

    // Avoid divide-by-zero when all neighbours got zero weight (extreme
    // edge stopping). Fall back to centre colour so output is sensible.
    let denoised = select(
        center_color,
        sum / weight_sum,
        weight_sum > 1e-6
    );

    textureStore(output_tex, center_pos, vec4<f32>(denoised, center.a));
}
