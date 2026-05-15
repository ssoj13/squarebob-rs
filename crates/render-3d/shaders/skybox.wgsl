// Skybox shader - renders equirectangular HDR environment map as background
// Fullscreen triangle with ray-marching against unit sphere
// Only renders where depth buffer is at clear value (no geometry)

const PI: f32 = 3.141592653589793;

struct Camera {
    view_proj: mat4x4<f32>,
    view: mat4x4<f32>,
    inv_view_proj: mat4x4<f32>,
    position: vec3<f32>,
    xray_alpha: f32,
    flat_shading: f32,
    _pad0: f32,
    _pad1: f32,
    _pad2: f32,
}

struct EnvParams {
    intensity: f32,
    rotation: f32,
    enabled: f32,
    _pad: f32,
}

@group(0) @binding(0) var<uniform> camera: Camera;
@group(0) @binding(1) var env_map: texture_2d<f32>;
@group(0) @binding(2) var env_sampler: sampler;
@group(0) @binding(3) var<uniform> env: EnvParams;

struct VertexOutput {
    @builtin(position) position: vec4<f32>,
    @location(0) uv: vec2<f32>,
}

// Fullscreen triangle
@vertex
fn vs_main(@builtin(vertex_index) vertex_idx: u32) -> VertexOutput {
    var out: VertexOutput;
    let x = f32(i32(vertex_idx & 1u) * 4 - 1);
    let y = f32(i32(vertex_idx >> 1u) * 4 - 1);
    // Reversed-Z convention: clip-space z=0.0 is the far plane (depth
    // attachment is cleared to 0.0 and the cube pipeline uses Greater).
    // The skybox must sit at far so any geometry (depth > 0) wins the
    // `GreaterEqual` compare. Forward-Z used z=1.0 here, which now
    // marks the near plane and would paint over every cube.
    out.position = vec4<f32>(x, y, 0.0, 1.0);
    out.uv = vec2<f32>((x + 1.0) * 0.5, (1.0 - y) * 0.5);
    return out;
}

// Direction to equirectangular UV
fn dir_to_equirect_uv(dir: vec3<f32>, rotation: f32) -> vec2<f32> {
    let d = normalize(dir);
    let phi = atan2(d.z, d.x) + rotation;
    let theta = acos(clamp(d.y, -1.0, 1.0));
    return vec2<f32>((phi + PI) / (2.0 * PI), theta / PI);
}

@fragment
fn fs_main(in: VertexOutput) -> @location(0) vec4<f32> {
    if env.enabled < 0.5 {
        return vec4<f32>(0.1, 0.1, 0.1, 1.0); // Default dark background
    }

    // Reconstruct world-space ray from screen UV. Reversed-Z: NDC z=0.0
    // is the far plane (the direction we want), but with a
    // perspective_infinite_reverse_rh matrix exactly 0.0 maps to view_z=-∞
    // — `inv_view_proj * (uv, 0, 1)` has w ≈ 0, the perspective divide
    // produces NaN/Inf and the texture sample returns black. Stepping a
    // tiny ε off the far plane (here 0.001, corresponding to view_z ≈
    // -1000 × near, way beyond any geometry) keeps w well-defined while
    // the resulting direction is indistinguishable from a true
    // infinite-far ray for skybox sampling.
    let ndc = vec4<f32>(in.uv.x * 2.0 - 1.0, 1.0 - in.uv.y * 2.0, 0.001, 1.0);
    let world = camera.inv_view_proj * ndc;
    let dir = normalize(world.xyz / world.w - camera.position);

    let uv = dir_to_equirect_uv(dir, env.rotation);
    let color = textureSample(env_map, env_sampler, uv).rgb * env.intensity;

    return vec4<f32>(color, 1.0);
}
