// Wavefront Ray Generation pass.
// Generates camera rays for all pixels.

struct Camera {
    inv_view: mat4x4<f32>,
    inv_proj: mat4x4<f32>,
    position: vec3<f32>,
    _pad0: u32,
    frame_count: u32,
    max_bounces: u32,
    max_transmission: u32,
    dof_enabled: u32,
    aperture: f32,
    focus_dist: f32,
    _pad1: vec2<u32>,
    // Slice plane params (match PtCameraUniform)
    slice_enabled: f32,
    slice_position: f32,
    slice_invert: f32,
    _pad2: f32,
    slice_normal: vec3<f32>,
    _pad3: f32,
    // Spectral options (match PtCameraUniform)
    spectral_mode: u32,
    spectral_samples: u32,
    spectral_dispersion: u32,
    _pad4: u32,
}

struct Dims {
    full_width: u32,
    full_height: u32,
    tile_width: u32,
    tile_height: u32,
    tile_x: u32,
    tile_y: u32,
    _pad: vec2<u32>,
}

struct Ray {
    origin: vec3<f32>,
    pixel_id: u32,
    dir: vec3<f32>,
    bounce: u32,
    throughput: vec3<f32>,
    flags: u32,
}

@group(0) @binding(0) var<uniform> camera: Camera;
@group(0) @binding(1) var<uniform> dims: Dims;
@group(0) @binding(2) var<storage, read_write> rays: array<Ray>;
@group(0) @binding(3) var<storage, read_write> count: array<atomic<u32>>;
@group(0) @binding(4) var<storage, read> sample_map: array<u32>;
@group(0) @binding(5) var<storage, read> accum: array<vec4<f32>>;

// PCG hash for random
fn pcg(n: u32) -> u32 {
    var h = n * 747796405u + 2891336453u;
    h = ((h >> ((h >> 28u) + 4u)) ^ h) * 277803737u;
    return (h >> 22u) ^ h;
}

fn rand(seed: ptr<function, u32>) -> f32 {
    *seed = pcg(*seed);
    return f32(*seed) / 4294967295.0;
}

@compute @workgroup_size(8, 8)
fn main(@builtin(global_invocation_id) gid: vec3<u32>) {
    let width = dims.tile_width;
    let height = dims.tile_height;
    if gid.x >= width || gid.y >= height { return; }

    let global_x = dims.tile_x + gid.x;
    let global_y = dims.tile_y + gid.y;
    if global_x >= dims.full_width || global_y >= dims.full_height {
        let ray_index = gid.y * width + gid.x;
        rays[ray_index] = Ray(
            vec3<f32>(0.0),
            0u,
            vec3<f32>(0.0),
            0u,
            vec3<f32>(0.0),
            0u
        );
        return;
    }

    let pixel_id = global_y * dims.full_width + global_x;
    let ray_index = gid.y * width + gid.x;
    let spp_limit = sample_map[pixel_id];
    // Use actual sample count from accum buffer (not frame_count which is batched on CPU)
    let current_samples = u32(accum[pixel_id].w);
    if current_samples >= spp_limit {
        // Mark ray inactive to avoid stale rays from previous frames.
        rays[ray_index] = Ray(
            vec3<f32>(0.0),
            pixel_id,
            vec3<f32>(0.0),
            0u,
            vec3<f32>(0.0),
            0u
        );
        return;
    }
    var seed = pixel_id ^ (camera.frame_count * 1973u);

    // Jitter for anti-aliasing
    let jx = rand(&seed);
    let jy = rand(&seed);
    
    // NDC coords (matching megakernel)
    let ndc = vec2<f32>(
        (f32(global_x) + jx) / f32(dims.full_width) * 2.0 - 1.0,
        1.0 - (f32(global_y) + jy) / f32(dims.full_height) * 2.0,
    );

    // Unproject near and far planes to view space
    let near = camera.inv_proj * vec4<f32>(ndc, -1.0, 1.0);
    let far  = camera.inv_proj * vec4<f32>(ndc,  1.0, 1.0);
    let near3 = near.xyz / near.w;
    let far3  = far.xyz / far.w;
    let origin_view = near3;
    let dir_view = normalize(far3 - near3);

    var origin: vec3<f32>;
    var dir: vec3<f32>;

    // DoF (optional)
    if camera.dof_enabled != 0u && camera.aperture > 0.0 {
        let t_focus = camera.focus_dist / max(abs(dir_view.z), 0.001);
        let focus_point_view = origin_view + dir_view * t_focus;
        let r = sqrt(rand(&seed)) * camera.aperture;
        let theta = rand(&seed) * 6.283185;
        let lens_sample = vec2<f32>(r * cos(theta), r * sin(theta));
        let lens_origin_view = origin_view + vec3<f32>(lens_sample, 0.0);
        let new_dir_view = normalize(focus_point_view - lens_origin_view);
        origin = (camera.inv_view * vec4<f32>(lens_origin_view, 1.0)).xyz;
        dir = normalize((camera.inv_view * vec4<f32>(new_dir_view, 0.0)).xyz);
    } else {
        origin = (camera.inv_view * vec4<f32>(origin_view, 1.0)).xyz;
        dir = normalize((camera.inv_view * vec4<f32>(dir_view, 0.0)).xyz);
    }

    // Write ray
    rays[ray_index] = Ray(
        origin,
        pixel_id,
        dir,
        0u,
        vec3<f32>(1.0),
        1u  // active
    );

    // Count buffer is initialized on CPU (count_in + count_out).
}
