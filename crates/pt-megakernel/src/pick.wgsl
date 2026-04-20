// BVH ray picking compute shader (single ray)
//
// Uses same BVH node/instance layout as path tracer.

struct BVHNode {
    aabb_min: vec3<f32>,
    left_or_first: u32,
    aabb_max: vec3<f32>,
    count: u32,
};

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

struct RayPick {
    origin: vec3<f32>,
    _pad0: f32,
    dir: vec3<f32>,
    _pad1: f32,
};

struct Ray {
    origin: vec3<f32>,
    dir: vec3<f32>,
};

struct HitInfo {
    t: f32,
    inst_idx: u32,
    hit: u32,
    _pad: u32,
};

struct PickResult {
    object_id: u32,
    hit: u32,
    _pad0: u32,
    _pad1: u32,
    t: f32,
    _pad2: vec3<f32>,
};

@group(0) @binding(0) var<storage, read> nodes: array<BVHNode>;
@group(0) @binding(1) var<storage, read> instances: array<Instance>;
@group(0) @binding(2) var<uniform> pick: RayPick;
@group(0) @binding(3) var<storage, read_write> out_hit: PickResult;

const MAX_STACK_DEPTH: u32 = 32u;
const T_MAX: f32 = 1e30;
const EPSILON: f32 = 1e-6;

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

fn intersect_instance(ray: Ray, inst_idx: u32) -> HitInfo {
    var hit: HitInfo;
    hit.hit = 0u;
    hit.t = T_MAX;
    hit.inst_idx = inst_idx;

    let inst = instances[inst_idx];
    let m_inv = inst_model_inv(inst);

    let o_local = (m_inv * vec4<f32>(ray.origin, 1.0)).xyz;
    let d_local = (m_inv * vec4<f32>(ray.dir, 0.0)).xyz;

    let tt = intersect_unit_cube(o_local, d_local);
    let t_enter = tt.x;
    let t_exit = tt.y;

    if t_exit < 0.0 || t_enter > t_exit {
        return hit;
    }

    let t_hit = select(t_enter, t_exit, t_enter < EPSILON);
    if t_hit < EPSILON { return hit; }

    hit.t = t_hit;
    hit.hit = 1u;
    return hit;
}

fn intersect_aabb(ray: Ray, inv_dir: vec3<f32>, node: BVHNode, t_best: f32) -> bool {
    let t1 = (node.aabb_min - ray.origin) * inv_dir;
    let t2 = (node.aabb_max - ray.origin) * inv_dir;
    let tmin = max(max(min(t1.x, t2.x), min(t1.y, t2.y)), min(t1.z, t2.z));
    let tmax = min(min(max(t1.x, t2.x), max(t1.y, t2.y)), max(t1.z, t2.z));
    return tmax >= max(tmin, 0.0) && tmin < t_best;
}

fn trace_ray(ray: Ray) -> HitInfo {
    var best: HitInfo;
    best.hit = 0u;
    best.t = T_MAX;
    best.inst_idx = 0u;

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
            for (var i = 0u; i < node.count; i++) {
                let hit = intersect_instance(ray, node.left_or_first + i);
                if hit.hit == 1u && hit.t < best.t {
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

@compute @workgroup_size(1, 1, 1)
fn main() {
    var ray: Ray;
    ray.origin = pick.origin;
    ray.dir = normalize(pick.dir);

    let hit = trace_ray(ray);
    if hit.hit == 1u {
        let inst = instances[hit.inst_idx];
        out_hit.object_id = inst.object_id;
        out_hit.t = hit.t;
        out_hit.hit = 1u;
    } else {
        out_hit.object_id = 0u;
        out_hit.t = 0.0;
        out_hit.hit = 0u;
    }
}
