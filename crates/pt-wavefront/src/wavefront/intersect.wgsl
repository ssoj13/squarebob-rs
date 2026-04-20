// Wavefront Intersection pass.
// BVH traversal for all active rays.

struct BvhNode {
    min: vec3<f32>,
    left_or_first: u32,
    max: vec3<f32>,
    count: u32,
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
}

// Reconstruct mat4 from 4 column vectors.
fn inst_model_inv(inst: Instance) -> mat4x4<f32> {
    return mat4x4<f32>(
        inst.model_inv_0,
        inst.model_inv_1,
        inst.model_inv_2,
        inst.model_inv_3,
    );
}

// Invert a mat4 (for computing model from model_inv).
// Simple cofactor-based inversion for affine transforms.
fn mat4_inverse(m: mat4x4<f32>) -> mat4x4<f32> {
    let a00 = m[0][0]; let a01 = m[0][1]; let a02 = m[0][2]; let a03 = m[0][3];
    let a10 = m[1][0]; let a11 = m[1][1]; let a12 = m[1][2]; let a13 = m[1][3];
    let a20 = m[2][0]; let a21 = m[2][1]; let a22 = m[2][2]; let a23 = m[2][3];
    let a30 = m[3][0]; let a31 = m[3][1]; let a32 = m[3][2]; let a33 = m[3][3];

    let b00 = a00 * a11 - a01 * a10;
    let b01 = a00 * a12 - a02 * a10;
    let b02 = a00 * a13 - a03 * a10;
    let b03 = a01 * a12 - a02 * a11;
    let b04 = a01 * a13 - a03 * a11;
    let b05 = a02 * a13 - a03 * a12;
    let b06 = a20 * a31 - a21 * a30;
    let b07 = a20 * a32 - a22 * a30;
    let b08 = a20 * a33 - a23 * a30;
    let b09 = a21 * a32 - a22 * a31;
    let b10 = a21 * a33 - a23 * a31;
    let b11 = a22 * a33 - a23 * a32;

    let det = b00 * b11 - b01 * b10 + b02 * b09 + b03 * b08 - b04 * b07 + b05 * b06;
    let inv_det = 1.0 / det;

    return mat4x4<f32>(
        vec4<f32>(
            (a11 * b11 - a12 * b10 + a13 * b09) * inv_det,
            (a02 * b10 - a01 * b11 - a03 * b09) * inv_det,
            (a31 * b05 - a32 * b04 + a33 * b03) * inv_det,
            (a22 * b04 - a21 * b05 - a23 * b03) * inv_det,
        ),
        vec4<f32>(
            (a12 * b08 - a10 * b11 - a13 * b07) * inv_det,
            (a00 * b11 - a02 * b08 + a03 * b07) * inv_det,
            (a32 * b02 - a30 * b05 - a33 * b01) * inv_det,
            (a20 * b05 - a22 * b02 + a23 * b01) * inv_det,
        ),
        vec4<f32>(
            (a10 * b10 - a11 * b08 + a13 * b06) * inv_det,
            (a01 * b08 - a00 * b10 - a03 * b06) * inv_det,
            (a30 * b04 - a31 * b02 + a33 * b00) * inv_det,
            (a21 * b02 - a20 * b04 - a23 * b00) * inv_det,
        ),
        vec4<f32>(
            (a11 * b07 - a10 * b09 - a12 * b06) * inv_det,
            (a00 * b09 - a01 * b07 + a02 * b06) * inv_det,
            (a31 * b01 - a30 * b03 - a32 * b00) * inv_det,
            (a20 * b03 - a21 * b01 + a22 * b00) * inv_det,
        ),
    );
}

struct Ray {
    origin: vec3<f32>,
    pixel_id: u32,
    dir: vec3<f32>,
    bounce: u32,
    throughput: vec3<f32>,
    flags: u32,
}

// Layout must match Rust WfHit (32 bytes)
struct Hit {
    t: f32,              // offset 0
    instance_id: u32,    // offset 4
    _pad: vec2<u32>,     // offset 8 (padding for vec3 alignment)
    normal: vec3<f32>,   // offset 16
    hit: u32,            // offset 28
}

@group(0) @binding(0) var<storage, read> nodes: array<BvhNode>;
@group(0) @binding(1) var<storage, read> instances: array<Instance>;
@group(0) @binding(2) var<storage, read> rays: array<Ray>;
@group(0) @binding(3) var<storage, read_write> hits: array<Hit>;
@group(0) @binding(4) var<storage, read_write> count: array<atomic<u32>>;

// Ray-AABB intersection
fn ray_aabb(ray_o: vec3<f32>, ray_inv_d: vec3<f32>, bmin: vec3<f32>, bmax: vec3<f32>, t_max: f32) -> f32 {
    let t1 = (bmin - ray_o) * ray_inv_d;
    let t2 = (bmax - ray_o) * ray_inv_d;
    let tmin = max(max(min(t1.x, t2.x), min(t1.y, t2.y)), min(t1.z, t2.z));
    let tmax = min(min(max(t1.x, t2.x), max(t1.y, t2.y)), max(t1.z, t2.z));
    if tmax < 0.0 || tmin > tmax || tmin > t_max { return 1e30; }
    return tmin;
}

// Ray-cube intersection (unit cube [-0.5, 0.5]^3)
fn ray_cube(ray_o: vec3<f32>, ray_d: vec3<f32>, t_max: f32) -> vec2<f32> {
    let inv_d = 1.0 / ray_d;
    let t1 = (-0.5 - ray_o) * inv_d;
    let t2 = (0.5 - ray_o) * inv_d;
    let tmin = max(max(min(t1.x, t2.x), min(t1.y, t2.y)), min(t1.z, t2.z));
    let tmax = min(min(max(t1.x, t2.x), max(t1.y, t2.y)), max(t1.z, t2.z));
    if tmax < 0.0 || tmin > tmax || tmin > t_max { return vec2<f32>(1e30, 0.0); }
    return vec2<f32>(max(tmin, 0.0001), 1.0);
}

// Get cube normal at hit point
fn cube_normal(p: vec3<f32>) -> vec3<f32> {
    let ap = abs(p);
    if ap.x > ap.y && ap.x > ap.z { return vec3<f32>(sign(p.x), 0.0, 0.0); }
    if ap.y > ap.z { return vec3<f32>(0.0, sign(p.y), 0.0); }
    return vec3<f32>(0.0, 0.0, sign(p.z));
}

@compute @workgroup_size(64)
fn main(@builtin(global_invocation_id) gid: vec3<u32>) {
    let ray_count = atomicLoad(&count[0]);
    if gid.x >= ray_count { return; }

    let ray = rays[gid.x];
    if (ray.flags & 1u) == 0u {
        hits[gid.x] = Hit(1e30, 0u, vec2<u32>(0u, 0u), vec3<f32>(0.0), 0u);
        return;
    }

    let ray_inv_d = 1.0 / ray.dir;
    var closest_t = 1e30;
    var closest_inst = 0u;
    var closest_normal = vec3<f32>(0.0);

    // BVH traversal stack
    var stack: array<u32, 32>;
    var sp = 0;
    stack[sp] = 0u;
    sp += 1;

    while sp > 0 {
        sp -= 1;
        let node_idx = stack[sp];
        let node = nodes[node_idx];

        // Skip if ray misses node
        if ray_aabb(ray.origin, ray_inv_d, node.min, node.max, closest_t) >= closest_t {
            continue;
        }

        if node.count > 0u {
            // Leaf node - test instances
            for (var i = 0u; i < node.count; i++) {
                let inst_idx = node.left_or_first + i;
                let inst = instances[inst_idx];

                // Transform ray to instance space
                let inv_model = inst_model_inv(inst);
                let local_o = (inv_model * vec4<f32>(ray.origin, 1.0)).xyz;
                let local_d = (inv_model * vec4<f32>(ray.dir, 0.0)).xyz;

                let hit_info = ray_cube(local_o, local_d, closest_t);
                if hit_info.y > 0.0 && hit_info.x < closest_t {
                    closest_t = hit_info.x;
                    closest_inst = inst_idx;
                    let local_hit = local_o + local_d * hit_info.x;
                    let local_n = cube_normal(local_hit);
                    // Transform normal back to world space using model matrix
                    let model = mat4_inverse(inv_model);
                    closest_normal = normalize((model * vec4<f32>(local_n, 0.0)).xyz);
                }
            }
        } else {
            // Internal node - push children
            stack[sp] = node.left_or_first;
            sp += 1;
            stack[sp] = node.left_or_first + 1u;
            sp += 1;
        }
    }

    hits[gid.x] = Hit(
        closest_t,
        closest_inst,
        vec2<u32>(0u, 0u),  // padding
        closest_normal,
        select(0u, 1u, closest_t < 1e20)
    );
}
