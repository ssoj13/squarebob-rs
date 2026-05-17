// Autodesk Standard Surface Shader
// WGSL port of MaterialX implementation
//
// References:
// - https://autodesk.github.io/standard-surface/
// - https://github.com/AcademySoftwareFoundation/MaterialX

// ============================================================================
// Constants
// ============================================================================

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
    xray_alpha: f32,  // X-Ray mode: 1.0 = normal, 0.5 = transparent
    flat_shading: f32, // 1.0 = flat (face normals), 0.0 = smooth
    auto_normals: f32, // 1.0 = auto-flip inverted normals, 0.0 = disabled
    _pad2: f32,
    _pad3: f32,
}

struct Light {
    direction: vec3<f32>,
    _pad1: f32,
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

// Using vec4 for colors to ensure proper alignment (16-byte)
struct StandardSurfaceParams {
    // Base (vec4: rgb=color, a=weight)
    base_color_weight: vec4<f32>,
    // Specular (vec4: rgb=color, a=weight)
    specular_color_weight: vec4<f32>,
    // Transmission (vec4: rgb=color, a=weight)
    transmission_color_weight: vec4<f32>,
    // Subsurface (vec4: rgb=color, a=weight)
    subsurface_color_weight: vec4<f32>,
    // Coat (vec4: rgb=color, a=weight)
    coat_color_weight: vec4<f32>,
    // Emission (vec4: rgb=color, a=weight)
    emission_color_weight: vec4<f32>,
    // Opacity (vec4: rgb=opacity, a=unused)
    opacity: vec4<f32>,
    // Packed scalars
    // x=diffuse_roughness, y=metalness, z=specular_roughness, w=specular_IOR
    params1: vec4<f32>,
    // x=specular_anisotropy, y=coat_roughness, z=coat_IOR, w=unused
    params2: vec4<f32>,
}

@group(0) @binding(0) var<uniform> camera: Camera;
@group(0) @binding(1) var<uniform> lights: LightRig;
@group(1) @binding(0) var<uniform> material: StandardSurfaceParams;

// ============================================================================
// Vertex Shader
// ============================================================================

struct VertexInput {
    @location(0) position: vec3<f32>,
    @location(1) normal: vec3<f32>,
    @location(2) uv: vec2<f32>,
}

struct VertexOutput {
    @builtin(position) clip_position: vec4<f32>,
    @location(0) world_position: vec3<f32>,
    @location(1) world_normal: vec3<f32>,
    @location(2) uv: vec2<f32>,
}

struct ModelUniform {
    model: mat4x4<f32>,
    normal_matrix: mat4x4<f32>,
    object_id: u32,
    _pad0: u32,
    _pad1: u32,
    _pad2: u32,
}

@group(2) @binding(0) var<uniform> model: ModelUniform;

// Shadow mapping for key light
struct ShadowUniform {
    light_view_proj: mat4x4<f32>,
}

@group(3) @binding(0) var shadow_map: texture_depth_2d;
@group(3) @binding(1) var shadow_sampler: sampler_comparison;
@group(3) @binding(2) var<uniform> shadow: ShadowUniform;

// Environment map (equirectangular HDR/EXR)
struct EnvParams {
    intensity: f32,
    rotation: f32,
    enabled: f32,
    _pad: f32,
}

@group(4) @binding(0) var env_map: texture_2d<f32>;
@group(4) @binding(1) var env_sampler: sampler;
@group(4) @binding(2) var<uniform> env: EnvParams;

@vertex
fn vs_main(in: VertexInput) -> VertexOutput {
    var out: VertexOutput;

    let world_pos = model.model * vec4<f32>(in.position, 1.0);
    out.world_position = world_pos.xyz;
    out.clip_position = camera.view_proj * world_pos;
    out.world_normal = normalize((model.normal_matrix * vec4<f32>(in.normal, 0.0)).xyz);
    out.uv = in.uv;

    return out;
}

// ============================================================================
// Helper Functions
// ============================================================================

fn square(x: f32) -> f32 {
    return x * x;
}

fn pow5(x: f32) -> f32 {
    let x2 = x * x;
    return x2 * x2 * x;
}

// Schlick Fresnel
fn fresnel_schlick(cos_theta: f32, F0: vec3<f32>) -> vec3<f32> {
    return F0 + (vec3<f32>(1.0) - F0) * pow5(1.0 - cos_theta);
}

// IOR to F0
fn ior_to_f0(ior: f32) -> f32 {
    let r = (ior - 1.0) / (ior + 1.0);
    return r * r;
}

// GGX Normal Distribution
fn distribution_ggx(NdotH: f32, roughness: f32) -> f32 {
    let a = roughness * roughness;
    let a2 = a * a;
    let denom = NdotH * NdotH * (a2 - 1.0) + 1.0;
    return a2 / (PI * denom * denom);
}

// Smith GGX Geometry (height-correlated)
fn geometry_smith(NdotV: f32, NdotL: f32, roughness: f32) -> f32 {
    let a = roughness * roughness;
    let a2 = a * a;

    let lambda_v = sqrt(a2 + (1.0 - a2) * NdotV * NdotV);
    let lambda_l = sqrt(a2 + (1.0 - a2) * NdotL * NdotL);

    return 2.0 * NdotV * NdotL / (lambda_v * NdotL + lambda_l * NdotV + EPSILON);
}

// Oren-Nayar Diffuse
fn oren_nayar(NdotV: f32, NdotL: f32, LdotV: f32, roughness: f32) -> f32 {
    let sigma2 = roughness * roughness;
    let A = 1.0 - 0.5 * sigma2 / (sigma2 + 0.33);
    let B = 0.45 * sigma2 / (sigma2 + 0.09);

    let s = LdotV - NdotL * NdotV;
    var t: f32;
    if s > 0.0 {
        t = max(NdotL, NdotV);
    } else {
        t = 1.0;
    }

    return A + B * s / t;
}

// Direction to equirectangular UV
fn dir_to_equirect_uv(dir: vec3<f32>, rotation: f32) -> vec2<f32> {
    let d = normalize(dir);
    // Spherical coordinates
    let phi = atan2(d.z, d.x) + rotation;
    let theta = acos(clamp(d.y, -1.0, 1.0));
    // Map to [0,1]
    let u = (phi + PI) / (2.0 * PI);
    let v = theta / PI;
    return vec2<f32>(u, v);
}

// Sample environment map
fn sample_env(dir: vec3<f32>) -> vec3<f32> {
    if env.enabled < 0.5 {
        return vec3<f32>(0.0);
    }
    let uv = dir_to_equirect_uv(dir, env.rotation);
    let color = textureSample(env_map, env_sampler, uv).rgb;
    return color * env.intensity;
}

// Shadow sampling with PCF (percentage closer filtering)
fn sample_shadow(world_pos: vec3<f32>) -> f32 {
    // Transform to light space
    let light_space = shadow.light_view_proj * vec4<f32>(world_pos, 1.0);
    let proj_coords = light_space.xyz / light_space.w;
    
    // Transform to [0, 1] range for texture sampling (flip Y for WGSL)
    let shadow_uv = proj_coords.xy * vec2<f32>(0.5, -0.5) + vec2<f32>(0.5);
    let current_depth = proj_coords.z;
    
    // Check if outside shadow map bounds
    if shadow_uv.x < 0.0 || shadow_uv.x > 1.0 || shadow_uv.y < 0.0 || shadow_uv.y > 1.0 || current_depth > 1.0 {
        return 1.0; // No shadow outside map
    }
    
    // PCF - sample multiple points for soft shadows
    let texel_size = 1.0 / 2048.0; // SHADOW_MAP_SIZE
    var shadow_sum = 0.0;
    let bias = 0.002; // Bias to reduce shadow acne
    
    // 3x3 PCF kernel
    for (var x = -1; x <= 1; x++) {
        for (var y = -1; y <= 1; y++) {
            let offset = vec2<f32>(f32(x), f32(y)) * texel_size;
            shadow_sum += textureSampleCompare(
                shadow_map,
                shadow_sampler,
                shadow_uv + offset,
                current_depth - bias
            );
        }
    }
    
    return shadow_sum / 9.0;
}

// Stable hash for object ID coloring (wireframe)
fn hash_u32(x: u32) -> u32 {
    var v = x;
    v ^= v >> 16;
    v *= 0x7feb352d;
    v ^= v >> 15;
    v *= 0x846ca68b;
    v ^= v >> 16;
    return v;
}

fn id_to_color(id: u32) -> vec3<f32> {
    let h = hash_u32(id);
    let r = f32(h & 255u) / 255.0;
    let g = f32((h >> 8) & 255u) / 255.0;
    let b = f32((h >> 16) & 255u) / 255.0;
    // Lift the color slightly for readability on dark backgrounds.
    return vec3<f32>(r, g, b) * 0.8 + vec3<f32>(0.2);
}

// ============================================================================
// Fragment Shader
// ============================================================================

// Separated diffuse and specular for xray_alpha handling
struct LightContribution {
    diffuse: vec3<f32>,
    specular: vec3<f32>,
}

// Compute lighting contribution from a single directional light
fn compute_light(
    light: Light,
    N: vec3<f32>,
    V: vec3<f32>,
    NdotV: f32,
    effective_base: vec3<f32>,
    F0: vec3<f32>,
    diffuse_roughness: f32,
    specular_roughness: f32,
    specular: f32,
    metalness: f32,
    coat: f32,
    coat_color: vec3<f32>,
    coat_roughness: f32,
    coat_IOR: f32,
) -> LightContribution {
    var result: LightContribution;
    result.diffuse = vec3<f32>(0.0);
    result.specular = vec3<f32>(0.0);
    
    // Skip if light is off
    if light.intensity < EPSILON {
        return result;
    }
    
    let L = normalize(-light.direction);
    let H = normalize(V + L);
    
    let NdotL = max(dot(N, L), 0.0);
    if NdotL < EPSILON {
        return result;
    }
    
    let NdotH = max(dot(N, H), 0.0);
    let VdotH = max(dot(V, H), 0.0);
    let LdotV = max(dot(L, V), 0.0);
    
    // Diffuse (Oren-Nayar)
    let diffuse_factor = oren_nayar(NdotV, NdotL, LdotV, diffuse_roughness);
    let diffuse = effective_base * diffuse_factor * PI_INV;
    
    // Specular (GGX)
    let D = distribution_ggx(NdotH, specular_roughness);
    let G = geometry_smith(NdotV, NdotL, specular_roughness);
    let F = fresnel_schlick(VdotH, F0);
    let specular_brdf = (D * G * F) / (4.0 * NdotV * NdotL + EPSILON);
    
    // Energy conservation
    let kS = F;
    let kD = (vec3<f32>(1.0) - kS) * (1.0 - metalness);
    
    // Coat layer (counts as specular)
    var coat_contribution = vec3<f32>(0.0);
    if coat > EPSILON {
        let coat_F0 = vec3<f32>(ior_to_f0(coat_IOR));
        let coat_D = distribution_ggx(NdotH, coat_roughness);
        let coat_G = geometry_smith(NdotV, NdotL, coat_roughness);
        let coat_F = fresnel_schlick(VdotH, coat_F0);
        coat_contribution = coat * coat_color *
                           (coat_D * coat_G * coat_F) / (4.0 * NdotV * NdotL + EPSILON);
    }
    
    let radiance = light.color * light.intensity;
    result.diffuse = kD * diffuse * radiance * NdotL;
    result.specular = (specular_brdf * specular + coat_contribution) * radiance * NdotL;
    return result;
}

struct GBufferOutput {
    @location(0) albedo_roughness: vec4<f32>,
    @location(1) normal_metalness: vec4<f32>,
    @location(2) occlusion: vec4<f32>,
}

@fragment
fn fs_main(in: VertexOutput, @builtin(front_facing) is_front: bool) -> @location(0) vec4<f32> {
    // View direction (needed for backface detection)
    let V = normalize(camera.position - in.world_position);
    
    // Normal: use face normal (flat) or interpolated vertex normal (smooth)
    var N: vec3<f32>;
    if camera.flat_shading > 0.5 {
        // Flat shading: compute face normal from screen-space derivatives
        let dpdx = dpdx(in.world_position);
        let dpdy = dpdy(in.world_position);
        N = normalize(cross(dpdx, dpdy));
    } else {
        N = normalize(in.world_normal);
    }
    
    // Ensure backfaces light correctly when culling is disabled.
    if !is_front {
        N = -N;
    }

    // Auto-flip normal if facing away from camera (handles inverted normals)
    if camera.auto_normals > 0.5 && dot(N, V) < 0.0 {
        N = -N;
    }
    let NdotV = max(dot(N, V), EPSILON);

    // Unpack material parameters
    let base_color = material.base_color_weight.rgb;
    let base = material.base_color_weight.a;
    let specular_color = material.specular_color_weight.rgb;
    let specular = material.specular_color_weight.a;
    let coat_color = material.coat_color_weight.rgb;
    let coat = material.coat_color_weight.a;
    let emission_color = material.emission_color_weight.rgb;
    let emission = material.emission_color_weight.a;
    
    let diffuse_roughness = material.params1.x;
    let metalness = material.params1.y;
    let specular_roughness = max(material.params1.z, 0.04);
    let specular_IOR = material.params1.w;
    let coat_roughness = max(material.params2.y, 0.04);
    let coat_IOR = material.params2.z;

    // Effective base color
    let effective_base = base_color * base;

    // F0 for dielectrics from IOR, for metals from base_color
    let dielectric_F0 = vec3<f32>(ior_to_f0(specular_IOR));
    let F0 = mix(dielectric_F0 * specular_color, effective_base, metalness);

    // Sample shadow for key light
    let shadow_factor = sample_shadow(in.world_position);
    
    // Accumulate diffuse and specular separately (for xray_alpha handling)
    var diffuse_accum = vec3<f32>(0.0);
    var specular_accum = vec3<f32>(0.0);
    
    // Key light (main light) - with shadow
    let key_light = compute_light(
        lights.key, N, V, NdotV,
        effective_base, F0,
        diffuse_roughness, specular_roughness, specular, metalness,
        coat, coat_color, coat_roughness, coat_IOR
    );
    diffuse_accum += shadow_factor * key_light.diffuse;
    specular_accum += shadow_factor * key_light.specular;
    
    // Fill light (softer, side light)
    let fill_light = compute_light(
        lights.fill, N, V, NdotV,
        effective_base, F0,
        diffuse_roughness, specular_roughness, specular, metalness,
        coat, coat_color, coat_roughness, coat_IOR
    );
    diffuse_accum += fill_light.diffuse;
    specular_accum += fill_light.specular;
    
    // Rim light (back/edge light)
    let rim_light = compute_light(
        lights.rim, N, V, NdotV,
        effective_base, F0,
        diffuse_roughness, specular_roughness, specular, metalness,
        coat, coat_color, coat_roughness, coat_IOR
    );
    diffuse_accum += rim_light.diffuse;
    specular_accum += rim_light.specular;

    // Ambient / IBL
    if env.enabled > 0.5 {
        // Diffuse IBL - sample hemisphere around normal for better irradiance estimate
        // Normal-aligned samples + axis-aligned average for soft ambient fill
        let env_normal = sample_env(N);  // Primary direction along surface normal
        // Cross-axis samples for hemisphere approximation
        let T = normalize(cross(N, select(vec3<f32>(0.0, 1.0, 0.0), vec3<f32>(1.0, 0.0, 0.0), abs(N.y) > 0.9)));
        let B = cross(N, T);
        let env_t1 = sample_env(normalize(N + T * 0.7));
        let env_t2 = sample_env(normalize(N - T * 0.7));
        let env_b1 = sample_env(normalize(N + B * 0.7));
        let env_b2 = sample_env(normalize(N - B * 0.7));
        let env_up = sample_env(vec3<f32>(0.0, 1.0, 0.0));  // Ground plane fill
        // Weighted: normal direction strongest, cross-directions softer, sky fill subtle
        let env_diffuse = env_normal * 0.35 + (env_t1 + env_t2 + env_b1 + env_b2) * 0.125 + env_up * 0.15;
        diffuse_accum += env_diffuse * effective_base * (1.0 - metalness) * 0.5;
        
        // Specular IBL - sample along reflection
        let R = reflect(-V, N);
        let env_specular = sample_env(R);
        // Fresnel: more reflection at grazing angles
        let fresnel = F0 + (vec3<f32>(1.0) - F0) * pow5(1.0 - NdotV);
        let spec_atten = 1.0 - specular_roughness * specular_roughness; // smoother falloff
        specular_accum += env_specular * fresnel * spec_atten;
    } else {
        // Fallback to flat ambient
        diffuse_accum += lights.ambient * effective_base;
    }
    
    // Emission (not affected by xray_alpha, like specular)
    specular_accum += emission * emission_color;

    // Apply xray_alpha only to diffuse, keep specular at full intensity
    let base_alpha = (material.opacity.r + material.opacity.g + material.opacity.b) / 3.0;
    
    // Final color: diffuse fades with xray_alpha, specular stays bright
    let color = diffuse_accum * camera.xray_alpha + specular_accum;
    
    // Alpha: base opacity * xray, but boosted by specular brightness
    // This prevents specular from being blended away at low opacity
    let spec_brightness = max(specular_accum.r, max(specular_accum.g, specular_accum.b));
    let alpha = max(base_alpha * camera.xray_alpha, min(spec_brightness, 1.0));

    return vec4<f32>(color, alpha);
}

@fragment
fn fs_gbuffer(in: VertexOutput, @builtin(front_facing) is_front: bool) -> GBufferOutput {
    // View direction (needed for backface detection)
    let V = normalize(camera.position - in.world_position);

    // Normal: use face normal (flat) or interpolated vertex normal (smooth)
    var N: vec3<f32>;
    if camera.flat_shading > 0.5 {
        // Flat shading: compute face normal from screen-space derivatives
        let dpdx = dpdx(in.world_position);
        let dpdy = dpdy(in.world_position);
        N = normalize(cross(dpdx, dpdy));
    } else {
        N = normalize(in.world_normal);
    }

    // Ensure backfaces light correctly when culling is disabled.
    if !is_front {
        N = -N;
    }

    // Auto-flip normal if facing away from camera (handles inverted normals)
    if camera.auto_normals > 0.5 && dot(N, V) < 0.0 {
        N = -N;
    }

    // Material unpack
    let base_color = material.base_color_weight.rgb;
    let base = material.base_color_weight.a;
    let specular_roughness = max(material.params1.z, 0.04);
    let metalness = material.params1.y;

    let effective_base = base_color * base;

    // Encode normal from [-1,1] to [0,1]
    let enc_n = N * 0.5 + vec3<f32>(0.5);

    var out: GBufferOutput;
    out.albedo_roughness = vec4<f32>(effective_base, specular_roughness);
    out.normal_metalness = vec4<f32>(enc_n, metalness);
    out.occlusion = vec4<f32>(1.0, 1.0, 1.0, 1.0);
    return out;
}

// ============================================================================
// Wireframe variant (for debugging)
// ============================================================================

@fragment
fn fs_wireframe(in: VertexOutput) -> @location(0) vec4<f32> {
    let base_color = material.base_color_weight.rgb;
    let base_weight = clamp(material.base_color_weight.a, 0.0, 1.0);
    let mix_factor = clamp(base_weight * 0.65, 0.0, 1.0);
    let tint = mix(vec3<f32>(1.0, 1.0, 1.0), base_color, mix_factor);
    return vec4<f32>(tint, 1.0);
}
