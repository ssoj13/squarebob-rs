// Wavefront G-buffer pass for ReSTIR.
// Computes depth, normal, and motion vectors per pixel.

struct Ray {
    origin: vec3<f32>,
    pixel_id: u32,
    dir: vec3<f32>,
    bounce: u32,
    throughput: vec3<f32>,
    flags: u32,
}

struct Hit {
    t: f32,
    instance_id: u32,
    _pad: vec2<u32>,
    normal: vec3<f32>,
    hit: u32,
}

struct MotionVector {
    motion: vec2<f32>,
    depth: f32,
    valid: u32,
}

// Tile-aware params. `tile_w`/`tile_h` size the wavefront ray/hit buffers
// (always tile-local layout); `full_w`/`full_h` size the full-image g-buffer
// outputs. In non-tiled mode tile_x=tile_y=0 and tile_w/h == full_w/h.
struct Params {
    tile_w: u32,
    tile_h: u32,
    tile_x: u32,
    tile_y: u32,
    full_w: u32,
    full_h: u32,
    _pad: vec2<u32>,
    prev_view_proj: mat4x4<f32>,
    curr_view_proj: mat4x4<f32>,
}

@group(0) @binding(0) var<storage, read> rays: array<Ray>;
@group(0) @binding(1) var<storage, read> hits: array<Hit>;
@group(0) @binding(2) var<storage, read_write> depth_buf: array<f32>;
@group(0) @binding(3) var<storage, read_write> normal_buf: array<vec4<f32>>;
@group(0) @binding(4) var<storage, read_write> motion_buf: array<MotionVector>;
@group(0) @binding(5) var<uniform> params: Params;

@compute @workgroup_size(8, 8)
fn main(@builtin(global_invocation_id) gid: vec3<u32>) {
    if gid.x >= params.tile_w || gid.y >= params.tile_h { return; }
    // local_id: index into tile-sized rays/hits buffers
    let local_id = gid.y * params.tile_w + gid.x;
    // global_id: index into full-image depth/normal/motion buffers
    let global_x = params.tile_x + gid.x;
    let global_y = params.tile_y + gid.y;
    if global_x >= params.full_w || global_y >= params.full_h { return; }
    let global_id = global_y * params.full_w + global_x;

    let hit = hits[local_id];

    if hit.hit == 0u {
        depth_buf[global_id] = 0.0;
        normal_buf[global_id] = vec4<f32>(0.0);
        motion_buf[global_id] = MotionVector(vec2<f32>(0.0), 0.0, 0u);
        return;
    }

    let ray = rays[local_id];
    let world_pos = ray.origin + ray.dir * hit.t;

    let prev_clip = params.prev_view_proj * vec4<f32>(world_pos, 1.0);
    let curr_clip = params.curr_view_proj * vec4<f32>(world_pos, 1.0);

    if prev_clip.w <= 0.0 || curr_clip.w <= 0.0 {
        motion_buf[global_id] = MotionVector(vec2<f32>(0.0), hit.t, 0u);
    } else {
        let prev_ndc = prev_clip.xyz / prev_clip.w;
        let curr_ndc = curr_clip.xyz / curr_clip.w;

        // Motion vectors are in full-image pixel coordinates.
        let prev_px = vec2<f32>(
            (prev_ndc.x * 0.5 + 0.5) * f32(params.full_w),
            (1.0 - (prev_ndc.y * 0.5 + 0.5)) * f32(params.full_h)
        );
        let curr_px = vec2<f32>(
            (curr_ndc.x * 0.5 + 0.5) * f32(params.full_w),
            (1.0 - (curr_ndc.y * 0.5 + 0.5)) * f32(params.full_h)
        );

        motion_buf[global_id] = MotionVector(prev_px - curr_px, hit.t, 1u);
    }

    depth_buf[global_id] = hit.t;
    normal_buf[global_id] = vec4<f32>(hit.normal, 1.0);
}
