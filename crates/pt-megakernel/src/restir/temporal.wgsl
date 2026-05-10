// ReSTIR Temporal Resampling pass.
// Combine current frame with reprojected previous frame.

struct Sample {
    position: vec3<f32>,
    valid: u32,
    wi: vec3<f32>,
    light_type: u32,
    radiance: vec3<f32>,
    dist: f32,
    normal: vec3<f32>,
    _pad: u32,
}

struct Reservoir {
    sample: Sample,
    w_sum: f32,
    m: u32,
    w: f32,
    _pad: u32,
}

struct MotionVector {
    motion: vec2<f32>,
    depth: f32,
    valid: u32,
}

// Tile-aware params: width/height are the full image (all buffers below
// are full-image-sized). tile_x/y is the tile origin in full-image coords,
// tile_w/h size the dispatch — gid.xy is remapped to global coords before
// indexing.
//
// IMPORTANT: padding is three f32 scalars, NOT vec3<f32>. WGSL uniform
// layout rules align vec3<f32> to 16 bytes and round the struct up to a
// multiple of the largest member's align — that would make WGSL size 64
// while the Rust mirror is 48, breaking min_binding_size matching at
// pipeline creation.
struct Params {
    width: u32,
    height: u32,
    frame_count: u32,
    m_max: u32,
    depth_threshold: f32,
    _pad0: f32,
    _pad1: f32,
    _pad2: f32,
    tile_x: u32,
    tile_y: u32,
    tile_w: u32,
    tile_h: u32,
}

@group(0) @binding(0) var<storage, read> prev_reservoirs: array<Reservoir>;
@group(0) @binding(1) var<storage, read_write> curr_reservoirs: array<Reservoir>;
@group(0) @binding(2) var<storage, read> motion_vectors: array<MotionVector>;
@group(0) @binding(3) var<storage, read> prev_depth: array<f32>;
@group(0) @binding(4) var<storage, read> curr_depth: array<f32>;
@group(0) @binding(5) var<uniform> params: Params;

fn pcg(n: u32) -> u32 {
    var h = n * 747796405u + 2891336453u;
    h = ((h >> ((h >> 28u) + 4u)) ^ h) * 277803737u;
    return (h >> 22u) ^ h;
}

fn rand(seed: ptr<function, u32>) -> f32 {
    *seed = pcg(*seed);
    return f32(*seed) / 4294967295.0;
}

// Combine reservoir r2 into r1
fn combine_reservoirs(
    r1: ptr<function, Reservoir>,
    r2: Reservoir,
    target_at_r1: f32,
    seed: ptr<function, u32>
) {
    // Compute weight for r2's sample at r1's shading point
    let w2 = target_at_r1 * r2.w * f32(r2.m);

    (*r1).w_sum += w2;
    (*r1).m += r2.m;

    if rand(seed) * (*r1).w_sum < w2 {
        (*r1).sample = r2.sample;
    }
}

@compute @workgroup_size(8, 8)
fn main(@builtin(global_invocation_id) gid: vec3<u32>) {
    // Dispatch is tile-sized; remap gid.xy to full-image coords.
    if gid.x >= params.tile_w || gid.y >= params.tile_h { return; }
    let gx = params.tile_x + gid.x;
    let gy = params.tile_y + gid.y;
    if gx >= params.width || gy >= params.height { return; }
    let pixel_id = gy * params.width + gx;
    var reservoir = curr_reservoirs[pixel_id];
    var seed = pixel_id ^ (params.frame_count * 7919u);

    // Check if we have a valid current sample
    if reservoir.sample.valid == 0u {
        return;
    }

    // Get motion vector and reproject
    let mv = motion_vectors[pixel_id];
    if mv.valid == 0u {
        return;
    }

    // prev_pos must use global pixel coords (motion is in full-image space).
    let prev_pos = vec2<f32>(f32(gx), f32(gy)) + mv.motion;
    let prev_x = i32(prev_pos.x);
    let prev_y = i32(prev_pos.y);

    // Bounds check
    if prev_x < 0 || prev_x >= i32(params.width) ||
       prev_y < 0 || prev_y >= i32(params.height) {
        return;
    }

    let prev_pixel = u32(prev_y) * params.width + u32(prev_x);

    // Depth check for disocclusion
    let curr_z = curr_depth[pixel_id];
    let prev_z = prev_depth[prev_pixel];
    if abs(curr_z - prev_z) > params.depth_threshold * curr_z {
        return;
    }

    // Get previous reservoir
    var prev_reservoir = prev_reservoirs[prev_pixel];

    // Clamp history length to avoid bias
    if prev_reservoir.m > params.m_max {
        let scale = f32(params.m_max) / f32(prev_reservoir.m);
        prev_reservoir.w_sum *= scale;
        prev_reservoir.m = params.m_max;
    }

    // Compute target at current shading point
    // Simplified: assume same geometry, just use the sample
    let target_val = length(prev_reservoir.sample.radiance);

    // Combine reservoirs
    combine_reservoirs(&reservoir, prev_reservoir, target_val, &seed);

    // Update final weight
    if reservoir.m > 0u && reservoir.w_sum > 0.0 {
        let final_target = length(reservoir.sample.radiance);
        if final_target > 0.0 {
            reservoir.w = reservoir.w_sum / (f32(reservoir.m) * final_target);
        }
    }

    curr_reservoirs[pixel_id] = reservoir;
}
