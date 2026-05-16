// Instance-based BVH path tracing compute shader.
//
// Ray-box intersection against cube instances (no triangles).
// Each instance stores inverse model matrix + color.
// Progressive accumulation, GGX specular, HDR env map, NEE sun.

struct BVHNode {
    aabb_min: vec3<f32>,
    left_or_first: u32,
    aabb_max: vec3<f32>,
    count: u32,
};

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
    base_color_weight: vec4<f32>,         // rgb=color, a=weight
    specular_color_weight: vec4<f32>,     // rgb=color, a=weight
    transmission_color_weight: vec4<f32>, // rgb=color, a=weight
    subsurface_color_weight: vec4<f32>,   // rgb=color, a=weight
    coat_color_weight: vec4<f32>,         // rgb=color, a=weight
    emission_color_weight: vec4<f32>,     // rgb=color, a=weight (intensity in a)
    opacity: vec4<f32>,                   // rgb=opacity, a=unused
    params1: vec4<f32>,                   // x=diffuse_rough, y=metalness, z=spec_rough, w=spec_IOR
    params2: vec4<f32>,                   // x=anisotropy, y=coat_rough, z=coat_IOR, w=visible
};

struct Camera {
    inv_view: mat4x4<f32>,
    inv_proj: mat4x4<f32>,
    position: vec3<f32>,
    _pad0: u32,
    frame_count: u32,
    max_bounces: u32,
    max_transmission_depth: u32,
    dof_enabled: u32,
    aperture: f32,
    focus_distance: f32,
    _pad1: vec2<u32>,
    // Slice plane params
    slice_enabled: f32,
    slice_position: f32,
    slice_invert: f32,
    _pad2: f32,
    slice_normal: vec3<f32>,
    _pad3: f32,
    // Spectral options (PT only)
    spectral_mode: u32,
    spectral_samples: u32,
    spectral_dispersion: u32,
    sampler_mode: u32,
};

struct Ray {
    origin: vec3<f32>,
    dir: vec3<f32>,
};

struct HitInfo {
    t: f32,
    normal: vec3<f32>,
    inst_idx: u32,
    hit: bool,
};

struct EnvParams {
    intensity: f32,
    rotation: f32,
    enabled: f32,
    use_importance_sampling: f32,
    env_width: f32,
    env_height: f32,
    global_opacity: f32,
    time: f32,  // for procedural sky day/night cycle
};

struct EmissiveLight {
    center_area: vec4<f32>,
    axis_x: vec4<f32>,
    axis_y: vec4<f32>,
    axis_z: vec4<f32>,
    emission_weight: vec4<f32>,
    instance_idx: u32,
    _pad0: vec3<u32>,
};

struct EmissiveLightParams {
    params0: vec4<u32>, // enabled, samples_per_hit, light_count, reserved
    params1: vec4<f32>, // min_weight, total_weight, reserved
};

struct EmissiveLightSample {
    position: vec3<f32>,
    normal: vec3<f32>,
    emission: vec3<f32>,
    pdf_area: f32,
    instance_idx: u32,
};

struct VarianceData {
    mean: vec3<f32>,
    _pad0: u32,
    m2: vec3<f32>,
    count: u32,
};

// Stage G.A: ReSTIR-DI types. Layout must match the host-side `Sample`,
// `Reservoir`, `MotionVector` in `restir/reservoir.rs`. Declared here so
// Stage G.B can RIS-sample bounce 0 lights inside the megakernel.
struct Sample {
    position: vec3<f32>,
    valid: u32,
    wi: vec3<f32>,
    light_type: u32,
    radiance: vec3<f32>,
    dist: f32,
    normal: vec3<f32>,
    _pad: u32,
};

struct Reservoir {
    sample: Sample,
    w_sum: f32,
    m: u32,
    w: f32,
    _pad: u32,
};

struct MotionVector {
    motion: vec2<f32>,
    depth: f32,
    valid: u32,
};

@group(0) @binding(0) var<storage, read> nodes: array<BVHNode>;
@group(0) @binding(1) var<storage, read> instances: array<Instance>;
@group(0) @binding(2) var<uniform> camera: Camera;
@group(0) @binding(3) var output: texture_storage_2d<rgba32float, write>;
@group(0) @binding(4) var<storage, read_write> accum: array<vec4<f32>>;
@group(0) @binding(5) var<storage, read> materials: array<Material>;
@group(0) @binding(6) var env_map: texture_2d<f32>;
@group(0) @binding(7) var env_sampler: sampler;
@group(0) @binding(8) var<uniform> env: EnvParams;
@group(0) @binding(9) var<storage, read> env_marginal_cdf: array<f32>;
@group(0) @binding(10) var<storage, read> env_conditional_cdf: array<f32>;
@group(0) @binding(11) var<storage, read> sample_map: array<u32>;
@group(0) @binding(12) var<storage, read_write> variance: array<VarianceData>;
@group(0) @binding(13) var emissive_lights: texture_2d<f32>;
@group(0) @binding(14) var<uniform> emissive_light_params: EmissiveLightParams;
// Stage G.A plumbing. Bindings 15-17 will be exercised by Stage G.B
// (RIS) / G.C (temporal). For now they're declared so the megakernel BGL
// has slots ready and the host can already bind real ReSTIR buffers.
@group(0) @binding(15) var<storage, read_write> cur_reservoirs: array<Reservoir>;
@group(0) @binding(16) var<storage, read> prev_reservoirs: array<Reservoir>;
@group(0) @binding(17) var<storage, read> motion_vectors: array<MotionVector>;

// Vose alias table: each entry stores `(prob, alt)`. Picking a light
// is `i = rand * N`; if `rand2 < table[i].prob` return `i`, else
// return `table[i].alt`. Two memory loads regardless of N.
// (`alias` and `target` are reserved words in WGSL — field is `alt`.)
struct AliasEntry {
    prob: f32,
    alt: u32,
};
@group(0) @binding(18) var<storage, read> emissive_alias: array<AliasEntry>;
// Primary-hit AOVs for the OIDN denoiser. Accumulated across samples on
// `bounce == 0`: `.rgb` is the running sum of per-sample primary albedo /
// normal, `.w` is the sample count. The denoiser bridge divides at ingest.
// Without accumulation, sub-pixel jitter on edges leaves the AOV equal to
// whichever last sample landed there, while colour is averaged — that
// inconsistency makes OIDN paint false-edge artefacts.
@group(0) @binding(19) var<storage, read_write> albedo_aov: array<vec4<f32>>;
@group(0) @binding(20) var<storage, read_write> normal_aov: array<vec4<f32>>;

// LBVH morton-sort can produce branches deeper than log2(N) when the AABB
// distribution has many near-duplicates (which squarebob hits with many
// small files in a single dir → near-identical cube centres). 32 was too
// tight for 30K-instance scenes — a few rays per frame ran out of stack,
// silently returned no hit, and showed the env map through entire blocks
// of cubes that jittered frame-to-frame. 64 buys margin without hurting
// register pressure noticeably (256 B/thread).
const MAX_STACK_DEPTH: u32 = 64u;
const T_MAX: f32 = 1e30;
const EPSILON: f32 = 1e-6;
const PI: f32 = 3.14159265359;

const SUN_DIR: vec3<f32> = vec3<f32>(0.5, 0.8, 0.3);
const SUN_COLOR: vec3<f32> = vec3<f32>(1.0, 0.98, 0.95);
const SUN_INTENSITY: f32 = 5.0;
const SUN_ANGULAR_RADIUS: f32 = 0.00465;

// ---- RNG ----

fn pcg_hash(input: u32) -> u32 {
    var state = input * 747796405u + 2891336453u;
    let word = ((state >> ((state >> 28u) + 4u)) ^ state) * 277803737u;
    return (word >> 22u) ^ word;
}

fn rand(state: ptr<function, u32>) -> f32 {
    *state = pcg_hash(*state);
    return f32(*state) / 4294967296.0;
}

fn hash01(input: u32) -> f32 {
    return f32(pcg_hash(input)) / 4294967296.0;
}

// ---- ReSTIR-DI stubs (Stage G.A plumbing) ----
// Bodies fill in during Stage G.B (RIS) and G.C (temporal combine).
// Kept as standalone fns here so they appear in the module symbol
// table without altering bounce-0 NEE behaviour.

fn init_reservoir() -> Reservoir {
    var r: Reservoir;
    r.sample.valid = 0u;
    r.sample.position = vec3<f32>(0.0);
    r.sample.wi = vec3<f32>(0.0);
    r.sample.light_type = 0u;
    r.sample.radiance = vec3<f32>(0.0);
    r.sample.dist = 0.0;
    r.sample.normal = vec3<f32>(0.0);
    r.sample._pad = 0u;
    r.w_sum = 0.0;
    r.m = 0u;
    r.w = 0.0;
    r._pad = 0u;
    return r;
}

fn update_reservoir(
    r: ptr<function, Reservoir>,
    s: Sample,
    weight: f32,
    rng: ptr<function, u32>,
) -> bool {
    // Stage G.B will perform: w_sum += weight; m += 1; reservoir-pick s
    // with probability weight / w_sum. Left as a no-op for G.A.
    // WGSL phony assignments silence unused-param warnings without using
    // `let _ =` (reserved-identifier).
    _ = r;
    _ = s;
    _ = weight;
    _ = rng;
    return false;
}

fn combine_reservoirs(
    dst: ptr<function, Reservoir>,
    src: Reservoir,
    target_pdf: f32,
    rng: ptr<function, u32>,
) {
    // Stage G.C will combine `src` into `dst` using `target_pdf * src.w *
    // f32(src.m)`. No-op for G.A.
    _ = dst;
    _ = src;
    _ = target_pdf;
    _ = rng;
}

fn sample_pixel_jitter(pixel_idx: u32, frame_count: u32, rng: ptr<function, u32>) -> vec2<f32> {
    if camera.sampler_mode == 1u {
        let n = f32(frame_count + 1u);
        let scramble = vec2<f32>(
            hash01(pixel_idx ^ 0x9E3779B9u),
            hash01(pixel_idx ^ 0xBB67AE85u)
        );
        return fract(scramble + n * vec2<f32>(0.754877666, 0.569840296));
    }
    return vec2<f32>(rand(rng), rand(rng));
}

// Reconstruction-filter weight for a per-pixel sub-sample. Matches the
// per-pixel Gaussian reconstruction used by V-Ray / Octane / Cycles
// (without filter overlap into neighbouring pixels — that's Phase B).
//
// `jitter` is the sub-pixel position in `[0, 1)` returned by
// `sample_pixel_jitter`. We centre it at the pixel midpoint (`jitter
// - 0.5` → `[-0.5, +0.5]`) and apply a Gaussian with σ = 0.5 so that
// samples near the pixel centre carry the most weight while corner
// samples still contribute. The accumulator stores
// `sum(radiance * weight)` in `.rgb` and `sum(weight)` in `.w`; the
// final per-pixel colour is `accum.rgb / accum.w`.
//
// At σ = 0.5 the mean weight over the unit pixel is ≈ 0.73, so the
// effective "noise budget" of N samples is comparable to N box-filter
// samples but with the corner-sample variance suppressed — exactly the
// behaviour that fixes 2-3-pixel-cube silhouette noise.
fn pixel_filter_weight(jitter: vec2<f32>) -> f32 {
    let d = jitter - vec2<f32>(0.5, 0.5);
    let r2 = dot(d, d);
    // σ = 0.5 → 2σ² = 0.5 → exp(-r² / 0.5) = exp(-2 r²)
    return exp(-2.0 * r2);
}

// ---- Instance helpers ----

// Reconstruct mat4 from 4 column vectors stored in Instance.
fn inst_model_inv(inst: Instance) -> mat4x4<f32> {
    return mat4x4<f32>(
        inst.model_inv_0,
        inst.model_inv_1,
        inst.model_inv_2,
        inst.model_inv_3,
    );
}

// ---- Slice plane clipping ----

// Check if point should be clipped by slice plane
// Uses arbitrary normal direction via dot product
fn slice_clip(pos: vec3<f32>) -> bool {
    if camera.slice_enabled < 0.5 { return false; }
    // Distance from origin along slice normal
    let dist = dot(pos, camera.slice_normal) - camera.slice_position;
    if camera.slice_invert > 0.5 {
        return dist > 0.0;
    }
    return dist < 0.0;
}

// ---- Ray-box intersection ----

// Intersect unit cube [-0.5, 0.5]^3 in local space.
// Returns (t, normal_local). t < 0 means miss.
fn intersect_unit_cube(ray_o: vec3<f32>, ray_d: vec3<f32>) -> vec2<f32> {
    // Returns vec2(t_enter, t_exit). If t_enter > t_exit or t_exit < 0, miss.
    let inv_d = 1.0 / ray_d;
    let t0 = (vec3<f32>(-0.5) - ray_o) * inv_d;
    let t1 = (vec3<f32>(0.5) - ray_o) * inv_d;
    let tmin = min(t0, t1);
    let tmax = max(t0, t1);
    let t_enter = max(max(tmin.x, tmin.y), tmin.z);
    let t_exit = min(min(tmax.x, tmax.y), tmax.z);
    return vec2<f32>(t_enter, t_exit);
}

// Compute box face normal from hit point on unit cube.
fn box_normal(p: vec3<f32>) -> vec3<f32> {
    let a = abs(p);
    // Which face was hit: the coordinate closest to 0.5
    if a.x > a.y && a.x > a.z {
        return vec3<f32>(sign(p.x), 0.0, 0.0);
    } else if a.y > a.z {
        return vec3<f32>(0.0, sign(p.y), 0.0);
    } else {
        return vec3<f32>(0.0, 0.0, sign(p.z));
    }
}

// Ray-instance intersection: transform ray to local space, test unit cube.
fn intersect_instance(ray: Ray, inst_idx: u32) -> HitInfo {
    var hit: HitInfo;
    hit.hit = false;
    hit.t = T_MAX;
    hit.inst_idx = inst_idx;

    let inst = instances[inst_idx];
    let m_inv = inst_model_inv(inst);

    // Transform ray to local space
    let o_local = (m_inv * vec4<f32>(ray.origin, 1.0)).xyz;
    let d_local = (m_inv * vec4<f32>(ray.dir, 0.0)).xyz;

    let tt = intersect_unit_cube(o_local, d_local);
    let t_enter = tt.x;
    let t_exit = tt.y;

    if t_exit < 0.0 || t_enter > t_exit {
        return hit;
    }

    // Pick t: prefer enter, fall back to exit if inside
    let t_hit = select(t_enter, t_exit, t_enter < EPSILON);
    if t_hit < EPSILON { return hit; }

    // t is the same in local and world space (affine transform preserves t)
    hit.t = t_hit;

    // Compute world hit position and check slice plane
    let hit_world = ray.origin + ray.dir * t_hit;
    if slice_clip(hit_world) {
        return hit; // clipped by slice plane
    }

    // Compute normal in local space, transform to world
    let p_local = o_local + d_local * t_hit;
    let n_local = box_normal(p_local);
    // Normal transform: transpose(model_inv) * n (for non-uniform scale)
    let n_world = normalize((transpose(m_inv) * vec4<f32>(n_local, 0.0)).xyz);
    hit.normal = n_world;
    hit.hit = true;

    return hit;
}

// ---- BVH traversal ----

fn intersect_aabb(ray: Ray, inv_dir: vec3<f32>, node: BVHNode, t_best: f32) -> bool {
    let t1 = (node.aabb_min - ray.origin) * inv_dir;
    let t2 = (node.aabb_max - ray.origin) * inv_dir;
    let tmin = max(max(min(t1.x, t2.x), min(t1.y, t2.y)), min(t1.z, t2.z));
    let tmax = min(min(max(t1.x, t2.x), max(t1.y, t2.y)), max(t1.z, t2.z));
    return tmax >= max(tmin, 0.0) && tmin < t_best;
}

fn trace_ray(ray: Ray) -> HitInfo {
    var best: HitInfo;
    best.hit = false;
    best.t = T_MAX;

    let inv_dir = 1.0 / ray.dir;

    var stack: array<u32, MAX_STACK_DEPTH>;
    var sp: u32 = 1u;
    stack[0] = 0u;
    var loop_safety = 0u;

    while sp > 0u {
        loop_safety += 1u;
        if loop_safety > 4096u { break; } // Safety break

        sp -= 1u;
        let node = nodes[stack[sp]];

        if !intersect_aabb(ray, inv_dir, node, best.t) {
            continue;
        }

        if node.count > 0u {
            // Leaf: test instances
            for (var i = 0u; i < node.count; i++) {
                let hit = intersect_instance(ray, node.left_or_first + i);
                if hit.hit && hit.t < best.t {
                    best = hit;
                }
            }
        } else {
            if sp + 2u <= MAX_STACK_DEPTH {
                stack[sp] = node.left_or_first + 1u;
                sp += 1u;
                stack[sp] = node.left_or_first;
                sp += 1u;
            }
        }
    }

    return best;
}

fn trace_shadow_ray(ray: Ray, max_t: f32) -> bool {
    let inv_dir = 1.0 / ray.dir;
    var stack: array<u32, MAX_STACK_DEPTH>;
    var sp: u32 = 1u;
    stack[0] = 0u;
    var loop_safety = 0u;

    while sp > 0u {
        loop_safety += 1u;
        if loop_safety > 4096u { break; } // Safety break

        sp -= 1u;
        let node = nodes[stack[sp]];

        if !intersect_aabb(ray, inv_dir, node, max_t) {
            continue;
        }

        if node.count > 0u {
            for (var i = 0u; i < node.count; i++) {
                let hit = intersect_instance(ray, node.left_or_first + i);
                if hit.hit && hit.t < max_t && hit.t > EPSILON {
                    return true;
                }
            }
        } else {
            if sp + 2u <= MAX_STACK_DEPTH {
                stack[sp] = node.left_or_first + 1u;
                sp += 1u;
                stack[sp] = node.left_or_first;
                sp += 1u;
            }
        }
    }

    return false;
}

// ---- Sampling ----

fn cosine_hemisphere(r1: f32, r2: f32) -> vec3<f32> {
    let phi = 2.0 * PI * r1;
    let cos_theta = sqrt(r2);
    let sin_theta = sqrt(1.0 - r2);
    return vec3<f32>(cos(phi) * sin_theta, cos_theta, sin(phi) * sin_theta);
}

fn sample_ggx(r1: f32, r2: f32, alpha: f32) -> vec3<f32> {
    let a2 = alpha * alpha;
    let phi = 2.0 * PI * r1;
    let cos_theta = sqrt((1.0 - r2) / (1.0 + (a2 - 1.0) * r2));
    let sin_theta = sqrt(1.0 - cos_theta * cos_theta);
    return vec3<f32>(cos(phi) * sin_theta, cos_theta, sin(phi) * sin_theta);
}

fn ggx_d(ndoth: f32, alpha: f32) -> f32 {
    let a2 = alpha * alpha;
    let d = ndoth * ndoth * (a2 - 1.0) + 1.0;
    return a2 / (PI * d * d + EPSILON);
}

fn smith_g1(ndotv: f32, alpha: f32) -> f32 {
    let a2 = alpha * alpha;
    let denom = ndotv + sqrt(a2 + (1.0 - a2) * ndotv * ndotv);
    return 2.0 * ndotv / (denom + EPSILON);
}

fn fresnel_schlick(cos_theta: f32, f0: vec3<f32>) -> vec3<f32> {
    let t = 1.0 - cos_theta;
    let t2 = t * t;
    return f0 + (1.0 - f0) * (t2 * t2 * t);
}

fn onb_from_normal(n: vec3<f32>) -> mat3x3<f32> {
    var t: vec3<f32>;
    if abs(n.y) < 0.999 {
        t = normalize(cross(vec3<f32>(0.0, 1.0, 0.0), n));
    } else {
        t = normalize(cross(vec3<f32>(1.0, 0.0, 0.0), n));
    }
    let b = cross(n, t);
    return mat3x3<f32>(t, n, b);
}

fn sample_disk(r1: f32, r2: f32) -> vec2<f32> {
    let theta = 2.0 * PI * r1;
    let r = sqrt(r2);
    return vec2<f32>(r * cos(theta), r * sin(theta));
}

fn sample_sun_direction(rng: ptr<function, u32>) -> vec3<f32> {
    let sun_dir = normalize(SUN_DIR);
    var t: vec3<f32>;
    if abs(sun_dir.y) < 0.999 {
        t = normalize(cross(vec3<f32>(0.0, 1.0, 0.0), sun_dir));
    } else {
        t = normalize(cross(vec3<f32>(1.0, 0.0, 0.0), sun_dir));
    }
    let b = cross(sun_dir, t);
    let r1 = rand(rng);
    let r2 = rand(rng);
    let r = SUN_ANGULAR_RADIUS * sqrt(r1);
    let theta = 2.0 * PI * r2;
    return normalize(sun_dir + r * (cos(theta) * t + sin(theta) * b));
}

// ---- Camera ----

fn gen_ray(x: f32, y: f32, dims: vec2<f32>, jx: f32, jy: f32, rng: ptr<function, u32>) -> Ray {
    let ndc = vec2<f32>(
        (x + jx) / dims.x * 2.0 - 1.0,
        1.0 - (y + jy) / dims.y * 2.0,
    );

    let near = camera.inv_proj * vec4<f32>(ndc, -1.0, 1.0);
    let far  = camera.inv_proj * vec4<f32>(ndc,  1.0, 1.0);
    let near3 = near.xyz / near.w;
    let far3  = far.xyz / far.w;
    let origin_view = near3;
    let dir_view = normalize(far3 - near3);

    var ray: Ray;

    if camera.dof_enabled != 0u && camera.aperture > 0.0 {
        let t_focus = camera.focus_distance / max(abs(dir_view.z), 0.001);
        let focus_point_view = origin_view + dir_view * t_focus;
        let lens_sample = sample_disk(rand(rng), rand(rng)) * camera.aperture;
        let lens_origin_view = origin_view + vec3<f32>(lens_sample, 0.0);
        let new_dir_view = normalize(focus_point_view - lens_origin_view);
        ray.origin = (camera.inv_view * vec4<f32>(lens_origin_view, 1.0)).xyz;
        ray.dir = normalize((camera.inv_view * vec4<f32>(new_dir_view, 0.0)).xyz);
    } else {
        ray.origin = (camera.inv_view * vec4<f32>(origin_view, 1.0)).xyz;
        ray.dir = normalize((camera.inv_view * vec4<f32>(dir_view, 0.0)).xyz);
    }

    return ray;
}

// ---- Environment ----

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

// Atmospheric sky with Rayleigh + Mie scattering, day/night cycle
fn atmospheric_sky(dir: vec3<f32>, time: f32) -> vec3<f32> {
    // Sun orbits - full cycle every ~60 seconds
    let sun_angle = time * 0.1;
    let sun_height = sin(sun_angle) * 0.8 + 0.1;
    let sun_x = cos(sun_angle) * 0.6;
    let sun_z = sin(sun_angle * 0.7) * 0.4;
    let sun_dir = normalize(vec3<f32>(sun_x, sun_height, sun_z));
    let sun_dot = max(dot(dir, sun_dir), 0.0);

    // Rayleigh scattering (blue sky)
    let zenith = max(dir.y, 0.0);

    // Mie scattering (sun halo)
    let mie_g = 0.76;
    let mie_phase = (1.0 - mie_g * mie_g) / pow(1.0 + mie_g * mie_g - 2.0 * mie_g * sun_dot, 1.5);
    let mie = mie_phase * 0.003;

    // Optical depth
    let optical_depth = 1.0 / max(dir.y + 0.15, 0.05);
    let extinction = exp(-optical_depth * 0.3);

    // Day/night/sunset factors
    let day_factor = clamp(sun_dir.y * 2.0 + 0.5, 0.0, 1.0);
    let sunset_factor = clamp(1.0 - abs(sun_dir.y) * 3.0, 0.0, 1.0);

    // Sky color
    let day_sky = vec3<f32>(0.3, 0.5, 0.9);
    let sunset_sky = vec3<f32>(0.9, 0.4, 0.2);
    let night_sky = vec3<f32>(0.02, 0.02, 0.05);
    let sky_base = mix(night_sky, mix(day_sky, sunset_sky, sunset_factor), day_factor);
    let sky_blue = sky_base * (1.0 - extinction) * 2.0;

    // Horizon glow
    let horizon_color = mix(vec3<f32>(0.3, 0.2, 0.3), vec3<f32>(1.0, 0.5, 0.2), sunset_factor);
    let horizon_glow = horizon_color * (1.0 - zenith) * extinction * (0.3 + sunset_factor * 0.7);

    // Sun disk + corona + glow
    let sun_color = mix(vec3<f32>(1.0, 0.3, 0.1), vec3<f32>(1.0, 0.95, 0.9), clamp(sun_dir.y * 2.0, 0.0, 1.0));
    let sun_visible = select(0.0, 1.0, sun_dir.y > -0.1);
    let sun_disk = pow(sun_dot, 512.0) * sun_color * 20.0 * sun_visible;
    let corona = pow(sun_dot, 8.0) * sun_color * mie * 50.0 * sun_visible;
    let glow = pow(sun_dot, 2.0) * mix(vec3<f32>(0.5, 0.2, 0.1), vec3<f32>(0.4, 0.3, 0.2), day_factor) * (1.0 - zenith) * sun_visible;

    // Ground
    let ground_color = mix(vec3<f32>(0.02, 0.02, 0.03), vec3<f32>(0.1, 0.08, 0.06), day_factor);
    let ground = max(-dir.y, 0.0) * ground_color;

    // Stars at night
    let star_hash = fract(sin(dot(dir.xz, vec2<f32>(12.9898, 78.233))) * 43758.5453);
    let stars = select(0.0, star_hash * star_hash * 2.0, star_hash > 0.997) * (1.0 - day_factor);

    return sky_blue + horizon_glow + sun_disk + corona + glow + ground + vec3<f32>(stars);
}

// ---- Spectral helpers (approximate) ----

fn wavelength_to_rgb(lambda: f32) -> vec3<f32> {
    var r: f32 = 0.0;
    var g: f32 = 0.0;
    var b: f32 = 0.0;

    if lambda >= 380.0 && lambda < 440.0 {
        r = -(lambda - 440.0) / (440.0 - 380.0);
        g = 0.0;
        b = 1.0;
    } else if lambda >= 440.0 && lambda < 490.0 {
        r = 0.0;
        g = (lambda - 440.0) / (490.0 - 440.0);
        b = 1.0;
    } else if lambda >= 490.0 && lambda < 510.0 {
        r = 0.0;
        g = 1.0;
        b = -(lambda - 510.0) / (510.0 - 490.0);
    } else if lambda >= 510.0 && lambda < 580.0 {
        r = (lambda - 510.0) / (580.0 - 510.0);
        g = 1.0;
        b = 0.0;
    } else if lambda >= 580.0 && lambda < 645.0 {
        r = 1.0;
        g = -(lambda - 645.0) / (645.0 - 580.0);
        b = 0.0;
    } else if lambda >= 645.0 && lambda <= 720.0 {
        r = 1.0;
        g = 0.0;
        b = 0.0;
    }

    var factor: f32 = 1.0;
    if lambda > 700.0 {
        factor = 0.3 + 0.7 * (720.0 - lambda) / 20.0;
    } else if lambda < 420.0 {
        factor = 0.3 + 0.7 * (lambda - 380.0) / 40.0;
    }

    return vec3<f32>(r, g, b) * factor;
}

fn spectral_tint(rng: ptr<function, u32>) -> vec3<f32> {
    if camera.spectral_mode == 0u {
        return vec3<f32>(1.0);
    }
    let n = max(1u, camera.spectral_samples);
    var sum = vec3<f32>(0.0);
    for (var i = 0u; i < n; i++) {
        let l = mix(380.0, 720.0, rand(rng));
        sum += wavelength_to_rgb(l);
    }
    return sum / f32(n);
}

fn sky_color(dir: vec3<f32>) -> vec3<f32> {
    if env.enabled > 0.5 {
        let uv = dir_to_equirect_uv(dir, env.rotation);
        let color = textureSampleLevel(env_map, env_sampler, uv, 0.0).rgb;
        return color * env.intensity;
    } else {
        return atmospheric_sky(dir, env.time) * env.intensity;
    }
}

// ---- MIS helpers ----

fn mis_power_heuristic(pdf_a: f32, pdf_b: f32) -> f32 {
    let a2 = pdf_a * pdf_a;
    let b2 = pdf_b * pdf_b;
    return a2 / (a2 + b2 + EPSILON);
}

fn pdf_cosine_hemisphere(cos_theta: f32) -> f32 {
    return cos_theta / PI;
}

fn pdf_ggx(ndoth: f32, hdotv: f32, alpha: f32) -> f32 {
    let d = ggx_d(ndoth, alpha);
    return d * ndoth / (4.0 * hdotv + EPSILON);
}

fn clamp_firefly(color: vec3<f32>) -> vec3<f32> {
    let c = max(color, vec3<f32>(0.0));
    let lum = dot(c, vec3<f32>(0.2126, 0.7152, 0.0722));
    let max_lum = 1000.0;
    let scale = select(1.0, max_lum / max(lum, EPSILON), lum > max_lum);
    return c * scale;
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

fn pick_alias_index(rng: ptr<function, u32>, count: u32) -> u32 {
    if count <= 1u {
        return 0u;
    }
    let i = min(u32(rand(rng) * f32(count)), count - 1u);
    let entry = emissive_alias[i];
    if rand(rng) < entry.prob {
        return i;
    }
    return entry.alt;
}

fn sample_emissive_light(rng: ptr<function, u32>) -> EmissiveLightSample {
    let light_count = emissive_light_params.params0.z;
    let total_weight = max(emissive_light_params.params1.y, EPSILON);
    // O(1) light selection via Vose alias table — replaces the previous
    // O(N) linear scan that crippled scenes with thousands of emissives.
    let idx = pick_alias_index(rng, light_count);
    let selected = load_emissive_light(idx);

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
    let total_area = max(selected.center_area.w, EPSILON);

    let face_xi = rand(rng) * total_area;
    let u = rand(rng) - 0.5;
    let v = rand(rng) - 0.5;
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
    result.pdf_area = max(pdf_area, EPSILON);
    result.instance_idx = selected.instance_idx;
    return result;
}

fn emissive_light_pdf_solid(instance_idx: u32, dist2: f32, light_cos: f32) -> f32 {
    let light_count = emissive_light_params.params0.z;
    if light_count == 0u || light_cos <= 0.0 {
        return 0.0;
    }

    let total_weight = max(emissive_light_params.params1.y, EPSILON);
    for (var i = 0u; i < light_count; i++) {
        let light = load_emissive_light(i);
        if light.instance_idx == instance_idx {
            let area = max(light.center_area.w, EPSILON);
            let pdf_area = (light.emission_weight.w / total_weight) / area;
            return max(pdf_area * dist2 / max(light_cos, EPSILON), EPSILON);
        }
    }
    return 0.0;
}

// ---- Environment importance sampling ----

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
    let sin_theta = max(sin(PI * v), EPSILON);

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

    let pdf = max(marginal_pdf * conditional_pdf * f32(w) * f32(h) / (2.0 * PI * PI * sin_theta), EPSILON);
    return vec4<f32>(dir, pdf);
}

// ---- Path tracing kernel ----

@compute @workgroup_size(8, 8)
fn main(@builtin(global_invocation_id) gid: vec3<u32>) {
    let dims = textureDimensions(output);
    let w = dims.x;
    let h = dims.y;
    let px = gid.xy;

    if px.x >= w || px.y >= h {
        return;
    }

    let pixel_idx = px.y * w + px.x;
    
    // Check adaptive sampling limit. The accumulator's `.w` is now the
    // sum of reconstruction-filter weights (fractional), so we read the
    // per-pixel integer sample count from the variance Welford counter
    // instead — it gets `+= 1u` once per accepted dispatch (see
    // `var_data.count` below).
    let spp_limit = sample_map[pixel_idx];
    let current_samples = variance[pixel_idx].count;
    if spp_limit != 0u && current_samples >= spp_limit {
        return;
    }
    var rng = pcg_hash(pixel_idx * 1973u + camera.frame_count * 6133u + 1u);

    let pixel_jitter = sample_pixel_jitter(pixel_idx, camera.frame_count, &rng);
    let pixel_weight = pixel_filter_weight(pixel_jitter);
    var ray = gen_ray(
        f32(px.x),
        f32(px.y),
        vec2<f32>(f32(w), f32(h)),
        pixel_jitter.x,
        pixel_jitter.y,
        &rng
    );

    var throughput = vec3<f32>(1.0);
    var radiance = vec3<f32>(0.0);
    var transmission_depth = 0u;
    var last_bsdf_pdf = 0.0;
    var last_sample_was_transmission = false;

    for (var bounce = 0u; bounce <= camera.max_bounces; bounce++) {
        let hit = trace_ray(ray);

        if !hit.hit {
            if bounce == 0u {
                // Primary miss: zero AOV contribution, but the sample
                // still adds its filter weight so the per-pixel running
                // average stays at zero (instead of div-by-zero at
                // ingest) and remains consistent with the colour
                // reconstruction filter.
                albedo_aov[pixel_idx] += vec4<f32>(0.0, 0.0, 0.0, pixel_weight);
                normal_aov[pixel_idx] += vec4<f32>(0.0, 0.0, 0.0, pixel_weight);
            }
            radiance += throughput * sky_color(ray.dir);
            break;
        }

        let inst = instances[hit.inst_idx];
        let mat = materials[inst.material_id];
        let base_color = inst.color.rgb * mat.base_color_weight.rgb;
        let p = ray.origin + ray.dir * hit.t;

        // Ensure normal faces the ray
        var normal = hit.normal;
        if dot(normal, ray.dir) > 0.0 {
            normal = -normal;
        }

        if bounce == 0u {
            // Primary-hit AOVs for OIDN, accumulated across samples.
            // `.w` carries the per-pixel sample count; divide in the
            // denoiser bridge.
            //
            // Albedo contract per Intel OIDN: diffuse base for reflective
            // surfaces plus emission for emissive surfaces. Without the
            // emission term, lit pixels look like "noisy bright spots on
            // a dark surface" to the network, which then over-smooths
            // them into splotchy colored blocks. Metallic surfaces have
            // no diffuse component, so they fall through to pure
            // emission when self-lit.
            let primary_metallic = mat.params1.y;
            let primary_emission = mat.emission_color_weight.rgb * mat.emission_color_weight.a;
            let primary_albedo = base_color * (1.0 - primary_metallic) + primary_emission;
            // Weighted by the reconstruction-filter weight so AOVs share
            // the same spatial filter as colour — the denoiser then sees
            // matched spatial statistics on all three inputs.
            albedo_aov[pixel_idx] += vec4<f32>(primary_albedo * pixel_weight, pixel_weight);
            normal_aov[pixel_idx] += vec4<f32>(normal * pixel_weight, pixel_weight);
        }

        // Unpack material fields
        let opacity = mat.opacity.x;
        let go = env.global_opacity;

        let base_weight = mat.base_color_weight.a * go * opacity;
        let spec_color = mat.specular_color_weight.rgb;
        let spec_weight = mat.specular_color_weight.a * go * opacity;
        let transmission_color = mat.transmission_color_weight.rgb;
        let transmission_weight = mat.transmission_color_weight.a * go * opacity;
        let subsurface_color = mat.subsurface_color_weight.rgb;
        let subsurface_weight = mat.subsurface_color_weight.a * go * opacity;
        let coat_color = mat.coat_color_weight.rgb;
        let coat_weight = mat.coat_color_weight.a * go * opacity;
        let emission = mat.emission_color_weight.rgb * mat.emission_color_weight.a * opacity;
        let metallic = mat.params1.y;
        let roughness = max(mat.params1.z, 0.04);
        let ior = mat.params1.w;
        let dispersion = clamp(mat.params2.x, 0.0, 1.0);
        let coat_roughness = max(mat.params2.y, 0.04);
        let coat_ior = mat.params2.z;

        let diffuse_color = (base_color * base_weight + subsurface_color * subsurface_weight) * (1.0 - metallic);

        if max(max(emission.x, emission.y), emission.z) > 0.0 {
            var emission_mis = 1.0;
            if emissive_light_params.params0.x != 0u
                && bounce > 0u
                && !last_sample_was_transmission
                && last_bsdf_pdf > 0.0
            {
                let light_cos_hit = max(dot(normal, -ray.dir), 0.0);
                let light_pdf_hit =
                    emissive_light_pdf_solid(hit.inst_idx, hit.t * hit.t, light_cos_hit);
                if light_pdf_hit > 0.0 {
                    emission_mis = mis_power_heuristic(last_bsdf_pdf, light_pdf_hit);
                }
            }
            radiance += throughput * emission * emission_mis;
        }

        // Russian roulette after first bounce
        if bounce > 0u {
            let p_continue = max(max(throughput.x, throughput.y), throughput.z);
            if rand(&rng) > p_continue {
                break;
            }
            throughput /= p_continue;
        }

        let v_dir = -ray.dir;
        let ndotv = max(dot(normal, v_dir), EPSILON);
        let basis = onb_from_normal(normal);
        let f0_dielectric = vec3<f32>(pow((ior - 1.0) / (ior + 1.0), 2.0));
        let f0 = mix(f0_dielectric * spec_color, base_color, metallic);
        let alpha = roughness * roughness;
        let fresnel_estimate = fresnel_schlick(ndotv, f0);
        let fresnel_avg = (fresnel_estimate.x + fresnel_estimate.y + fresnel_estimate.z) / 3.0;
        let w_spec = spec_weight * fresnel_avg;
        let w_trans = transmission_weight * (1.0 - fresnel_avg);
        let w_diff = base_weight * (1.0 - metallic) * (1.0 - fresnel_avg);
        let w_total = w_spec + w_trans + w_diff + EPSILON;
        let p_spec = w_spec / w_total;
        let p_trans = w_trans / w_total;
        let p_diff = max(1.0 - p_spec - p_trans, EPSILON);
        let ior_r = ior * (1.0 + dispersion * 0.15);
        let ior_g = ior;
        let ior_b = ior * (1.0 - dispersion * 0.15);
        let trans_tint = vec3<f32>(ior_r, ior_g, ior_b) / max(ior, EPSILON);
        let transmission_color_disp = transmission_color * trans_tint;

        // NEE: direct sun light (opaque surfaces)
        if env.enabled < 0.5 {
            let sun_dir_sample = sample_sun_direction(&rng);
            let ndotl_sun = dot(normal, sun_dir_sample);

            if ndotl_sun > 0.0 {
                var shadow_ray: Ray;
                shadow_ray.origin = p + normal * 0.001;
                shadow_ray.dir = sun_dir_sample;

                if !trace_shadow_ray(shadow_ray, T_MAX) {
                    let f_sun = fresnel_schlick(ndotl_sun, f0);
                    let diffuse_contrib = diffuse_color * (1.0 - f_sun) * ndotl_sun / PI;

                    let alpha = roughness * roughness;
                    let h_sun = normalize(v_dir + sun_dir_sample);
                    let ndoth_sun = max(dot(normal, h_sun), EPSILON);
                    let hdotv_sun = max(dot(h_sun, v_dir), EPSILON);
                    let d_sun = ggx_d(ndoth_sun, alpha);
                    let g_sun = smith_g1(ndotv, alpha) * smith_g1(ndotl_sun, alpha);
                    let f_spec_sun = fresnel_schlick(hdotv_sun, f0);
                    let spec_contrib = spec_weight * f_spec_sun * d_sun * g_sun / (4.0 * ndotv * ndotl_sun + EPSILON);

                    radiance += throughput * (diffuse_contrib + spec_contrib) * SUN_COLOR * SUN_INTENSITY;
                }
            }
        }

        // NEE: HDR environment importance sampling (opaque only)
        if transmission_weight < 0.5 && env.enabled > 0.5 && env.use_importance_sampling > 0.5 {
            let env_sample = sample_env_direction(rand(&rng), rand(&rng));
            let env_dir = env_sample.xyz;
            let env_pdf = env_sample.w;
            let ndotl_env = dot(normal, env_dir);

            if ndotl_env > 0.0 {
                var shadow_ray: Ray;
                shadow_ray.origin = p + normal * 0.001;
                shadow_ray.dir = env_dir;

                if !trace_shadow_ray(shadow_ray, T_MAX) {
                    let env_radiance = sky_color(env_dir);
                    let f_env = fresnel_schlick(ndotl_env, f0);
                    let diffuse_contrib_env = diffuse_color * (1.0 - f_env) * ndotl_env / PI;

                    let h_env = normalize(v_dir + env_dir);
                    let ndoth_env = max(dot(normal, h_env), EPSILON);
                    let hdotv_env = max(dot(h_env, v_dir), EPSILON);
                    let d_env = ggx_d(ndoth_env, alpha);
                    let g_env = smith_g1(ndotv, alpha) * smith_g1(ndotl_env, alpha);
                    let f_spec_env = fresnel_schlick(hdotv_env, f0);
                    let spec_contrib_env = spec_weight * f_spec_env * d_env * g_env / (4.0 * ndotv * ndotl_env + EPSILON);

                    let pdf_diffuse_env = pdf_cosine_hemisphere(ndotl_env);
                    let pdf_spec_env = pdf_ggx(ndoth_env, hdotv_env, alpha);
                    let pdf_bsdf_env = p_diff * pdf_diffuse_env + p_spec * pdf_spec_env;

                    let mis_w = mis_power_heuristic(env_pdf, pdf_bsdf_env);
                    let env_contrib = (diffuse_contrib_env + spec_contrib_env) * env_radiance * mis_w / max(env_pdf, EPSILON);
                    radiance += throughput * env_contrib;
                }
            }
        }

        // NEE: explicit emissive cube sampling. This is the main variance
        // reduction path for scenes with many small neon/emissive cubes.
        //
        // Stage G.B: at bounce 0, if `emissive_light_params.params0.w != 0`
        // (host-driven ReSTIR-DI flag) we replace the multi-sample NEE
        // estimator with RIS over M candidates. RIS draws M (= params1.z)
        // candidate light samples from the same alias table, keeps a
        // running weighted reservoir, then shadow-tests ONLY the selected
        // sample. The unbiased RIS weight `W = w_sum / (m * target)` makes
        // the surviving sample's contribution equivalent in expectation to
        // the full NEE estimator, at much lower variance for high-M.
        //
        // The reservoir is also persisted to `cur_reservoirs[pixel_idx]`
        // so Stage G.C (temporal) can resample it next frame.
        if transmission_weight < 0.5
            && emissive_light_params.params0.x != 0u
            && emissive_light_params.params0.z > 0u
        {
            let restir_di = bounce == 0u && emissive_light_params.params0.w != 0u;
            if restir_di {
                let m_cand = max(1u, u32(emissive_light_params.params1.z));
                var r_sample: Sample;
                r_sample.position = vec3<f32>(0.0);
                r_sample.valid = 0u;
                r_sample.wi = vec3<f32>(0.0);
                r_sample.light_type = 1u; // emissive cube
                r_sample.radiance = vec3<f32>(0.0);
                r_sample.dist = 0.0;
                r_sample.normal = vec3<f32>(0.0);
                r_sample._pad = 0u;
                var r_w_sum: f32 = 0.0;
                var r_m: u32 = 0u;

                for (var k = 0u; k < m_cand; k++) {
                    let cand = sample_emissive_light(&rng);
                    if cand.instance_idx == hit.inst_idx { continue; }
                    let to_light = cand.position - p;
                    let dist2 = dot(to_light, to_light);
                    if dist2 <= EPSILON { continue; }
                    let dist = sqrt(dist2);
                    let light_dir = to_light / dist;
                    let ndotl_c = dot(normal, light_dir);
                    let light_cos_c = dot(cand.normal, -light_dir);
                    if ndotl_c <= 0.0 || light_cos_c <= 0.0 { continue; }
                    // Source pdf: alias-table picks proportional to power /
                    // area, converted to solid angle.
                    let pdf_solid =
                        max(cand.pdf_area * dist2 / max(light_cos_c, EPSILON), EPSILON);
                    // Target function: luminance(emission) * cos_theta — a
                    // cheap proxy for the full radiance × BSDF × cos with no
                    // visibility (visibility is checked once on the
                    // selected sample below).
                    let lum_emission = max(
                        dot(cand.emission, vec3<f32>(0.2126, 0.7152, 0.0722)),
                        0.0,
                    );
                    let p_target = lum_emission * ndotl_c;
                    let w_i = p_target / pdf_solid;
                    r_m += 1u;
                    r_w_sum += w_i;
                    if rand(&rng) * r_w_sum < w_i {
                        r_sample.position = cand.position;
                        r_sample.wi = light_dir;
                        r_sample.radiance = cand.emission;
                        r_sample.dist = dist;
                        r_sample.normal = cand.normal;
                        r_sample.valid = 1u;
                    }
                }

                // Unbiased RIS weight for the selected sample.
                var r_w: f32 = 0.0;
                if r_sample.valid != 0u && r_w_sum > 0.0 && r_m > 0u {
                    let lum_sel = max(
                        dot(r_sample.radiance, vec3<f32>(0.2126, 0.7152, 0.0722)),
                        0.0,
                    );
                    let target_sel = lum_sel * max(dot(normal, r_sample.wi), 0.0);
                    if target_sel > 0.0 {
                        r_w = r_w_sum / (f32(r_m) * target_sel);
                    }
                }

                // Shadow ray on the selected sample only — the win vs
                // multi-sample NEE is exactly this: one shadow ray, best-M
                // candidate.
                if r_sample.valid != 0u && r_w > 0.0 {
                    var shadow_ray: Ray;
                    shadow_ray.origin = p + normal * 0.001;
                    shadow_ray.dir = r_sample.wi;
                    if !trace_shadow_ray(shadow_ray, max(r_sample.dist - 0.002, EPSILON)) {
                        let ndotl = max(dot(normal, r_sample.wi), 0.0);
                        let f_light = fresnel_schlick(ndotl, f0);
                        let diffuse_contrib = diffuse_color * (1.0 - f_light) * ndotl / PI;
                        let h_l = normalize(v_dir + r_sample.wi);
                        let ndoth_l = max(dot(normal, h_l), EPSILON);
                        let hdotv_l = max(dot(h_l, v_dir), EPSILON);
                        let d_l = ggx_d(ndoth_l, alpha);
                        let g_l = smith_g1(ndotv, alpha) * smith_g1(ndotl, alpha);
                        let f_spec_l = fresnel_schlick(hdotv_l, f0);
                        let spec_contrib =
                            spec_weight * f_spec_l * d_l * g_l * ndotl
                            / (4.0 * ndotv * ndotl + EPSILON);
                        radiance += throughput
                            * (diffuse_contrib + spec_contrib)
                            * r_sample.radiance
                            * r_w;
                    } else {
                        // Occluded — drop the sample so Stage G.C temporal
                        // doesn't propagate a visibility lie.
                        r_sample.valid = 0u;
                    }
                }

                // Persist reservoir for temporal reuse next frame.
                var out_res: Reservoir;
                out_res.sample = r_sample;
                out_res.w_sum = r_w_sum;
                out_res.m = r_m;
                out_res.w = r_w;
                out_res._pad = 0u;
                cur_reservoirs[pixel_idx] = out_res;
            } else {
                let light_spp = max(1u, emissive_light_params.params0.y);
                for (var light_sample_idx = 0u; light_sample_idx < light_spp; light_sample_idx++) {
                    let light_sample = sample_emissive_light(&rng);
                    if light_sample.instance_idx == hit.inst_idx {
                        continue;
                    }

                    let to_light = light_sample.position - p;
                    let dist2 = dot(to_light, to_light);
                    if dist2 <= EPSILON {
                        continue;
                    }

                    let dist = sqrt(dist2);
                    let light_dir = to_light / dist;
                    let ndotl = dot(normal, light_dir);
                    let light_cos = dot(light_sample.normal, -light_dir);
                    if ndotl <= 0.0 || light_cos <= 0.0 {
                        continue;
                    }

                    var shadow_ray: Ray;
                    shadow_ray.origin = p + normal * 0.001;
                    shadow_ray.dir = light_dir;

                    if !trace_shadow_ray(shadow_ray, max(dist - 0.002, EPSILON)) {
                        let f_light = fresnel_schlick(ndotl, f0);
                        let diffuse_contrib_light = diffuse_color * (1.0 - f_light) * ndotl / PI;

                        let h_light = normalize(v_dir + light_dir);
                        let ndoth_light = max(dot(normal, h_light), EPSILON);
                        let hdotv_light = max(dot(h_light, v_dir), EPSILON);
                        let d_light = ggx_d(ndoth_light, alpha);
                        let g_light = smith_g1(ndotv, alpha) * smith_g1(ndotl, alpha);
                        let f_spec_light = fresnel_schlick(hdotv_light, f0);
                        let spec_contrib_light =
                            spec_weight * f_spec_light * d_light * g_light * ndotl
                            / (4.0 * ndotv * ndotl + EPSILON);

                        let pdf_light_solid =
                            max(light_sample.pdf_area * dist2 / max(light_cos, EPSILON), EPSILON);
                        let pdf_diffuse_light = pdf_cosine_hemisphere(ndotl);
                        let pdf_spec_light = pdf_ggx(ndoth_light, hdotv_light, alpha);
                        let pdf_bsdf_light = p_diff * pdf_diffuse_light + p_spec * pdf_spec_light;
                        let mis_w = mis_power_heuristic(pdf_light_solid, pdf_bsdf_light);

                        radiance += throughput
                            * (diffuse_contrib_light + spec_contrib_light)
                            * light_sample.emission
                            * (mis_w / (pdf_light_solid * f32(light_spp)));
                    }
                }
            }
        }

        // Coat layer (clearcoat)
        if coat_weight > 0.001 {
            let coat_f0 = vec3<f32>(pow((coat_ior - 1.0) / (coat_ior + 1.0), 2.0));
            let coat_fresnel = fresnel_schlick(ndotv, coat_f0);
            let coat_reflect_prob = coat_weight * (coat_fresnel.x + coat_fresnel.y + coat_fresnel.z) / 3.0;

            if rand(&rng) < coat_reflect_prob {
                let coat_alpha = coat_roughness * coat_roughness;
                let h_local = sample_ggx(rand(&rng), rand(&rng), coat_alpha);
                let h_world = normalize(basis * h_local);
                let hdotv = max(dot(h_world, v_dir), EPSILON);
                let reflect_dir = reflect(-v_dir, h_world);
                let ndotl = dot(normal, reflect_dir);

                if ndotl > 0.0 {
                    let ndoth = max(dot(normal, h_world), EPSILON);
                    let f = fresnel_schlick(hdotv, coat_f0);
                    let g = smith_g1(ndotv, coat_alpha) * smith_g1(ndotl, coat_alpha);
                    let weight = f * g * hdotv / (ndotv * ndoth + EPSILON);
                    throughput *= coat_color * weight / max(coat_reflect_prob, EPSILON);
                    ray.origin = p + normal * 0.001;
                    ray.dir = normalize(reflect_dir);
                    last_bsdf_pdf = max(coat_reflect_prob, EPSILON)
                        * pdf_ggx(ndoth, hdotv, coat_alpha);
                    last_sample_was_transmission = false;
                    continue;
                }
            }
            throughput *= 1.0 - coat_weight * coat_fresnel;
        }

        // Specular vs Transmission vs Diffuse sampling
        let lobe_rand = rand(&rng);

        if lobe_rand < p_spec {
            // GGX specular reflection
            let r1 = rand(&rng);
            let r2 = rand(&rng);
            let h_local = sample_ggx(r1, r2, alpha);
            let h_world = normalize(basis * h_local);
            let hdotv = max(dot(h_world, v_dir), EPSILON);
            let reflect_dir = reflect(-v_dir, h_world);
            let ndotl = dot(normal, reflect_dir);

            if ndotl <= 0.0 { break; }

            let ndoth = max(dot(normal, h_world), EPSILON);
            let f = fresnel_schlick(hdotv, f0);
            let g = smith_g1(ndotv, alpha) * smith_g1(ndotl, alpha);
            let weight = f * g * hdotv / (ndotv * ndoth + EPSILON);

            throughput *= weight / max(p_spec, EPSILON);
            ray.origin = p + normal * 0.001;
            ray.dir = normalize(reflect_dir);
            last_bsdf_pdf = max(p_spec, EPSILON) * pdf_ggx(ndoth, hdotv, alpha);
            last_sample_was_transmission = false;
        } else if lobe_rand < p_spec + p_trans {
            // Transmission / refraction
            if transmission_depth >= camera.max_transmission_depth {
                break;
            }
            transmission_depth += 1u;
            let eta = select(ior, 1.0 / ior, dot(normal, ray.dir) < 0.0);
            let h_local = sample_ggx(rand(&rng), rand(&rng), alpha);
            let h_world = normalize(basis * h_local);
            let cos_i = dot(v_dir, h_world);
            let sin2_t = eta * eta * (1.0 - cos_i * cos_i);
            if sin2_t > 1.0 {
                let reflect_dir = reflect(-v_dir, h_world);
                throughput *= transmission_color_disp / max(p_trans, EPSILON);
                ray.origin = p + normal * 0.001;
                ray.dir = normalize(reflect_dir);
                last_bsdf_pdf = 0.0;
                last_sample_was_transmission = true;
            } else {
                let cos_t = sqrt(1.0 - sin2_t);
                let refr_dir = normalize(eta * -v_dir + (eta * cos_i - cos_t) * h_world);
                throughput *= transmission_color_disp / max(p_trans, EPSILON);
                ray.origin = p - normal * 0.001;
                ray.dir = refr_dir;
                last_bsdf_pdf = 0.0;
                last_sample_was_transmission = true;
            }
        } else {
            // Lambert diffuse
            let r1 = rand(&rng);
            let r2 = rand(&rng);
            let local_dir = cosine_hemisphere(r1, r2);
            let world_dir = basis * local_dir;

            let f_diffuse = fresnel_schlick(max(dot(normal, normalize(world_dir)), 0.0), f0);
            let diff_weight = diffuse_color * (1.0 - f_diffuse);

            let diffuse_pdf = pdf_cosine_hemisphere(max(dot(normal, normalize(world_dir)), 0.0));
            throughput *= diff_weight / p_diff;
            ray.origin = p + normal * 0.001;
            ray.dir = normalize(world_dir);
            last_bsdf_pdf = p_diff * diffuse_pdf;
            last_sample_was_transmission = false;
        }
    }

    // Spectral tint (approximate)
    if camera.spectral_mode != 0u {
        let base_tint = spectral_tint(&rng);
        let dispersion_weight = f32(transmission_depth) * 0.15;
        let dispersion_mix = select(0.0, dispersion_weight, camera.spectral_dispersion != 0u);
        let cool_tint = vec3<f32>(base_tint.z, base_tint.y, base_tint.x);
        let tint = mix(base_tint, cool_tint, clamp(dispersion_mix, 0.0, 1.0));
        radiance *= tint;
    }

    // Progressive accumulation with per-pixel reconstruction-filter
    // weights (Gaussian, σ = 0.5). The accumulator now stores
    // `sum(radiance * w)` in `.rgb` and `sum(w)` in `.w`; the displayed
    // colour is `.rgb / .w`, which gives proper filter integration and
    // sharply suppresses silhouette-edge variance on tiny (2-3 pixel)
    // geometry that box-filtering cannot resolve.
    //
    // Firefly clamp stays on the per-sample radiance (not on the
    // running sum) — clamping the sum would bias the average downward
    // as more samples land.
    let sample_radiance = clamp_firefly(radiance);

    let prev = accum[pixel_idx];
    let new_accum = prev + vec4<f32>(sample_radiance * pixel_weight, pixel_weight);

    // Welford on raw radiance (no weighting): variance estimates per
    // sample of underlying signal, what adaptive sampling needs. Also
    // doubles as our integer sample-count source for the adaptive cap
    // at the top of this entry point.
    var var_data = variance[pixel_idx];
    var_data.count += 1u;
    let var_n = f32(var_data.count);
    let var_delta = sample_radiance - var_data.mean;
    var_data.mean += var_delta / var_n;
    let var_delta2 = sample_radiance - var_data.mean;
    var_data.m2 += var_delta * var_delta2;
    variance[pixel_idx] = var_data;

    accum[pixel_idx] = new_accum;

    // Filter-normalised output: divide by sum of weights, not sample
    // count. `max(...,1e-6)` keeps the first-sample case finite for
    // pixels whose first jitter happened to land exactly at a corner.
    let weight_sum = max(new_accum.w, 1e-6);
    let avg_color = new_accum.rgb / weight_sum;
    textureStore(output, vec2<i32>(px), vec4<f32>(avg_color, 1.0));
}
