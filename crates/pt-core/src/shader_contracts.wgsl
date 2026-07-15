// Shared path-tracing contracts.
//
// Keep host validation at upload boundaries. These guards protect shader math from
// malformed external material data and provide one ray-origin policy everywhere.

const PT_DEFAULT_IOR: f32 = 1.5;
const PT_MIN_IOR: f32 = 1.0;
const PT_MAX_IOR: f32 = 4.0;

fn safe_ior(value: f32) -> f32 {
    if isNan(value) || isInf(value) {
        return PT_DEFAULT_IOR;
    }
    return clamp(value, PT_MIN_IOR, PT_MAX_IOR);
}

// Ray Tracing Gems, Chapter 6: scale-aware offset plus an integer ULP shift.
// The small-origin branch avoids denormalized bit manipulation near zero.
fn offset_ray_component(position: f32, normal: f32) -> f32 {
    const ORIGIN: f32 = 1.0 / 32.0;
    const FLOAT_SCALE: f32 = 1.0 / 65536.0;
    const INT_SCALE: f32 = 256.0;

    if abs(position) < ORIGIN {
        return position + FLOAT_SCALE * normal;
    }

    let offset = i32(INT_SCALE * normal);
    let bits = bitcast<i32>(position);
    let shifted_bits = select(bits - offset, bits + offset, position >= 0.0);
    return bitcast<f32>(shifted_bits);
}

fn offset_ray_origin(
    position: vec3<f32>,
    geometric_normal: vec3<f32>,
    outgoing_direction: vec3<f32>,
) -> vec3<f32> {
    let oriented_normal = select(
        -geometric_normal,
        geometric_normal,
        dot(geometric_normal, outgoing_direction) >= 0.0,
    );
    return vec3<f32>(
        offset_ray_component(position.x, oriented_normal.x),
        offset_ray_component(position.y, oriented_normal.y),
        offset_ray_component(position.z, oriented_normal.z),
    );
}

fn shadow_ray_max_t(
    origin: vec3<f32>,
    target_position: vec3<f32>,
    target_normal: vec3<f32>,
    direction: vec3<f32>,
) -> f32 {
    let target = offset_ray_origin(target_position, target_normal, -direction);
    return distance(origin, target);
}
