// Shared ReSTIR-DI ABI, receiver material state, and target function.

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

struct RestirSurface {
    position: vec3<f32>,
    instance_id: u32,
    normal: vec3<f32>,
    material_id: u32,
    view: vec3<f32>,
    valid: u32,
    diffuse_color: vec3<f32>,
    roughness: f32,
    f0: vec3<f32>,
    specular_weight: f32,
    opacity: f32,
    _pad0: f32,
    _pad1: f32,
    _pad2: f32,
}

struct Reservoir {
    sample: Sample,
    w_sum: f32,
    m: u32,
    w: f32,
    _pad: u32,
    surface: RestirSurface,
}

const RESTIR_PI: f32 = 3.14159265359;
const RESTIR_EPSILON: f32 = 1e-6;

fn restir_empty_surface() -> RestirSurface {
    return RestirSurface(
        vec3<f32>(0.0),
        0xffffffffu,
        vec3<f32>(0.0),
        0xffffffffu,
        vec3<f32>(0.0),
        0u,
        vec3<f32>(0.0),
        1.0,
        vec3<f32>(0.0),
        0.0,
        0.0,
        0.0,
        0.0,
        0.0,
    );
}

fn restir_make_surface(
    instance: Instance,
    material: Material,
    position: vec3<f32>,
    normal: vec3<f32>,
    view: vec3<f32>,
    materialize_mix: f32,
    instance_id: u32,
) -> RestirSurface {
    let base_color = mix(
        instance.color.rgb,
        material.base_color_weight.rgb,
        clamp(materialize_mix, 0.0, 1.0),
    );
    let metallic = clamp(material.params1.y, 0.0, 1.0);
    let ior = safe_ior(material.params1.w);
    let dielectric_f0 = vec3<f32>(pow((ior - 1.0) / (ior + 1.0), 2.0));
    let f0 = mix(
        dielectric_f0 * max(material.specular_color_weight.rgb, vec3<f32>(0.0)),
        max(base_color, vec3<f32>(0.0)),
        metallic,
    );
    return RestirSurface(
        position,
        instance_id,
        normalize(normal),
        instance.material_id,
        normalize(view),
        1u,
        max(base_color * material.base_color_weight.a, vec3<f32>(0.0))
            * (1.0 - metallic),
        max(material.params1.z, 0.04),
        max(f0, vec3<f32>(0.0)),
        max(material.specular_color_weight.a, 0.0),
        clamp(material.opacity.x, 0.0, 1.0),
        0.0,
        0.0,
        0.0,
    );
}

fn restir_surfaces_compatible(receiver: RestirSurface, candidate: RestirSurface) -> bool {
    return receiver.valid != 0u
        && candidate.valid != 0u
        && receiver.instance_id == candidate.instance_id
        && receiver.material_id == candidate.material_id;
}

fn restir_luminance(value: vec3<f32>) -> f32 {
    return max(dot(value, vec3<f32>(0.2126, 0.7152, 0.0722)), 0.0);
}

fn restir_sample_direction_at(sample: Sample, surface_position: vec3<f32>) -> vec3<f32> {
    if sample.light_type == 1u {
        let to_light = sample.position - surface_position;
        let distance_squared = dot(to_light, to_light);
        if distance_squared <= 1e-20 {
            return vec3<f32>(0.0);
        }
        return to_light * inverseSqrt(distance_squared);
    }

    let direction_squared = dot(sample.wi, sample.wi);
    if direction_squared <= 1e-20 {
        return vec3<f32>(0.0);
    }
    return sample.wi * inverseSqrt(direction_squared);
}

fn restir_fresnel_schlick(cos_theta: f32, f0: vec3<f32>) -> vec3<f32> {
    return f0 + (vec3<f32>(1.0) - f0) * pow(1.0 - cos_theta, 5.0);
}

fn restir_ggx_d(ndoth: f32, alpha: f32) -> f32 {
    let alpha_squared = alpha * alpha;
    let denominator = ndoth * ndoth * (alpha_squared - 1.0) + 1.0;
    return alpha_squared / (RESTIR_PI * denominator * denominator + RESTIR_EPSILON);
}

fn restir_smith_g1(ndotv: f32, alpha: f32) -> f32 {
    let alpha_squared = alpha * alpha;
    let ndotv_squared = ndotv * ndotv;
    return 2.0 * ndotv
        / (
            ndotv
            + sqrt(alpha_squared + ndotv_squared - alpha_squared * ndotv_squared)
            + RESTIR_EPSILON
        );
}

fn restir_bsdf_at(surface: RestirSurface, wi: vec3<f32>) -> vec3<f32> {
    if surface.valid == 0u {
        return vec3<f32>(0.0);
    }
    let normal = normalize(surface.normal);
    let view = normalize(surface.view);
    let cos_theta = max(dot(normal, wi), 0.0);
    let ndotv = max(dot(normal, view), 0.0);
    if cos_theta <= 0.0 || ndotv <= 0.0 {
        return vec3<f32>(0.0);
    }
    let half_vector_squared = dot(view + wi, view + wi);
    if half_vector_squared <= 1e-20 {
        return vec3<f32>(0.0);
    }
    let half_vector = (view + wi) * inverseSqrt(half_vector_squared);
    let ndoth = max(dot(normal, half_vector), RESTIR_EPSILON);
    let hdotv = max(dot(half_vector, view), RESTIR_EPSILON);
    let alpha = surface.roughness * surface.roughness;
    let fresnel = restir_fresnel_schlick(hdotv, surface.f0);
    let distribution = restir_ggx_d(ndoth, alpha);
    let geometry = restir_smith_g1(ndotv, alpha)
        * restir_smith_g1(cos_theta, alpha);
    let specular = surface.specular_weight * fresnel * distribution * geometry
        / max(4.0 * ndotv * cos_theta, RESTIR_EPSILON);
    let diffuse = surface.diffuse_color * (vec3<f32>(1.0) - fresnel) / RESTIR_PI;
    return max((diffuse + specular) * surface.opacity, vec3<f32>(0.0));
}

fn restir_contribution_at(sample: Sample, surface: RestirSurface) -> vec3<f32> {
    if sample.valid == 0u || surface.valid == 0u {
        return vec3<f32>(0.0);
    }
    let wi = restir_sample_direction_at(sample, surface.position);
    let cosine = max(dot(normalize(surface.normal), wi), 0.0);
    return max(sample.radiance, vec3<f32>(0.0))
        * restir_bsdf_at(surface, wi)
        * cosine;
}

fn restir_target_at(sample: Sample, surface: RestirSurface) -> f32 {
    return restir_luminance(restir_contribution_at(sample, surface));
}

fn restir_update_reservoir(
    reservoir: ptr<function, Reservoir>,
    sample: Sample,
    weight: f32,
    seed: ptr<function, u32>,
) {
    if weight <= 0.0 || isNan(weight) || isInf(weight) {
        return;
    }
    (*reservoir).w_sum += weight;
    (*reservoir).m += 1u;
    if rand(seed) * (*reservoir).w_sum < weight {
        (*reservoir).sample = sample;
    }
}

fn restir_combine_reservoirs(
    receiver: ptr<function, Reservoir>,
    candidate: Reservoir,
    target_at_receiver: f32,
    seed: ptr<function, u32>,
) {
    if candidate.sample.valid == 0u || candidate.m == 0u
        || candidate.w <= 0.0 || target_at_receiver <= 0.0
    {
        return;
    }
    let weight = target_at_receiver * candidate.w * f32(candidate.m);
    if isNan(weight) || isInf(weight) || weight <= 0.0 {
        return;
    }
    (*receiver).w_sum += weight;
    (*receiver).m += candidate.m;
    if rand(seed) * (*receiver).w_sum < weight {
        (*receiver).sample = candidate.sample;
    }
}

fn restir_finalize_reservoir(reservoir: ptr<function, Reservoir>) {
    let target = restir_target_at((*reservoir).sample, (*reservoir).surface);
    if (*reservoir).m > 0u && (*reservoir).w_sum > 0.0 && target > 0.0 {
        (*reservoir).w = (*reservoir).w_sum / (f32((*reservoir).m) * target);
        if !isNan((*reservoir).w) && !isInf((*reservoir).w) {
            return;
        }
    }
    (*reservoir).w = 0.0;
}
