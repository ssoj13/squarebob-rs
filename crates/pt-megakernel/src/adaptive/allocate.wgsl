// Adaptive Sampling - SPP Allocation pass.
// Allocate samples based on variance.

struct VarianceData {
    mean: vec3<f32>,
    _pad0: u32,
    m2: vec3<f32>,
    count: u32,
}

struct Params {
    width: u32,
    height: u32,
    min_spp: u32,
    max_spp: u32,
    variance_threshold: f32,
    _pad: vec3<f32>,
}

@group(0) @binding(0) var<storage, read> variance: array<VarianceData>;
@group(0) @binding(1) var<storage, read_write> sample_map: array<u32>;
@group(0) @binding(2) var<uniform> params: Params;

@compute @workgroup_size(8, 8)
fn main(@builtin(global_invocation_id) gid: vec3<u32>) {
    if gid.x >= params.width || gid.y >= params.height { return; }

    let pixel_id = gid.y * params.width + gid.x;
    let data = variance[pixel_id];

    // Compute variance (M2 / (n-1))
    var var_val = 0.0;
    if data.count > 1u {
        let v = data.m2 / f32(data.count - 1u);
        var_val = (v.x + v.y + v.z) / 3.0; // Average luminance variance
    }

    let min_spp = min(params.min_spp, params.max_spp);

    // Do not classify a pixel from only a handful of samples. Early variance
    // estimates are noisy enough that they can freeze visible noise into place.
    if data.count < min_spp {
        sample_map[pixel_id] = params.max_spp;
        return;
    }

    // Map variance to a per-pixel SPP cap. At the threshold we allocate a small
    // extra budget; by 10x threshold the pixel receives the configured max.
    let span = params.max_spp - min_spp;
    let severity = clamp(var_val / max(params.variance_threshold * 10.0, 1e-6), 0.0, 1.0);
    var spp = min_spp + u32(round(severity * f32(span)));

    // Keep giving "quiet" pixels a small rolling budget. This preserves the
    // speed benefit of adaptive sampling without permanently locking in an
    // unlucky low-variance estimate.
    let refinement_budget = max(8u, min(min_spp / 2u, 64u));
    spp = max(spp, data.count + refinement_budget);

    sample_map[pixel_id] = min(spp, params.max_spp);
}
