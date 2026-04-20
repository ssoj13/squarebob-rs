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

    // Map variance to SPP
    // Higher variance = more samples needed
    var spp = params.min_spp;

    if var_val > params.variance_threshold {
        // Scale linearly with variance
        let scale = min(var_val / params.variance_threshold, 10.0);
        spp = min(u32(f32(params.min_spp) * scale), params.max_spp);
    }

    sample_map[pixel_id] = spp;
}
