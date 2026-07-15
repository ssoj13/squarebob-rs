// Adaptive Sampling - Variance Estimation pass.
// Uses Welford's online algorithm for running variance.

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
    let accumulated = samples[pixel_id];
    let total_count = u32(accumulated.w);
    var data = variance[pixel_id];

    if total_count <= data.count {
        return;
    }

    // One wavefront dispatch emits at most one new sample per pixel. Reconstruct
    // that sample from the cumulative sum, then feed the same Welford state used
    // by the megakernel. A gap means the producer contract was violated; keep
    // sampling conservatively instead of trusting an unrecoverable variance.
    if total_count != data.count + 1u {
        data.mean = accumulated.rgb / max(accumulated.w, 1.0);
        data.m2 = vec3<f32>(3.402823e38);
        data.count = total_count;
        variance[pixel_id] = data;
        return;
    }

    let previous_sum = data.mean * f32(data.count);
    let sample = accumulated.rgb - previous_sum;
    data.count = total_count;
    let n = f32(data.count);
    let delta = sample - data.mean;
    data.mean += delta / n;
    let delta2 = sample - data.mean;
    data.m2 += delta * delta2;
    variance[pixel_id] = data;
}
