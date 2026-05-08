// ReSTIR Initial Sampling pass.
// Generate M initial candidates and select one via RIS.

struct Hit {
    t: f32,
    instance_id: u32,
    _pad: vec2<u32>,
    normal: vec3<f32>,
    hit: u32,
}

struct BvhNode {
    aabb_min: vec3<f32>,
    left_or_first: u32,
    aabb_max: vec3<f32>,
    count: u32,
}

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
}

struct Ray {
    origin: vec3<f32>,
    pixel_id: u32,
    dir: vec3<f32>,
    bounce: u32,
    throughput: vec3<f32>,
    flags: u32,
}

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

struct Params {
    width: u32,
    height: u32,
    frame_count: u32,
    num_candidates: u32,
}

struct EmissiveLight {
    center_area: vec4<f32>,
    axis_x: vec4<f32>,
    axis_y: vec4<f32>,
    axis_z: vec4<f32>,
    emission_weight: vec4<f32>,
    instance_idx: u32,
    _pad0: vec3<u32>,
}

struct EmissiveLightSample {
    position: vec3<f32>,
    normal: vec3<f32>,
    emission: vec3<f32>,
    pdf_area: f32,
    instance_idx: u32,
}

struct EmissiveLightParams {
    params0: vec4<u32>,
    params1: vec4<f32>,
}

@group(0) @binding(0) var<storage, read> hits: array<Hit>;
@group(0) @binding(1) var<storage, read_write> reservoirs: array<Reservoir>;
@group(0) @binding(2) var<uniform> params: Params;
@group(0) @binding(3) var env_map: texture_2d<f32>;
@group(0) @binding(4) var env_sampler: sampler;
@group(0) @binding(5) var<uniform> env: EnvParams;
@group(0) @binding(6) var<storage, read> env_marginal_cdf: array<f32>;
@group(0) @binding(7) var<storage, read> env_conditional_cdf: array<f32>;
@group(0) @binding(8) var<storage, read> rays: array<Ray>;
@group(0) @binding(9) var<storage, read> nodes: array<BvhNode>;
@group(0) @binding(10) var<storage, read> instances: array<Instance>;
@group(0) @binding(11) var emissive_lights: texture_2d<f32>;
@group(0) @binding(12) var<uniform> emissive_light_params: EmissiveLightParams;

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

// Sample uniform direction on sphere
fn sample_sphere(seed: ptr<function, u32>) -> vec3<f32> {
    let u1 = rand(seed);
    let u2 = rand(seed);
    let z = 1.0 - 2.0 * u1;
    let r = sqrt(max(0.0, 1.0 - z * z));
    let phi = 6.283185 * u2;
    return vec3<f32>(r * cos(phi), r * sin(phi), z);
}

const PI: f32 = 3.14159265359;
const EPS: f32 = 1e-6;
const T_MAX: f32 = 1e30;
const MAX_STACK_DEPTH: u32 = 32u;

fn inst_model_inv(inst: Instance) -> mat4x4<f32> {
    return mat4x4<f32>(
        inst.model_inv_0,
        inst.model_inv_1,
        inst.model_inv_2,
        inst.model_inv_3,
    );
}

fn intersect_unit_cube(ray_o: vec3<f32>, ray_d: vec3<f32>) -> vec2<f32> {
    let inv_d = 1.0 / ray_d;
    let t0 = (vec3<f32>(-0.5) - ray_o) * inv_d;
    let t1 = (vec3<f32>(0.5) - ray_o) * inv_d;
    let tmin = min(t0, t1);
    let tmax = max(t0, t1);
    let t_enter = max(max(tmin.x, tmin.y), tmin.z);
    let t_exit = min(min(tmax.x, tmax.y), tmax.z);
    return vec2<f32>(t_enter, t_exit);
}

fn intersect_instance_shadow(ray: Ray, inst_idx: u32, max_t: f32) -> bool {
    let inst = instances[inst_idx];
    let m_inv = inst_model_inv(inst);
    let o_local = (m_inv * vec4<f32>(ray.origin, 1.0)).xyz;
    let d_local = (m_inv * vec4<f32>(ray.dir, 0.0)).xyz;
    let tt = intersect_unit_cube(o_local, d_local);
    let t_enter = tt.x;
    let t_exit = tt.y;
    if t_exit < 0.0 || t_enter > t_exit {
        return false;
    }
    let t_hit = select(t_enter, t_exit, t_enter < EPS);
    return t_hit > EPS && t_hit < max_t;
}

fn intersect_aabb(ray: Ray, inv_dir: vec3<f32>, node: BvhNode, t_best: f32) -> bool {
    let t1 = (node.aabb_min - ray.origin) * inv_dir;
    let t2 = (node.aabb_max - ray.origin) * inv_dir;
    let tmin = max(max(min(t1.x, t2.x), min(t1.y, t2.y)), min(t1.z, t2.z));
    let tmax = min(min(max(t1.x, t2.x), max(t1.y, t2.y)), max(t1.z, t2.z));
    return tmax >= max(tmin, 0.0) && tmin < t_best;
}

fn trace_shadow_ray(ray: Ray, max_t: f32) -> bool {
    let inv_dir = 1.0 / ray.dir;
    var stack: array<u32, MAX_STACK_DEPTH>;
    var sp: u32 = 1u;
    stack[0] = 0u;
    var loop_safety = 0u;

    while sp > 0u {
        loop_safety += 1u;
        if loop_safety > 4096u { break; }

        sp -= 1u;
        let node = nodes[stack[sp]];
        if !intersect_aabb(ray, inv_dir, node, max_t) {
            continue;
        }

        if node.count > 0u {
            for (var i = 0u; i < node.count; i++) {
                if intersect_instance_shadow(ray, node.left_or_first + i, max_t) {
                    return true;
                }
            }
        } else if sp + 2u <= MAX_STACK_DEPTH {
            stack[sp] = node.left_or_first + 1u;
            sp += 1u;
            stack[sp] = node.left_or_first;
            sp += 1u;
        }
    }
    return false;
}

fn dir_to_equirect_uv(dir: vec3<f32>, rotation: f32) -> vec2<f32> {
    let theta = atan2(dir.z, dir.x);
    let phi = asin(clamp(dir.y, -1.0, 1.0));
    var u = theta / (2.0 * PI) + 0.5 + rotation / (2.0 * PI);
    let v = 0.5 - phi / PI;
    u = u - floor(u);
    return vec2<f32>(u, v);
}

fn equirect_uv_to_dir(uv: vec2<f32>) -> vec3<f32> {
    let theta = (uv.x - 0.5) * 2.0 * PI;
    let phi = (0.5 - uv.y) * PI;
    let cos_phi = cos(phi);
    return vec3<f32>(cos_phi * cos(theta), sin(phi), cos_phi * sin(theta));
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

fn binary_search_cdf(cdf_offset: u32, size: u32, xi: f32) -> u32 {
    var lo = 0u;
    var hi = size;
    while lo < hi {
        let mid = (lo + hi) / 2u;
        if env_conditional_cdf[cdf_offset + mid] < xi {
            lo = mid + 1u;
        } else {
            hi = mid;
        }
    }
    return min(lo, size - 1u);
}

fn binary_search_marginal(size: u32, xi: f32) -> u32 {
    var lo = 0u;
    var hi = size;
    while lo < hi {
        let mid = (lo + hi) / 2u;
        if env_marginal_cdf[mid] < xi {
            lo = mid + 1u;
        } else {
            hi = mid;
        }
    }
    return min(lo, size - 1u);
}

// Returns (direction.xyz, pdf.w)
fn sample_env_direction(r1: f32, r2: f32) -> vec4<f32> {
    let w = u32(max(env.env_width, 1.0));
    let h = u32(max(env.env_height, 1.0));
    let y = binary_search_marginal(h, r2);
    let row_offset = y * w;
    let x = binary_search_cdf(row_offset, w, r1);

    let u = (f32(x) + 0.5) / f32(w);
    let v = (f32(y) + 0.5) / f32(h);
    let uv = vec2<f32>(u - env.rotation / (2.0 * PI), v);
    let dir = equirect_uv_to_dir(uv);
    let sin_theta = max(sin(PI * v), EPS);

    var marginal_pdf: f32;
    if y == 0u {
        marginal_pdf = env_marginal_cdf[0];
    } else {
        marginal_pdf = env_marginal_cdf[y] - env_marginal_cdf[y - 1u];
    }

    var conditional_pdf: f32;
    if x == 0u {
        conditional_pdf = env_conditional_cdf[row_offset];
    } else {
        conditional_pdf = env_conditional_cdf[row_offset + x] - env_conditional_cdf[row_offset + x - 1u];
    }

    let pdf = max(marginal_pdf * conditional_pdf * f32(w) * f32(h) / (2.0 * PI * PI * sin_theta), EPS);
    return vec4<f32>(dir, pdf);
}

fn load_emissive_light(idx: u32) -> EmissiveLight {
    var light: EmissiveLight;
    let x = i32(idx);
    light.center_area = textureLoad(emissive_lights, vec2<i32>(x, 0), 0);
    light.axis_x = textureLoad(emissive_lights, vec2<i32>(x, 1), 0);
    light.axis_y = textureLoad(emissive_lights, vec2<i32>(x, 2), 0);
    light.axis_z = textureLoad(emissive_lights, vec2<i32>(x, 3), 0);
    light.emission_weight = textureLoad(emissive_lights, vec2<i32>(x, 4), 0);
    light.instance_idx = u32(textureLoad(emissive_lights, vec2<i32>(x, 5), 0).x);
    light._pad0 = vec3<u32>(0u);
    return light;
}

fn sample_emissive_light(seed: ptr<function, u32>) -> EmissiveLightSample {
    let light_count = emissive_light_params.params0.z;
    let total_weight = max(emissive_light_params.params1.y, EPS);
    var pick_weight = rand(seed) * total_weight;
    var selected = load_emissive_light(0u);
    for (var i = 0u; i < light_count; i++) {
        let candidate = load_emissive_light(i);
        let weight = max(candidate.emission_weight.w, 0.0);
        if pick_weight <= weight || i == light_count - 1u {
            selected = candidate;
            break;
        }
        pick_weight -= weight;
    }

    let center = selected.center_area.xyz;
    let axis_x = selected.axis_x.xyz;
    let axis_y = selected.axis_y.xyz;
    let axis_z = selected.axis_z.xyz;
    let len_x = length(axis_x);
    let len_y = length(axis_y);
    let len_z = length(axis_z);
    let area_yz = len_y * len_z;
    let area_xz = len_x * len_z;
    let area_xy = len_x * len_y;
    let total_area = max(selected.center_area.w, EPS);

    let face_xi = rand(seed) * total_area;
    let u = rand(seed) - 0.5;
    let v = rand(seed) - 0.5;
    var position = center;
    var normal = vec3<f32>(0.0, 1.0, 0.0);

    let nx = normalize(cross(axis_y, axis_z));
    let ny = normalize(cross(axis_z, axis_x));
    let nz = normalize(cross(axis_x, axis_y));

    if face_xi < area_yz {
        position = center + axis_x * 0.5 + axis_y * u + axis_z * v;
        normal = nx;
    } else if face_xi < area_yz * 2.0 {
        position = center - axis_x * 0.5 + axis_y * u + axis_z * v;
        normal = -nx;
    } else if face_xi < area_yz * 2.0 + area_xz {
        position = center + axis_y * 0.5 + axis_x * u + axis_z * v;
        normal = ny;
    } else if face_xi < area_yz * 2.0 + area_xz * 2.0 {
        position = center - axis_y * 0.5 + axis_x * u + axis_z * v;
        normal = -ny;
    } else if face_xi < area_yz * 2.0 + area_xz * 2.0 + area_xy {
        position = center + axis_z * 0.5 + axis_x * u + axis_y * v;
        normal = nz;
    } else {
        position = center - axis_z * 0.5 + axis_x * u + axis_y * v;
        normal = -nz;
    }

    let light_pick_pdf = selected.emission_weight.w / total_weight;
    let pdf_area = light_pick_pdf / total_area;

    var result: EmissiveLightSample;
    result.position = position;
    result.normal = normal;
    result.emission = selected.emission_weight.xyz;
    result.pdf_area = max(pdf_area, EPS);
    result.instance_idx = selected.instance_idx;
    return result;
}

// Update reservoir with new sample
fn update_reservoir(r: ptr<function, Reservoir>, s: Sample, w: f32, seed: ptr<function, u32>) {
    (*r).w_sum += w;
    (*r).m += 1u;
    if rand(seed) * (*r).w_sum < w {
        (*r).sample = s;
    }
}

@compute @workgroup_size(8, 8)
fn main(@builtin(global_invocation_id) gid: vec3<u32>) {
    if gid.x >= params.width || gid.y >= params.height { return; }

    let pixel_id = gid.y * params.width + gid.x;
    let hit = hits[pixel_id];

    // Initialize reservoir
    var reservoir: Reservoir;
    reservoir.w_sum = 0.0;
    reservoir.m = 0u;
    reservoir.w = 0.0;
    reservoir.sample.valid = 0u;

    if hit.hit == 0u {
        reservoirs[pixel_id] = reservoir;
        return;
    }

    var seed = pixel_id ^ (params.frame_count * 1973u);
    let ray = rays[pixel_id];
    let surface_p = ray.origin + ray.dir * hit.t;
    let surface_n = normalize(hit.normal);
    let emissive_available =
        emissive_light_params.params0.x != 0u && emissive_light_params.params0.z > 0u;
    let env_available = env.enabled > 0.5;

    // Generate candidates
    for (var i = 0u; i < params.num_candidates; i++) {
        var wi: vec3<f32>;
        var pdf: f32;
        var radiance: vec3<f32>;
        var sample_position = vec3<f32>(0.0);
        var sample_normal = vec3<f32>(0.0);
        var light_type = 0u;

        let both_sources = emissive_available && env_available;
        let choose_emissive = emissive_available && (!env_available || rand(&seed) < 0.5);
        let source_pdf_scale = select(1.0, 0.5, both_sources);

        if choose_emissive {
            let light_sample = sample_emissive_light(&seed);
            if light_sample.instance_idx == hit.instance_id {
                continue;
            }

            let to_light = light_sample.position - surface_p;
            let dist2 = dot(to_light, to_light);
            if dist2 <= EPS {
                continue;
            }

            let dist = sqrt(dist2);
            wi = to_light / dist;
            let cos_theta = dot(wi, surface_n);
            let light_cos = dot(light_sample.normal, -wi);
            if cos_theta <= 0.0 || light_cos <= 0.0 {
                continue;
            }

            var shadow_ray = Ray(
                surface_p + surface_n * 0.001,
                ray.pixel_id,
                wi,
                ray.bounce,
                ray.throughput,
                ray.flags
            );
            if trace_shadow_ray(shadow_ray, max(dist - 0.002, EPS)) {
                continue;
            }

            pdf = max(light_sample.pdf_area * dist2 / max(light_cos, EPS), EPS) * source_pdf_scale;
            radiance = light_sample.emission;
            sample_position = light_sample.position;
            sample_normal = light_sample.normal;
            light_type = 1u;
        } else if env.enabled > 0.5 && env.use_importance_sampling > 0.5 {
            let env_sample = sample_env_direction(rand(&seed), rand(&seed));
            wi = env_sample.xyz;
            pdf = env_sample.w * source_pdf_scale;
            radiance = sky_color(wi);
            sample_position = surface_p + wi * 1e10;
            sample_normal = -wi;
        } else {
            wi = sample_sphere(&seed);
            pdf = 0.07957747 * source_pdf_scale; // 1/(4*pi)
            radiance = sky_color(wi);
            sample_position = surface_p + wi * 1e10;
            sample_normal = -wi;
        }

        let cos_theta = max(dot(wi, surface_n), 0.0);
        if cos_theta > 0.0 {
            var sample: Sample;
            sample.position = sample_position;
            sample.valid = 1u;
            sample.wi = wi;
            sample.light_type = light_type;
            sample.radiance = radiance;
            sample.dist = distance(sample_position, surface_p);
            sample.normal = sample_normal;

            // Target function: radiance * cos_theta
            let target_val = max(dot(radiance, vec3<f32>(0.2126, 0.7152, 0.0722)), 0.0) * cos_theta;
            let w = target_val / max(pdf, EPS);
            update_reservoir(&reservoir, sample, w, &seed);
        }
    }

    // Compute final weight
    if reservoir.m > 0u && reservoir.w_sum > 0.0 {
        let target_val = max(dot(reservoir.sample.radiance, vec3<f32>(0.2126, 0.7152, 0.0722)), 0.0) *
            max(dot(reservoir.sample.wi, surface_n), 0.0);
        if target_val > 0.0 {
            reservoir.w = reservoir.w_sum / (f32(reservoir.m) * target_val);
        }
    }

    reservoirs[pixel_id] = reservoir;
}
