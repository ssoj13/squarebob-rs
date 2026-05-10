// Path Guiding Sample pass.
// Query SVO to get guided sampling direction.

struct SvoNode {
    children: array<u32, 8>,
    radiance: vec3<f32>,
    count: u32,
}

struct Params {
    scene_min: vec4<f32>,   // xyz=min
    scene_max: vec4<f32>,   // xyz=max
    params0: vec4<u32>,     // x=resolution, y=frame_count, z=tile_w, w=tile_h
    params1: vec4<f32>,     // x=guide_weight
    tile_pos: vec4<u32>,    // x=tile_x, y=tile_y, z=full_w, w=full_h
}

@group(0) @binding(0) var<storage, read> svo: array<SvoNode>;
@group(0) @binding(1) var<storage, read_write> guide: array<u32>;
@group(0) @binding(2) var<uniform> params: Params;

// PCG hash
fn pcg(n: u32) -> u32 {
    var h = n * 747796405u + 2891336453u;
    h = ((h >> ((h >> 28u) + 4u)) ^ h) * 277803737u;
    return (h >> 22u) ^ h;
}

fn rand(seed: ptr<function, u32>) -> f32 {
    *seed = pcg(*seed);
    return f32(*seed) / 4294967295.0;
}

// Map world position to voxel index
fn world_to_voxel(pos: vec3<f32>) -> vec3<u32> {
    let scene_min = params.scene_min.xyz;
    let scene_max = params.scene_max.xyz;
    let norm = (pos - scene_min) / (scene_max - scene_min);
    let clamped = clamp(norm, vec3<f32>(0.0), vec3<f32>(0.999));
    return vec3<u32>(clamped * f32(params.params0.x));
}

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

fn store_vec4(base: u32, v: vec4<f32>) {
    guide[base] = pack2x16float(v.xy);
    guide[base + 1u] = pack2x16float(v.zw);
}

// Sample hemisphere proportional to radiance distribution
fn sample_guided(pos: vec3<f32>, seed: ptr<function, u32>) -> vec3<f32> {
    let voxel = world_to_voxel(pos);
    let idx = voxel_to_index(voxel);

    // Simple approach: look at neighboring voxels and weight by radiance
    var total_weight = 0.0;
    var weighted_dir = vec3<f32>(0.0);
    var best_dir = vec3<f32>(0.0);
    var best_w = 0.0;

    // Sample nearby voxels
    for (var dx = -1; dx <= 1; dx++) {
        for (var dy = -1; dy <= 1; dy++) {
            for (var dz = -1; dz <= 1; dz++) {
                let nv = vec3<i32>(voxel) + vec3<i32>(dx, dy, dz);
                if nv.x < 0 || nv.y < 0 || nv.z < 0 { continue; }
                if nv.x >= i32(params.params0.x) ||
                   nv.y >= i32(params.params0.x) ||
                   nv.z >= i32(params.params0.x) { continue; }

                let nidx = voxel_to_index(vec3<u32>(nv));
                let node = svo[nidx];

                if node.count > 0u {
                    let rad = node.radiance / f32(node.count);
                    let w = length(rad);
                    let dir = normalize(vec3<f32>(f32(dx), f32(dy), f32(dz)) + vec3<f32>(0.001));
                    weighted_dir += dir * w;
                    total_weight += w;
                    if w > best_w {
                        best_w = w;
                        best_dir = dir;
                    }
                }
            }
        }
    }

    // If we have data, bias toward high-radiance direction
    if total_weight > 0.001 {
        let bias_dir = normalize(weighted_dir);
        // Mix with random direction
        let u1 = rand(seed);
        let u2 = rand(seed);
        let r = sqrt(u1);
        let theta = 6.283185 * u2;
        let rand_dir = vec3<f32>(r * cos(theta), sqrt(1.0 - u1), r * sin(theta));

        return normalize(mix(rand_dir, bias_dir, params.params1.x));
    }

    // Fallback: uniform hemisphere
    let u1 = rand(seed);
    let u2 = rand(seed);
    let r = sqrt(u1);
    let theta = 6.283185 * u2;
    return vec3<f32>(r * cos(theta), sqrt(1.0 - u1), r * sin(theta));
}

@compute @workgroup_size(64)
fn main(@builtin(global_invocation_id) gid: vec3<u32>) {
    // gid.x indexes into the *tile* pixel range [0, tile_w*tile_h). Remap
    // to a global pixel index so that the per-pixel `guide` buffer (which
    // is sized for the full image) does not alias between tiles.
    let local_idx = gid.x;
    let tile_w = params.params0.z;
    let tile_h = params.params0.w;
    let lx = local_idx % max(tile_w, 1u);
    let ly = local_idx / max(tile_w, 1u);
    if ly >= tile_h { return; }
    let gx = params.tile_pos.x + lx;
    let gy = params.tile_pos.y + ly;
    if gx >= params.tile_pos.z || gy >= params.tile_pos.w { return; }
    let pixel_idx = gy * params.tile_pos.z + gx;

    var seed = pixel_idx ^ (params.params0.y * 1973u);

    let base = guide_base(pixel_idx);
    let guided = load_vec4(base);
    let sample_pos = load_vec4(base + 2u);
    let sample_rad = load_vec4(base + 4u);
    if length(sample_rad.xyz) < 0.001 {
        store_vec4(base, vec4<f32>(0.0, 0.0, 0.0, 0.15915494)); // 1 / (2*pi)
        return;
    }

    let dir = sample_guided(sample_pos.xyz, &seed);
    let voxel = world_to_voxel(sample_pos.xyz);
    var total_weight = 0.0;
    var best_w = 0.0;
    var best_dir = vec3<f32>(0.0);
    for (var dx = -1; dx <= 1; dx++) {
        for (var dy = -1; dy <= 1; dy++) {
            for (var dz = -1; dz <= 1; dz++) {
                let nv = vec3<i32>(voxel) + vec3<i32>(dx, dy, dz);
                if nv.x < 0 || nv.y < 0 || nv.z < 0 { continue; }
                if nv.x >= i32(params.params0.x) ||
                   nv.y >= i32(params.params0.x) ||
                   nv.z >= i32(params.params0.x) { continue; }
                let nidx = voxel_to_index(vec3<u32>(nv));
                let node = svo[nidx];
                if node.count > 0u {
                    let rad = node.radiance / f32(node.count);
                    let w = length(rad);
                    total_weight += w;
                    if w > best_w {
                        best_w = w;
                        best_dir = normalize(vec3<f32>(f32(dx), f32(dy), f32(dz)) + vec3<f32>(0.001));
                    }
                }
            }
        }
    }

    let hemi_pdf = 0.15915494;
    var guide_pdf = hemi_pdf;
    if total_weight > 0.0 && best_w > 0.0 {
        let align = max(dot(dir, best_dir), 0.0);
        guide_pdf = max((best_w / total_weight) * (align / 3.14159265), 1e-5);
    }
    store_vec4(base, vec4<f32>(dir, guide_pdf));
}
