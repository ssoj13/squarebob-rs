// Adaptive Sampling - Variance Estimation pass.
// Uses Welford's online algorithm for running variance.

struct VarianceData {
    mean: vec3<f32>,
    _pad0: u32,
    m2: vec3<f32>,
    count: u32,
}

struct Params {
    width: u32,
    height: u32,
    _pad: vec2<u32>,
}

@group(0) @binding(0) var<storage, read> samples: array<vec4<f32>>;
@group(0) @binding(1) var<storage, read_write> variance: array<VarianceData>;
@group(0) @binding(2) var<uniform> params: Params;

@compute @workgroup_size(8, 8)
fn main(@builtin(global_invocation_id) gid: vec3<u32>) {
    if gid.x >= params.width || gid.y >= params.height { return; }

    let pixel_id = gid.y * params.width + gid.x;
    let accum = samples[pixel_id];
    if accum.w <= 0.0 {
        return;
    }
    let sample = accum.rgb / accum.w;
    var data = variance[pixel_id];

    // Welford's online algorithm
    data.count += 1u;
    let n = f32(data.count);
    let delta = sample - data.mean;
    data.mean += delta / n;
    let delta2 = sample - data.mean;
    data.m2 += delta * delta2;

    variance[pixel_id] = data;
}
