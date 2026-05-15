// PBR instanced cube shader for squarebob-rs 3D treemap
// Adapted from alembic-rs Standard Surface for instanced rendering
// Features: per-instance material lookup via material_id, 3-point lighting,
// GGX specular, Oren-Nayar diffuse, IBL, flat shading, xray mode, emission

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
    xray_alpha: f32,
    flat_shading: f32,
    slice_enabled: f32,
    slice_position: f32,
    slice_invert: f32,
    slice_normal: vec3<f32>,
    _pad: vec2<f32>,
}

struct Light {
    direction: vec3<f32>,
    _pad: f32,
    color: vec3<f32>,
    intensity: f32,
}

struct LightRig {
    key: Light,
    fill: Light,
    rim: Light,
    ambient: vec3<f32>,
    _pad: f32,
}

// Per-instance material — mirrors `pt_core::bvh::GpuMaterial` layout (Standard Surface).
struct GpuMaterial {
    base_color_weight: vec4<f32>,         // rgb=base color, a=weight
    specular_color_weight: vec4<f32>,     // rgb=specular tint, a=specular weight
    transmission_color_weight: vec4<f32>, // unused in PBR raster (PT only)
    subsurface_color_weight: vec4<f32>,   // unused in PBR raster
    coat_color_weight: vec4<f32>,         // unused in PBR raster
    emission_color_weight: vec4<f32>,     // rgb=emission color, a=intensity
    opacity: vec4<f32>,                   // rgb=opacity (PT), a unused
    params1: vec4<f32>,                   // x=diff_rough, y=metal, z=spec_rough, w=spec_IOR
    params2: vec4<f32>,                   // x=spec_aniso, y=coat_rough, z=coat_IOR, w=visible
};

// Global material params shared by all instances — kept tiny so the materialize
// slider stays live without rebuilding the cube instance buffer.
// Three trailing f32 pads (instead of vec3) keep WGSL std140 size at 16 bytes,
// matching the Rust `MatGlobalUniform` exactly (vec3 would round up to 32).
struct MatGlobal {
    materialize_mix: f32,  // 0=ignore mat library albedo, 1=use mat library albedo
    _pad0: f32,
    _pad1: f32,
    _pad2: f32,
};

@group(0) @binding(0) var<uniform> camera: Camera;
@group(0) @binding(1) var<uniform> lights: LightRig;
@group(0) @binding(2) var<storage, read> materials: array<GpuMaterial>;
@group(0) @binding(3) var<uniform> mat_global: MatGlobal;

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
    @location(9) material_id: u32,
}

struct VertexOutput {
    @builtin(position) clip_position: vec4<f32>,
    @location(0) world_pos: vec3<f32>,
    @location(1) world_normal: vec3<f32>,
    @location(2) color: vec4<f32>,
    @location(3) @interpolate(flat) object_id: u32,
    @location(4) @interpolate(flat) material_id: u32,
}

// ============================================================================
// BRDF Functions
// ============================================================================

fn pow5(x: f32) -> f32 {
    let x2 = x * x;
    return x2 * x2 * x;
}

fn ior_to_f0(ior: f32) -> f32 {
    let r = (ior - 1.0) / (ior + 1.0);
    return r * r;
}

fn fresnel_schlick(cos_theta: f32, F0: vec3<f32>) -> vec3<f32> {
    return F0 + (vec3<f32>(1.0) - F0) * pow5(1.0 - cos_theta);
}

fn distribution_ggx(NdotH: f32, roughness: f32) -> f32 {
    let a = roughness * roughness;
    let a2 = a * a;
    let denom = NdotH * NdotH * (a2 - 1.0) + 1.0;
    return a2 / (PI * denom * denom);
}

fn geometry_smith(NdotV: f32, NdotL: f32, roughness: f32) -> f32 {
    let a = roughness * roughness;
    let a2 = a * a;
    let lv = sqrt(a2 + (1.0 - a2) * NdotV * NdotV);
    let ll = sqrt(a2 + (1.0 - a2) * NdotL * NdotL);
    return 2.0 * NdotV * NdotL / (lv * NdotL + ll * NdotV + EPSILON);
}

fn oren_nayar(NdotV: f32, NdotL: f32, LdotV: f32, roughness: f32) -> f32 {
    let s2 = roughness * roughness;
    let A = 1.0 - 0.5 * s2 / (s2 + 0.33);
    let B = 0.45 * s2 / (s2 + 0.09);
    let s = LdotV - NdotL * NdotV;
    var t: f32;
    if s > 0.0 { t = max(NdotL, NdotV); } else { t = 1.0; }
    return A + B * s / t;
}

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

    let diff = oren_nayar(NdotV, NdotL, LdotV, roughness) * PI_INV;

    let D = distribution_ggx(NdotH, roughness);
    let G = geometry_smith(NdotV, NdotL, roughness);
    let F = fresnel_schlick(VdotH, F0);
    let spec_brdf = (D * G * F) / (4.0 * NdotV * NdotL + EPSILON);

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
    let wn = normalize((model * vec4<f32>(v.normal, 0.0)).xyz);

    var out: VertexOutput;
    out.clip_position = camera.view_proj * wp;
    out.world_pos = wp.xyz;
    out.world_normal = wn;
    out.color = i.color;
    out.object_id = i.object_id;
    out.material_id = i.material_id;
    return out;
}

// ============================================================================
// Fragment helpers
// ============================================================================

fn slice_clip(pos: vec3<f32>) -> bool {
    if camera.slice_enabled < 0.5 { return false; }
    let dist = dot(pos, camera.slice_normal) - camera.slice_position;
    if camera.slice_invert > 0.5 {
        return dist > 0.0;
    }
    return dist < 0.0;
}

// Per-fragment material handle: blends per-instance color tint with library albedo.
struct ResolvedMat {
    albedo: vec3<f32>,
    roughness: f32,
    metalness: f32,
    specular_weight: f32,
    ior: f32,
    emission: vec3<f32>,
}

fn resolve_material(mat_id: u32, instance_color: vec3<f32>) -> ResolvedMat {
    let m = materials[mat_id];
    var r: ResolvedMat;
    r.albedo = mix(instance_color, m.base_color_weight.rgb, mat_global.materialize_mix);
    r.roughness = max(m.params1.z, 0.04);  // specular roughness
    r.metalness = clamp(m.params1.y, 0.0, 1.0);
    r.specular_weight = m.specular_color_weight.a;
    r.ior = m.params1.w;
    r.emission = m.emission_color_weight.rgb * m.emission_color_weight.a;
    return r;
}

// ============================================================================
// Forward PBR Fragment (main rendering path)
// ============================================================================

@fragment
fn fs_main(in: VertexOutput, @builtin(front_facing) front: bool) -> @location(0) vec4<f32> {
    if slice_clip(in.world_pos) { discard; }

    let V = normalize(camera.position - in.world_pos);

    var N: vec3<f32>;
    if camera.flat_shading > 0.5 {
        N = normalize(cross(dpdx(in.world_pos), dpdy(in.world_pos)));
    } else {
        N = normalize(in.world_normal);
    }
    if !front { N = -N; }

    let NdotV = max(dot(N, V), EPSILON);
    let mat = resolve_material(in.material_id, in.color.rgb);
    let base = mat.albedo;
    let roughness = mat.roughness;
    let metalness = mat.metalness;
    let specular = mat.specular_weight;

    let dielectric_F0 = vec3<f32>(ior_to_f0(mat.ior));
    let F0 = mix(dielectric_F0, base, metalness);

    var diff_acc = vec3<f32>(0.0);
    var spec_acc = vec3<f32>(0.0);

    let key = compute_light(lights.key, N, V, NdotV, base, F0, roughness, metalness, specular);
    diff_acc += key.diffuse;
    spec_acc += key.specular;

    let fill = compute_light(lights.fill, N, V, NdotV, base, F0, roughness, metalness, specular);
    diff_acc += fill.diffuse;
    spec_acc += fill.specular;

    let rim = compute_light(lights.rim, N, V, NdotV, base, F0, roughness, metalness, specular);
    diff_acc += rim.diffuse;
    spec_acc += rim.specular;

    if env.enabled > 0.5 {
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

        let R = reflect(-V, N);
        let env_spec = sample_env(R);
        let fresnel = F0 + (vec3<f32>(1.0) - F0) * pow5(1.0 - NdotV);
        spec_acc += env_spec * fresnel * (1.0 - roughness * roughness);
    } else {
        diff_acc += lights.ambient * base;
    }

    // Emission is additive — gives lights/neon a self-illumination look in PBR raster.
    let lit = diff_acc * camera.xray_alpha + spec_acc + mat.emission;
    let spec_bright = max(spec_acc.r, max(spec_acc.g, spec_acc.b));
    let emis_bright = max(mat.emission.r, max(mat.emission.g, mat.emission.b));
    let alpha = max(in.color.a * camera.xray_alpha, min(spec_bright + emis_bright, 1.0));

    return vec4<f32>(lit, alpha);
}

// ============================================================================
// G-Buffer Fragment (deferred path)
// ============================================================================

struct GBufferOut {
    @location(0) albedo_roughness: vec4<f32>,
    @location(1) normal_metalness: vec4<f32>,
}

@fragment
fn fs_gbuffer(in: VertexOutput, @builtin(front_facing) front: bool) -> GBufferOut {
    if slice_clip(in.world_pos) { discard; }

    var N: vec3<f32>;
    if camera.flat_shading > 0.5 {
        N = normalize(cross(dpdx(in.world_pos), dpdy(in.world_pos)));
    } else {
        N = normalize(in.world_normal);
    }
    if !front { N = -N; }

    let mat = resolve_material(in.material_id, in.color.rgb);
    var out: GBufferOut;
    out.albedo_roughness = vec4<f32>(mat.albedo, mat.roughness);
    out.normal_metalness = vec4<f32>(N * 0.5 + 0.5, mat.metalness);
    return out;
}

// ============================================================================
// Wireframe Fragment
// ============================================================================

@fragment
fn fs_wireframe(in: VertexOutput) -> @location(0) vec4<f32> {
    if slice_clip(in.world_pos) { discard; }

    // Wireframe stays per-instance-color tinted; library albedo not needed here.
    let mix_factor = clamp(in.color.a, 0.0, 1.0) * 0.4;
    let tint = mix(vec3<f32>(1.0, 1.0, 1.0), in.color.rgb, mix_factor);
    return vec4<f32>(tint, 1.0);
}
