// Path Guiding SVO Update pass.
// Accumulate path radiance into octree voxels.

struct SvoNode {
    children: array<u32, 8>,
    radiance: vec3<f32>,
    count: u32,
}

struct Params {
    scene_min: vec4<f32>,   // xyz=min
    scene_max: vec4<f32>,   // xyz=max
    params0: vec4<u32>,     // x=resolution, y=sample_count
    params1: vec4<f32>,     // x=decay
}

@group(0) @binding(0) var<storage, read_write> svo: array<SvoNode>;
@group(0) @binding(1) var<storage, read> guide: array<u32>;
@group(0) @binding(2) var<uniform> params: Params;

// Map world position to voxel index
fn world_to_voxel(pos: vec3<f32>) -> vec3<u32> {
    let scene_min = params.scene_min.xyz;
    let scene_max = params.scene_max.xyz;
    let norm = (pos - scene_min) / (scene_max - scene_min);
    let clamped = clamp(norm, vec3<f32>(0.0), vec3<f32>(0.999));
    return vec3<u32>(clamped * f32(params.params0.x));
}

// Flatten 3D voxel coords to 1D index
fn voxel_to_index(v: vec3<u32>) -> u32 {
    let r = params.params0.x;
    return v.x + v.y * r + v.z * r * r;
}

fn guide_base(idx: u32) -> u32 {
    return idx * 6u;
}

fn load_vec4(base: u32) -> vec4<f32> {
    let a = guide[base];
    let b = guide[base + 1u];
    let v0 = unpack2x16float(a);
    let v1 = unpack2x16float(b);
    return vec4<f32>(v0.x, v0.y, v1.x, v1.y);
}

@compute @workgroup_size(1)
fn main(@builtin(global_invocation_id) gid: vec3<u32>) {
    // Single-threaded update to avoid data races on SVO nodes.
    if gid.x != 0u { return; }

    for (var i = 0u; i < params.params0.y; i++) {
        let base = guide_base(i);
        let sample_pos = load_vec4(base + 2u);
        let sample_rad = load_vec4(base + 4u);

        // Skip invalid samples
        if length(sample_rad.xyz) < 0.001 { continue; }

        // Map to voxel
        let voxel = world_to_voxel(sample_pos.xyz);
        let idx = voxel_to_index(voxel);

        svo[idx].radiance += sample_rad.xyz;
        svo[idx].count += 1u;
    }
}
