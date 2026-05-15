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

    // DMC-style noise estimate: relative standard error of the mean.
    //   variance = M2 / (n-1)
    //   std_err  = sqrt(variance / n)              (uncertainty of mean estimate)
    //   noise    = std_err / max(luminance(mean), eps)
    // This is the same quantity V-Ray's DMC sampler / Arnold's noise threshold
    // expose to the user, so a single tunable `variance_threshold` works across
    // an HDR luminance range instead of clipping bright pixels to "too noisy"
    // and dark pixels to "too clean".
    var noise = 0.0;
    if data.count > 1u {
        let v = data.m2 / f32(data.count - 1u);
        let var_lum = dot(max(v, vec3<f32>(0.0)), vec3<f32>(0.2126, 0.7152, 0.0722));
        let std_err = sqrt(var_lum / f32(data.count));
        let mean_lum = dot(max(data.mean, vec3<f32>(0.0)), vec3<f32>(0.2126, 0.7152, 0.0722));
        noise = std_err / max(mean_lum, 1e-3);
    }

    let min_spp = min(params.min_spp, params.max_spp);

    // Do not classify a pixel from only a handful of samples. Early variance
    // estimates are noisy enough that they can freeze visible noise into place.
    if data.count < min_spp {
        sample_map[pixel_id] = params.max_spp;
        return;
    }

    // Map relative noise to a per-pixel SPP cap. At the threshold we hand out
    // min_spp; by 4× threshold we hit max_spp. The 4× factor reflects DMC
    // ergonomics: doubling samples halves std-err, so 4× threshold ≈ "needs
    // ~16× more samples", which is exactly the budget we should hand it.
    let span = params.max_spp - min_spp;
    let severity = clamp(noise / max(params.variance_threshold * 4.0, 1e-6), 0.0, 1.0);
    var spp = min_spp + u32(round(severity * f32(span)));

    // Keep giving "quiet" pixels a small rolling budget. This preserves the
    // speed benefit of adaptive sampling without permanently locking in an
    // unlucky low-variance estimate.
    let refinement_budget = max(8u, min(min_spp / 2u, 64u));
    spp = max(spp, data.count + refinement_budget);

    sample_map[pixel_id] = min(spp, params.max_spp);
}
