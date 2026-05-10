// ReSTIR Final Shading pass.
// Apply selected reservoir samples to produce final output.

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

struct Hit {
    t: f32,
    instance_id: u32,
    _pad: vec2<u32>,
    normal: vec3<f32>,
    hit: u32,
}

struct Ray {
    origin: vec3<f32>,
    pixel_id: u32,
    dir: vec3<f32>,
    bounce: u32,
    throughput: vec3<f32>,
    flags: u32,
}

// GPU instance: 96 bytes, matches GpuInstance in Rust.
struct Instance {
    model_inv_0: vec4<f32>,
    model_inv_1: vec4<f32>,
    model_inv_2: vec4<f32>,
    model_inv_3: vec4<f32>,
    color: vec4<f32>,
    object_id: u32,
    material_id: u32,
    _pad0: u32,
    _pad1: u32,
};

// Material matching GpuMaterial layout (144 bytes, vec4-packed).
struct Material {
    base_color_weight: vec4<f32>,
    specular_color_weight: vec4<f32>,
    transmission_color_weight: vec4<f32>,
    subsurface_color_weight: vec4<f32>,
    coat_color_weight: vec4<f32>,
    emission_color_weight: vec4<f32>,
    opacity: vec4<f32>,
    params1: vec4<f32>,
    params2: vec4<f32>,
};

// Tile-aware params: width/height are the full image. rays/hits are
// tile-local layout (sized by tile_w*tile_h); reservoirs/output/sample_map
// are full-image-sized.
struct Params {
    width: u32,
    height: u32,
    frame_count: u32,
    _pad: u32,
    camera_pos: vec3<f32>,
    _pad2: f32,
    tile_x: u32,
    tile_y: u32,
    tile_w: u32,
    tile_h: u32,
}

struct EnvParams {
    intensity: f32,
    rotation: f32,
    enabled: f32,
    use_importance_sampling: f32,
    env_width: f32,
    env_height: f32,
    global_opacity: f32,
    time: f32,
};

@group(0) @binding(0) var<storage, read> reservoirs: array<Reservoir>;
@group(0) @binding(1) var<storage, read> hits: array<Hit>;
@group(0) @binding(2) var<storage, read_write> output: array<vec4<f32>>;
@group(0) @binding(3) var<uniform> params: Params;
@group(0) @binding(4) var<storage, read> instances: array<Instance>;
@group(0) @binding(5) var<storage, read> materials: array<Material>;
@group(0) @binding(6) var<storage, read> sample_map: array<u32>;
@group(0) @binding(7) var<storage, read> rays: array<Ray>;
@group(0) @binding(8) var env_map: texture_2d<f32>;
@group(0) @binding(9) var env_sampler: sampler;
@group(0) @binding(10) var<uniform> env: EnvParams;

const PI: f32 = 3.14159265;
const EPS: f32 = 1e-5;

fn dir_to_equirect_uv(dir: vec3<f32>, rotation: f32) -> vec2<f32> {
    let theta = atan2(dir.z, dir.x);
    let phi = asin(clamp(dir.y, -1.0, 1.0));
    var u = theta / (2.0 * PI) + 0.5 + rotation / (2.0 * PI);
    let v = 0.5 - phi / PI;
    u = u - floor(u);
    return vec2<f32>(u, v);
}

fn atmospheric_sky(dir: vec3<f32>, time: f32) -> vec3<f32> {
    let sun_angle = time * 0.1;
    let sun_height = sin(sun_angle) * 0.8 + 0.1;
    let sun_x = cos(sun_angle) * 0.6;
    let sun_z = sin(sun_angle * 0.7) * 0.4;
    let sun_dir = normalize(vec3<f32>(sun_x, sun_height, sun_z));
    let sun_dot = max(dot(dir, sun_dir), 0.0);
    let zenith = max(dir.y, 0.0);
    let mie_g = 0.76;
    let mie_phase = (1.0 - mie_g * mie_g) / pow(1.0 + mie_g * mie_g - 2.0 * mie_g * sun_dot, 1.5);
    let mie = mie_phase * 0.003;
    let optical_depth = 1.0 / max(dir.y + 0.15, 0.05);
    let extinction = exp(-optical_depth * 0.3);
    let day_factor = clamp(sun_dir.y * 2.0 + 0.5, 0.0, 1.0);
    let sunset_factor = clamp(1.0 - abs(sun_dir.y) * 3.0, 0.0, 1.0);
    let day_sky = vec3<f32>(0.3, 0.5, 0.9);
    let sunset_sky = vec3<f32>(0.9, 0.4, 0.2);
    let night_sky = vec3<f32>(0.02, 0.02, 0.05);
    let sky_base = mix(night_sky, mix(day_sky, sunset_sky, sunset_factor), day_factor);
    let sky_blue = sky_base * (1.0 - extinction) * 2.0;
    let horizon_color = mix(vec3<f32>(0.3, 0.2, 0.3), vec3<f32>(1.0, 0.5, 0.2), sunset_factor);
    let horizon_glow = horizon_color * (1.0 - zenith) * extinction * (0.3 + sunset_factor * 0.7);
    let sun_color = mix(vec3<f32>(1.0, 0.3, 0.1), vec3<f32>(1.0, 0.95, 0.9), clamp(sun_dir.y * 2.0, 0.0, 1.0));
    let sun_visible = select(0.0, 1.0, sun_dir.y > -0.1);
    let sun_disk = pow(sun_dot, 512.0) * sun_color * 20.0 * sun_visible;
    let corona = pow(sun_dot, 8.0) * sun_color * mie * 50.0 * sun_visible;
    let glow = pow(sun_dot, 2.0) * mix(vec3<f32>(0.5, 0.2, 0.1), vec3<f32>(0.4, 0.3, 0.2), day_factor) * (1.0 - zenith) * sun_visible;
    let ground_color = mix(vec3<f32>(0.02, 0.02, 0.03), vec3<f32>(0.1, 0.08, 0.06), day_factor);
    let ground = max(-dir.y, 0.0) * ground_color;
    let star_hash = fract(sin(dot(dir.xz, vec2<f32>(12.9898, 78.233))) * 43758.5453);
    let stars = select(0.0, star_hash * star_hash * 2.0, star_hash > 0.997) * (1.0 - day_factor);
    return sky_blue + horizon_glow + sun_disk + corona + glow + ground + vec3<f32>(stars);
}

fn sky_color(dir: vec3<f32>) -> vec3<f32> {
    if env.enabled > 0.5 {
        let uv = dir_to_equirect_uv(dir, env.rotation);
        let color = textureSampleLevel(env_map, env_sampler, uv, 0.0).rgb;
        return color * env.intensity;
    }
    return atmospheric_sky(dir, env.time) * env.intensity;
}

fn fresnel_schlick(cos_theta: f32, f0: vec3<f32>) -> vec3<f32> {
    return f0 + (vec3<f32>(1.0) - f0) * pow(1.0 - cos_theta, 5.0);
}

fn ggx_d(ndoth: f32, alpha: f32) -> f32 {
    let a2 = alpha * alpha;
    let d = ndoth * ndoth * (a2 - 1.0) + 1.0;
    return a2 / (PI * d * d + EPS);
}

fn smith_g1(ndotv: f32, alpha: f32) -> f32 {
    let a = alpha;
    let a2 = a * a;
    let b = ndotv * ndotv;
    return 2.0 * ndotv / (ndotv + sqrt(a2 + b - a2 * b) + EPS);
}

@compute @workgroup_size(8, 8)
fn main(@builtin(global_invocation_id) gid: vec3<u32>) {
    if gid.x >= params.tile_w || gid.y >= params.tile_h { return; }
    let gx = params.tile_x + gid.x;
    let gy = params.tile_y + gid.y;
    if gx >= params.width || gy >= params.height { return; }
    // local_id: tile-local index for rays/hits buffers.
    let local_id = gid.y * params.tile_w + gid.x;
    // pixel_id: full-image global index for reservoirs/output/sample_map.
    let pixel_id = gy * params.width + gx;
    let reservoir = reservoirs[pixel_id];
    let hit = hits[local_id];
    let spp_limit = sample_map[pixel_id];
    // Use actual sample count from output buffer (not frame_count which is batched on CPU)
    let current_samples = u32(output[pixel_id].w);
    if current_samples >= spp_limit {
        return;
    }

    // Check for valid sample and hit
    if hit.hit == 0u || reservoir.sample.valid == 0u {
        if hit.hit == 0u {
            let ray = rays[local_id];
            let dir = normalize(ray.dir);
            output[pixel_id] += vec4<f32>(sky_color(dir), 1.0);
        }
        return;
    }

    // Compute shading contribution
    let wi = normalize(reservoir.sample.wi);
    var normal = normalize(hit.normal);
    let cos_theta = max(dot(wi, normal), 0.0);

    if cos_theta <= 0.0 {
        return;
    }

    let inst = instances[hit.instance_id];
    let mat = materials[inst.material_id];

    let base_color = inst.color.rgb * mat.base_color_weight.rgb;
    let base_weight = mat.base_color_weight.a;
    let spec_color = mat.specular_color_weight.rgb;
    let spec_weight = mat.specular_color_weight.a;
    let opacity = mat.opacity.x;

    let metallic = mat.params1.y;
    let roughness = max(mat.params1.z, 0.04);
    let ior = mat.params1.w;

    let ray = rays[local_id];
    let surface_p = ray.origin + ray.dir * hit.t;
    let v_dir = normalize(params.camera_pos - surface_p);
    let ndotv = max(dot(normal, v_dir), EPS);

    let h = normalize(v_dir + wi);
    let ndoth = max(dot(normal, h), EPS);
    let hdotv = max(dot(h, v_dir), EPS);

    let alpha = roughness * roughness;
    let f0_dielectric = vec3<f32>(pow((ior - 1.0) / (ior + 1.0), 2.0));
    let f0 = mix(f0_dielectric * spec_color, base_color, metallic);
    let f = fresnel_schlick(hdotv, f0);
    let d = ggx_d(ndoth, alpha);
    let g = smith_g1(ndotv, alpha) * smith_g1(cos_theta, alpha);
    let spec = spec_weight * f * d * g / max(4.0 * ndotv * cos_theta, EPS);

    let diffuse_color = (base_color * base_weight) * (1.0 - metallic);
    let diffuse = diffuse_color * (vec3<f32>(1.0) - f) / PI;

    let bsdf = (diffuse + spec) * opacity;

    let radiance = reservoir.sample.radiance;
    let contribution = radiance * bsdf * cos_theta * reservoir.w;

    // Accumulate (progressive)
    output[pixel_id] += vec4<f32>(contribution, 1.0);
}
