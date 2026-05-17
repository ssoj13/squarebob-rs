// Common constants and math utilities
// Ported from MaterialX GLSL to WGSL

// Constants
const M_PI: f32 = 3.141592653589793;
const M_PI_INV: f32 = 0.3183098861837907;
const M_2PI: f32 = 6.283185307179586;
const M_FLOAT_EPS: f32 = 1e-6;

// Square
fn mx_square(x: f32) -> f32 {
    return x * x;
}

fn mx_square_v3(x: vec3<f32>) -> vec3<f32> {
    return x * x;
}

// Power of 6 (for Schlick approximation)
fn mx_pow6(x: f32) -> f32 {
    let x2 = x * x;
    return x2 * x2 * x2;
}

// Safe normalize (avoids NaN for zero vectors)
fn mx_safe_normalize(v: vec3<f32>) -> vec3<f32> {
    let len = length(v);
    if len < M_FLOAT_EPS {
        return vec3<f32>(0.0, 0.0, 1.0);
    }
    return v / len;
}

// Forward-facing normal (flip if backfacing)
fn mx_forward_facing_normal(N: vec3<f32>, V: vec3<f32>) -> vec3<f32> {
    if dot(N, V) < 0.0 {
        return -N;
    }
    return N;
}

// Clamp to valid range
fn mx_clamp_f32(x: f32, lo: f32, hi: f32) -> f32 {
    return max(lo, min(hi, x));
}

fn mx_clamp_v3(x: vec3<f32>, lo: f32, hi: f32) -> vec3<f32> {
    return max(vec3<f32>(lo), min(vec3<f32>(hi), x));
}

// Linear interpolation
fn mx_mix_f32(a: f32, b: f32, t: f32) -> f32 {
    return a * (1.0 - t) + b * t;
}

fn mx_mix_v3(a: vec3<f32>, b: vec3<f32>, t: f32) -> vec3<f32> {
    return a * (1.0 - t) + b * t;
}

// Luminance (Rec. 709)
fn mx_luminance(color: vec3<f32>) -> f32 {
    return dot(color, vec3<f32>(0.2126, 0.7152, 0.0722));
}

// Spherical fibonacci for importance sampling
fn mx_spherical_fibonacci(i: u32, n: u32) -> vec2<f32> {
    let PHI = 1.6180339887498949; // Golden ratio
    let phi = M_2PI * (f32(i) / PHI - floor(f32(i) / PHI));
    let cos_theta = 1.0 - (2.0 * f32(i) + 1.0) / f32(n);
    let sin_theta = sqrt(max(0.0, 1.0 - cos_theta * cos_theta));
    return vec2<f32>(phi, cos_theta);
}

// Reflect vector
fn mx_reflect(I: vec3<f32>, N: vec3<f32>) -> vec3<f32> {
    return I - 2.0 * dot(N, I) * N;
}

// Refract vector
fn mx_refract(I: vec3<f32>, N: vec3<f32>, eta: f32) -> vec3<f32> {
    let NdotI = dot(N, I);
    let k = 1.0 - eta * eta * (1.0 - NdotI * NdotI);
    if k < 0.0 {
        return vec3<f32>(0.0); // Total internal reflection
    }
    return eta * I - (eta * NdotI + sqrt(k)) * N;
}
