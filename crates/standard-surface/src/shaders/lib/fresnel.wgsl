// Fresnel equations for Standard Surface
// Ported from MaterialX GLSL to WGSL

// Fresnel model types
const FRESNEL_DIELECTRIC: u32 = 0u;
const FRESNEL_CONDUCTOR: u32 = 1u;
const FRESNEL_SCHLICK: u32 = 2u;

// Fresnel data structure
struct FresnelData {
    model: u32,
    ior: vec3<f32>,          // Index of refraction (n)
    extinction: vec3<f32>,   // Extinction coefficient (k) for conductors
    F0: vec3<f32>,           // Reflectance at normal incidence (Schlick)
    F90: vec3<f32>,          // Reflectance at grazing angle (Schlick)
    exponent: f32,           // Schlick exponent (usually 5.0)
}

// IOR to F0 conversion
fn mx_ior_to_f0(ior: f32) -> f32 {
    let r = (ior - 1.0) / (ior + 1.0);
    return r * r;
}

fn mx_ior_to_f0_v3(ior: vec3<f32>) -> vec3<f32> {
    let r = (ior - vec3<f32>(1.0)) / (ior + vec3<f32>(1.0));
    return r * r;
}

// F0 to IOR conversion
fn mx_f0_to_ior(F0: f32) -> f32 {
    let sqrt_F0 = sqrt(clamp(F0, 0.01, 0.99));
    return (1.0 + sqrt_F0) / (1.0 - sqrt_F0);
}

fn mx_f0_to_ior_v3(F0: vec3<f32>) -> vec3<f32> {
    let sqrt_F0 = sqrt(clamp(F0, vec3<f32>(0.01), vec3<f32>(0.99)));
    return (vec3<f32>(1.0) + sqrt_F0) / (vec3<f32>(1.0) - sqrt_F0);
}

// Classic Schlick Fresnel approximation
fn mx_fresnel_schlick(cos_theta: f32, F0: f32, F90: f32) -> f32 {
    let x = clamp(1.0 - cos_theta, 0.0, 1.0);
    let x2 = x * x;
    let x5 = x2 * x2 * x;
    return F0 + (F90 - F0) * x5;
}

fn mx_fresnel_schlick_v3(cos_theta: f32, F0: vec3<f32>, F90: vec3<f32>) -> vec3<f32> {
    let x = clamp(1.0 - cos_theta, 0.0, 1.0);
    let x2 = x * x;
    let x5 = x2 * x2 * x;
    return F0 + (F90 - F0) * x5;
}

// Exact Fresnel for dielectrics
// https://seblagarde.wordpress.com/2013/04/29/memo-on-fresnel-equations/
fn mx_fresnel_dielectric(cos_theta: f32, ior: f32) -> f32 {
    let c = cos_theta;
    let g2 = ior * ior + c * c - 1.0;

    if g2 < 0.0 {
        // Total internal reflection
        return 1.0;
    }

    let g = sqrt(g2);
    let gmc = g - c;
    let gpc = g + c;
    let r1 = gmc / gpc;
    let r2 = (gpc * c - 1.0) / (gmc * c + 1.0);

    return 0.5 * r1 * r1 * (1.0 + r2 * r2);
}

// Fresnel for conductors (metals)
// https://seblagarde.wordpress.com/2013/04/29/memo-on-fresnel-equations/
fn mx_fresnel_conductor(cos_theta: f32, n: vec3<f32>, k: vec3<f32>) -> vec3<f32> {
    let cos_theta2 = cos_theta * cos_theta;
    let sin_theta2 = 1.0 - cos_theta2;
    let n2 = n * n;
    let k2 = k * k;

    let t0 = n2 - k2 - vec3<f32>(sin_theta2);
    let a2_plus_b2 = sqrt(t0 * t0 + 4.0 * n2 * k2);
    let t1 = a2_plus_b2 + vec3<f32>(cos_theta2);
    let a = sqrt(max(vec3<f32>(0.0), 0.5 * (a2_plus_b2 + t0)));
    let t2 = 2.0 * a * cos_theta;
    let Rs = (t1 - t2) / (t1 + t2);

    let t3 = cos_theta2 * a2_plus_b2 + vec3<f32>(sin_theta2 * sin_theta2);
    let t4 = t2 * sin_theta2;
    let Rp = Rs * (t3 - t4) / (t3 + t4);

    return 0.5 * (Rp + Rs);
}

// Generalized Schlick with F82 tint (Hoffman 2019)
// https://renderwonk.com/publications/wp-generalization-adobe/gen-adobe.pdf
fn mx_fresnel_hoffman_schlick(cos_theta: f32, F0: vec3<f32>, F82: vec3<f32>, F90: vec3<f32>, exponent: f32) -> vec3<f32> {
    let COS_THETA_MAX = 1.0 / 7.0;
    let COS_THETA_FACTOR = 1.0 / (COS_THETA_MAX * pow(1.0 - COS_THETA_MAX, 6.0));

    let x = clamp(cos_theta, 0.0, 1.0);
    let x_pow = pow(1.0 - x, exponent);
    let x6 = pow(1.0 - x, 6.0);

    let base = mix(F0, F90, x_pow);
    let a = mix(F0, F90, pow(1.0 - COS_THETA_MAX, exponent)) * (vec3<f32>(1.0) - F82) * COS_THETA_FACTOR;

    return base - a * x * x6;
}

// Initialize Fresnel data for dielectric
fn mx_init_fresnel_dielectric(ior: f32) -> FresnelData {
    var fd: FresnelData;
    fd.model = FRESNEL_DIELECTRIC;
    fd.ior = vec3<f32>(ior);
    fd.extinction = vec3<f32>(0.0);
    fd.F0 = vec3<f32>(mx_ior_to_f0(ior));
    fd.F90 = vec3<f32>(1.0);
    fd.exponent = 5.0;
    return fd;
}

// Initialize Fresnel data for conductor (metal)
fn mx_init_fresnel_conductor(ior: vec3<f32>, extinction: vec3<f32>) -> FresnelData {
    var fd: FresnelData;
    fd.model = FRESNEL_CONDUCTOR;
    fd.ior = ior;
    fd.extinction = extinction;
    fd.F0 = vec3<f32>(0.0);
    fd.F90 = vec3<f32>(0.0);
    fd.exponent = 5.0;
    return fd;
}

// Initialize Fresnel data for Schlick approximation
fn mx_init_fresnel_schlick_data(F0: vec3<f32>, F90: vec3<f32>) -> FresnelData {
    var fd: FresnelData;
    fd.model = FRESNEL_SCHLICK;
    fd.ior = vec3<f32>(0.0);
    fd.extinction = vec3<f32>(0.0);
    fd.F0 = F0;
    fd.F90 = F90;
    fd.exponent = 5.0;
    return fd;
}

// Compute Fresnel based on model type
fn mx_compute_fresnel(cos_theta: f32, fd: FresnelData) -> vec3<f32> {
    if fd.model == FRESNEL_DIELECTRIC {
        return vec3<f32>(mx_fresnel_dielectric(cos_theta, fd.ior.x));
    } else if fd.model == FRESNEL_CONDUCTOR {
        return mx_fresnel_conductor(cos_theta, fd.ior, fd.extinction);
    } else {
        // FRESNEL_SCHLICK
        return mx_fresnel_schlick_v3(cos_theta, fd.F0, fd.F90);
    }
}
