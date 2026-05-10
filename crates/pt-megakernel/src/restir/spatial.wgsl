// ReSTIR Spatial Resampling pass.
// Combine with neighbor pixels for noise reduction.

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

// Tile-aware params: width/height are the full image. All bound buffers
// (reservoirs_in/out, depth_buf, normal_buf) are full-image-sized. The
// dispatch is tile-sized; gid.xy is remapped to global coords via tile_*.
struct Params {
    width: u32,
    height: u32,
    frame_count: u32,
    num_neighbors: u32,
    radius: f32,
    normal_threshold: f32,
    depth_threshold: f32,
    _pad: f32,
    tile_x: u32,
    tile_y: u32,
    tile_w: u32,
    tile_h: u32,
}

@group(0) @binding(0) var<storage, read> reservoirs_in: array<Reservoir>;
@group(0) @binding(1) var<storage, read_write> reservoirs_out: array<Reservoir>;
@group(0) @binding(2) var<storage, read> depth_buf: array<f32>;
@group(0) @binding(3) var<storage, read> normal_buf: array<vec4<f32>>;
@group(0) @binding(4) var<uniform> params: Params;

fn pcg(n: u32) -> u32 {
    var h = n * 747796405u + 2891336453u;
    h = ((h >> ((h >> 28u) + 4u)) ^ h) * 277803737u;
    return (h >> 22u) ^ h;
}

fn rand(seed: ptr<function, u32>) -> f32 {
    *seed = pcg(*seed);
    return f32(*seed) / 4294967295.0;
}

// Check if two pixels are geometrically similar
fn geometry_test(
    depth1: f32, normal1: vec3<f32>,
    depth2: f32, normal2: vec3<f32>,
    depth_thresh: f32, normal_thresh: f32
) -> bool {
    // Depth test
    if abs(depth1 - depth2) > depth_thresh * depth1 {
        return false;
    }
    // Normal test
    if dot(normal1, normal2) < normal_thresh {
        return false;
    }
    return true;
}

// Combine reservoir r2 into r1
fn combine_reservoirs(
    r1: ptr<function, Reservoir>,
    r2: Reservoir,
    target_at_r1: f32,
    seed: ptr<function, u32>
) {
    let w2 = target_at_r1 * r2.w * f32(r2.m);
    (*r1).w_sum += w2;
    (*r1).m += r2.m;
    if rand(seed) * (*r1).w_sum < w2 {
        (*r1).sample = r2.sample;
    }
}

@compute @workgroup_size(8, 8)
fn main(@builtin(global_invocation_id) gid: vec3<u32>) {
    // Dispatch is tile-sized; remap to full-image coords for all buffers.
    if gid.x >= params.tile_w || gid.y >= params.tile_h { return; }
    let gx = params.tile_x + gid.x;
    let gy = params.tile_y + gid.y;
    if gx >= params.width || gy >= params.height { return; }
    let pixel_id = gy * params.width + gx;
    var reservoir = reservoirs_in[pixel_id];
    var seed = pixel_id ^ (params.frame_count * 31337u);

    // Check if we have a valid sample
    if reservoir.sample.valid == 0u {
        reservoirs_out[pixel_id] = reservoir;
        return;
    }

    let center_depth = depth_buf[pixel_id];
    let center_normal = normal_buf[pixel_id].xyz;

    // Sample neighbors (offsets are in full-image coords; neighbors may
    // lie outside this tile but reservoirs/depth/normal are full-image).
    for (var i = 0u; i < params.num_neighbors; i++) {
        // Random offset within radius
        let angle = rand(&seed) * 6.283185;
        let dist = sqrt(rand(&seed)) * params.radius;
        let offset = vec2<i32>(
            i32(cos(angle) * dist),
            i32(sin(angle) * dist)
        );

        let nx = i32(gx) + offset.x;
        let ny = i32(gy) + offset.y;

        // Bounds check
        if nx < 0 || nx >= i32(params.width) ||
           ny < 0 || ny >= i32(params.height) {
            continue;
        }

        let neighbor_id = u32(ny) * params.width + u32(nx);
        let neighbor_reservoir = reservoirs_in[neighbor_id];

        if neighbor_reservoir.sample.valid == 0u {
            continue;
        }

        // Geometry test
        let neighbor_depth = depth_buf[neighbor_id];
        let neighbor_normal = normal_buf[neighbor_id].xyz;

        if !geometry_test(
            center_depth, center_normal,
            neighbor_depth, neighbor_normal,
            params.depth_threshold, params.normal_threshold
        ) {
            continue;
        }

        // Compute target at center pixel
        // Simplified: use sample radiance directly
        let target_val = length(neighbor_reservoir.sample.radiance);

        combine_reservoirs(&reservoir, neighbor_reservoir, target_val, &seed);
    }

    // Update final weight
    if reservoir.m > 0u && reservoir.w_sum > 0.0 {
        let final_target = length(reservoir.sample.radiance);
        if final_target > 0.0 {
            reservoir.w = reservoir.w_sum / (f32(reservoir.m) * final_target);
        }
    }

    reservoirs_out[pixel_id] = reservoir;
}
