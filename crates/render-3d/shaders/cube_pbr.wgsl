// PBR instanced cube shader for dirstat-rs 3D treemap
// Adapted from alembic-rs Standard Surface for instanced rendering
// Features: 3-point lighting, GGX specular, Oren-Nayar diffuse, IBL, flat shading, xray mode

const PI: f32 = 3.141592653589793;
const PI_INV: f32 = 0.3183098861837907;
const EPSILON: f32 = 1e-6;

// ============================================================================
// Uniforms
// ============================================================================

struct Camera {
    view_proj: mat4x4<f32>,
    view: mat4x4<f32>,
    inv_view_proj: mat4x4<f32>,
    position: vec3<f32>,
    xray_alpha: f32,       // 1.0 = opaque, <1.0 = xray transparent
    flat_shading: f32,     // 1.0 = face normals via dpdx/dpdy
    slice_enabled: f32,    // 1.0 = slice plane active
    slice_position: f32,   // distance from origin along normal
    slice_invert: f32,     // 1.0 = invert side
    slice_normal: vec3<f32>, // Slice plane normal (normalized)
    _pad: vec2<f32>,
}

struct Light {
    direction: vec3<f32>,
    _pad: f32,
    color: vec3<f32>,
    intensity: f32,
}

// 3-point lighting rig (key, fill, rim)
struct LightRig {
    key: Light,
    fill: Light,
    rim: Light,
    ambient: vec3<f32>,
    _pad: f32,
}

// Global PBR material params (same for all cubes, per-instance color overrides base_color)
struct MaterialParams {
    roughness: f32,
    metalness: f32,
    specular_ior: f32,
    specular_weight: f32,
}

@group(0) @binding(0) var<uniform> camera: Camera;
@group(0) @binding(1) var<uniform> lights: LightRig;
@group(0) @binding(2) var<uniform> material: MaterialParams;

// Environment map (equirectangular HDR)
struct EnvParams {
    intensity: f32,
    rotation: f32,
    enabled: f32,
    _pad: f32,
}

@group(1) @binding(0) var env_map: texture_2d<f32>;
@group(1) @binding(1) var env_sampler: sampler;
@group(1) @binding(2) var<uniform> env: EnvParams;

// ============================================================================
// Vertex I/O
// ============================================================================

struct VertexInput {
    @location(0) position: vec3<f32>,
    @location(1) normal: vec3<f32>,
}

struct InstanceInput {
    @location(2) model_0: vec4<f32>,
    @location(3) model_1: vec4<f32>,
    @location(4) model_2: vec4<f32>,
    @location(5) model_3: vec4<f32>,
    @location(6) color: vec4<f32>,
    @location(7) hash: u32,
    @location(8) object_id: u32,
}

struct VertexOutput {
    @builtin(position) clip_position: vec4<f32>,
    @location(0) world_pos: vec3<f32>,
    @location(1) world_normal: vec3<f32>,
    @location(2) color: vec4<f32>,
    @location(3) @interpolate(flat) object_id: u32,
}

// ============================================================================
// BRDF Functions (from MaterialX / alembic-rs standard-surface)
// ============================================================================

fn pow5(x: f32) -> f32 {
    let x2 = x * x;
    return x2 * x2 * x;
}

fn ior_to_f0(ior: f32) -> f32 {
    let r = (ior - 1.0) / (ior + 1.0);
    return r * r;
}

// Schlick Fresnel approximation
fn fresnel_schlick(cos_theta: f32, F0: vec3<f32>) -> vec3<f32> {
    return F0 + (vec3<f32>(1.0) - F0) * pow5(1.0 - cos_theta);
}

// GGX Normal Distribution Function
fn distribution_ggx(NdotH: f32, roughness: f32) -> f32 {
    let a = roughness * roughness;
    let a2 = a * a;
    let denom = NdotH * NdotH * (a2 - 1.0) + 1.0;
    return a2 / (PI * denom * denom);
}

// Height-correlated Smith GGX geometry term
fn geometry_smith(NdotV: f32, NdotL: f32, roughness: f32) -> f32 {
    let a = roughness * roughness;
    let a2 = a * a;
    let lv = sqrt(a2 + (1.0 - a2) * NdotV * NdotV);
    let ll = sqrt(a2 + (1.0 - a2) * NdotL * NdotL);
    return 2.0 * NdotV * NdotL / (lv * NdotL + ll * NdotV + EPSILON);
}

// Oren-Nayar diffuse (rough surfaces)
fn oren_nayar(NdotV: f32, NdotL: f32, LdotV: f32, roughness: f32) -> f32 {
    let s2 = roughness * roughness;
    let A = 1.0 - 0.5 * s2 / (s2 + 0.33);
    let B = 0.45 * s2 / (s2 + 0.09);
    let s = LdotV - NdotL * NdotV;
    var t: f32;
    if s > 0.0 { t = max(NdotL, NdotV); } else { t = 1.0; }
    return A + B * s / t;
}

// Equirectangular environment map sampling
fn dir_to_equirect_uv(dir: vec3<f32>, rotation: f32) -> vec2<f32> {
    let d = normalize(dir);
    let phi = atan2(d.z, d.x) + rotation;
    let theta = acos(clamp(d.y, -1.0, 1.0));
    return vec2<f32>((phi + PI) / (2.0 * PI), theta / PI);
}

fn sample_env(dir: vec3<f32>) -> vec3<f32> {
    if env.enabled < 0.5 { return vec3<f32>(0.0); }
    let uv = dir_to_equirect_uv(dir, env.rotation);
    return textureSample(env_map, env_sampler, uv).rgb * env.intensity;
}

// ============================================================================
// Lighting
// ============================================================================

struct LightContrib {
    diffuse: vec3<f32>,
    specular: vec3<f32>,
}

// Compute PBR lighting from a single directional light
fn compute_light(
    light: Light, N: vec3<f32>, V: vec3<f32>, NdotV: f32,
    base: vec3<f32>, F0: vec3<f32>,
    roughness: f32, metalness: f32, specular: f32,
) -> LightContrib {
    var r: LightContrib;
    r.diffuse = vec3<f32>(0.0);
    r.specular = vec3<f32>(0.0);
    if light.intensity < EPSILON { return r; }

    let L = normalize(-light.direction);
    let H = normalize(V + L);
    let NdotL = max(dot(N, L), 0.0);
    if NdotL < EPSILON { return r; }

    let NdotH = max(dot(N, H), 0.0);
    let VdotH = max(dot(V, H), 0.0);
    let LdotV = max(dot(L, V), 0.0);

    // Diffuse: Oren-Nayar
    let diff = oren_nayar(NdotV, NdotL, LdotV, roughness) * PI_INV;

    // Specular: Cook-Torrance (GGX)
    let D = distribution_ggx(NdotH, roughness);
    let G = geometry_smith(NdotV, NdotL, roughness);
    let F = fresnel_schlick(VdotH, F0);
    let spec_brdf = (D * G * F) / (4.0 * NdotV * NdotL + EPSILON);

    // Energy conservation
    let kS = F;
    let kD = (vec3<f32>(1.0) - kS) * (1.0 - metalness);

    let radiance = light.color * light.intensity;
    r.diffuse = kD * base * diff * radiance * NdotL;
    r.specular = spec_brdf * specular * radiance * NdotL;
    return r;
}

// ============================================================================
// Vertex Shader
// ============================================================================

@vertex
fn vs_main(v: VertexInput, i: InstanceInput) -> VertexOutput {
    let model = mat4x4<f32>(i.model_0, i.model_1, i.model_2, i.model_3);
    let wp = model * vec4<f32>(v.position, 1.0);
    
    // Transform normal - normalize removes scale distortion for uniform-ish scales
    // For highly non-uniform scale, use flat shading in fragment shader instead
    let wn = normalize((model * vec4<f32>(v.normal, 0.0)).xyz);

    var out: VertexOutput;
    out.clip_position = camera.view_proj * wp;
    out.world_pos = wp.xyz;
    out.world_normal = wn;
    out.color = i.color;
    out.object_id = i.object_id;
    return out;
}

// ============================================================================
// Forward PBR Fragment (main rendering path)
// ============================================================================

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

@fragment
fn fs_main(in: VertexOutput, @builtin(front_facing) front: bool) -> @location(0) vec4<f32> {
    // Slice plane clipping
    if slice_clip(in.world_pos) { discard; }

    let V = normalize(camera.position - in.world_pos);

    // Normal: flat (dpdx/dpdy face normals) or smooth (interpolated vertex normals)
    var N: vec3<f32>;
    if camera.flat_shading > 0.5 {
        N = normalize(cross(dpdx(in.world_pos), dpdy(in.world_pos)));
    } else {
        N = normalize(in.world_normal);
    }
    if !front { N = -N; }

    let NdotV = max(dot(N, V), EPSILON);
    let base = in.color.rgb;
    let roughness = max(material.roughness, 0.04);
    let metalness = material.metalness;
    let specular = material.specular_weight;

    // F0: dielectric from IOR, metallic from base color
    let dielectric_F0 = vec3<f32>(ior_to_f0(material.specular_ior));
    let F0 = mix(dielectric_F0, base, metalness);

    var diff_acc = vec3<f32>(0.0);
    var spec_acc = vec3<f32>(0.0);

    // 3-point lighting rig
    let key = compute_light(lights.key, N, V, NdotV, base, F0, roughness, metalness, specular);
    diff_acc += key.diffuse;
    spec_acc += key.specular;

    let fill = compute_light(lights.fill, N, V, NdotV, base, F0, roughness, metalness, specular);
    diff_acc += fill.diffuse;
    spec_acc += fill.specular;

    let rim = compute_light(lights.rim, N, V, NdotV, base, F0, roughness, metalness, specular);
    diff_acc += rim.diffuse;
    spec_acc += rim.specular;

    // Environment IBL (hemisphere irradiance + specular reflection)
    if env.enabled > 0.5 {
        // Diffuse IBL: hemisphere sampling around normal
        let env_n = sample_env(N);
        let T = normalize(cross(N, select(vec3<f32>(0.0, 1.0, 0.0), vec3<f32>(1.0, 0.0, 0.0), abs(N.y) > 0.9)));
        let B = cross(N, T);
        let e1 = sample_env(normalize(N + T * 0.7));
        let e2 = sample_env(normalize(N - T * 0.7));
        let e3 = sample_env(normalize(N + B * 0.7));
        let e4 = sample_env(normalize(N - B * 0.7));
        let env_up = sample_env(vec3<f32>(0.0, 1.0, 0.0));
        let env_diff = env_n * 0.35 + (e1 + e2 + e3 + e4) * 0.125 + env_up * 0.15;
        diff_acc += env_diff * base * (1.0 - metalness) * 0.5;

        // Specular IBL: reflection sampling
        let R = reflect(-V, N);
        let env_spec = sample_env(R);
        let fresnel = F0 + (vec3<f32>(1.0) - F0) * pow5(1.0 - NdotV);
        spec_acc += env_spec * fresnel * (1.0 - roughness * roughness);
    } else {
        // Fallback flat ambient
        diff_acc += lights.ambient * base;
    }

    // XRay mode: diffuse fades with alpha, specular stays bright
    let color = diff_acc * camera.xray_alpha + spec_acc;
    let spec_bright = max(spec_acc.r, max(spec_acc.g, spec_acc.b));
    let alpha = max(in.color.a * camera.xray_alpha, min(spec_bright, 1.0));

    return vec4<f32>(color, alpha);
}

// ============================================================================
// G-Buffer Fragment (deferred path)
// ============================================================================

struct GBufferOut {
    @location(0) albedo_roughness: vec4<f32>,   // RGB = base color, A = roughness
    @location(1) normal_metalness: vec4<f32>,   // RGB = encoded normal [0,1], A = metalness
}

@fragment
fn fs_gbuffer(in: VertexOutput, @builtin(front_facing) front: bool) -> GBufferOut {
    // Slice plane clipping
    if slice_clip(in.world_pos) { discard; }

    var N: vec3<f32>;
    if camera.flat_shading > 0.5 {
        N = normalize(cross(dpdx(in.world_pos), dpdy(in.world_pos)));
    } else {
        N = normalize(in.world_normal);
    }
    if !front { N = -N; }

    var out: GBufferOut;
    out.albedo_roughness = vec4<f32>(in.color.rgb, material.roughness);
    out.normal_metalness = vec4<f32>(N * 0.5 + 0.5, material.metalness);
    return out;
}

// ============================================================================
// Wireframe Fragment
// ============================================================================

@fragment
fn fs_wireframe(in: VertexOutput) -> @location(0) vec4<f32> {
    // Slice plane clipping
    if slice_clip(in.world_pos) { discard; }

    let mix_factor = clamp(in.color.a, 0.0, 1.0) * 0.4;
    let tint = mix(vec3<f32>(1.0, 1.0, 1.0), in.color.rgb, mix_factor);
    return vec4<f32>(tint, 1.0);
}
