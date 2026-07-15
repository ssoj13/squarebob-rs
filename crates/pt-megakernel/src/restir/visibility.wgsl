// Shared ReSTIR-DI visibility traversal.
// Ray, Instance, nodes, and instances are supplied by initial/shade.

struct BvhNode {
    aabb_min: vec3<f32>,
    left_or_first: u32,
    aabb_max: vec3<f32>,
    count: u32,
}

const RESTIR_SHADOW_EPSILON: f32 = 1e-6;
const RESTIR_SHADOW_STACK_DEPTH: u32 = 32u;

fn restir_instance_model_inverse(instance: Instance) -> mat4x4<f32> {
    return mat4x4<f32>(
        instance.model_inv_0,
        instance.model_inv_1,
        instance.model_inv_2,
        instance.model_inv_3,
    );
}

fn restir_intersect_unit_cube(ray_origin: vec3<f32>, ray_direction: vec3<f32>) -> vec2<f32> {
    let inverse_direction = 1.0 / ray_direction;
    let t0 = (vec3<f32>(-0.5) - ray_origin) * inverse_direction;
    let t1 = (vec3<f32>(0.5) - ray_origin) * inverse_direction;
    let t_minimum = min(t0, t1);
    let t_maximum = max(t0, t1);
    return vec2<f32>(
        max(max(t_minimum.x, t_minimum.y), t_minimum.z),
        min(min(t_maximum.x, t_maximum.y), t_maximum.z),
    );
}

fn restir_intersect_instance_shadow(ray: Ray, instance_index: u32, max_t: f32) -> bool {
    let instance = instances[instance_index];
    let inverse_model = restir_instance_model_inverse(instance);
    let local_origin = (inverse_model * vec4<f32>(ray.origin, 1.0)).xyz;
    let local_direction = (inverse_model * vec4<f32>(ray.dir, 0.0)).xyz;
    let interval = restir_intersect_unit_cube(local_origin, local_direction);
    if interval.y < 0.0 || interval.x > interval.y {
        return false;
    }
    let hit_t = select(interval.x, interval.y, interval.x < RESTIR_SHADOW_EPSILON);
    return hit_t > RESTIR_SHADOW_EPSILON && hit_t < max_t;
}

fn restir_intersect_aabb(
    ray: Ray,
    inverse_direction: vec3<f32>,
    node: BvhNode,
    max_t: f32,
) -> bool {
    let t1 = (node.aabb_min - ray.origin) * inverse_direction;
    let t2 = (node.aabb_max - ray.origin) * inverse_direction;
    let t_minimum = max(max(min(t1.x, t2.x), min(t1.y, t2.y)), min(t1.z, t2.z));
    let t_maximum = min(min(max(t1.x, t2.x), max(t1.y, t2.y)), max(t1.z, t2.z));
    return t_maximum >= max(t_minimum, 0.0) && t_minimum < max_t;
}

fn restir_trace_shadow_ray(ray: Ray, max_t: f32) -> bool {
    let inverse_direction = 1.0 / ray.dir;
    var stack: array<u32, RESTIR_SHADOW_STACK_DEPTH>;
    var stack_size = 1u;
    stack[0] = 0u;
    var traversal_overflow = false;

    while stack_size > 0u {
        stack_size -= 1u;
        let node = nodes[stack[stack_size]];
        if !restir_intersect_aabb(ray, inverse_direction, node, max_t) {
            continue;
        }
        if node.count > 0u {
            for (var index = 0u; index < node.count; index++) {
                if restir_intersect_instance_shadow(
                    ray,
                    node.left_or_first + index,
                    max_t,
                ) {
                    return true;
                }
            }
        } else {
            if stack_size + 2u > RESTIR_SHADOW_STACK_DEPTH {
                traversal_overflow = true;
                break;
            }
            stack[stack_size] = node.left_or_first + 1u;
            stack_size += 1u;
            stack[stack_size] = node.left_or_first;
            stack_size += 1u;
        }
    }

    if traversal_overflow {
        for (
            var instance_index = 0u;
            instance_index < arrayLength(&instances);
            instance_index++
        ) {
            if restir_intersect_instance_shadow(ray, instance_index, max_t) {
                return true;
            }
        }
    }
    return false;
}
