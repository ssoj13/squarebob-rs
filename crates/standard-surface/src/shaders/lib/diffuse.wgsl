// Diffuse BRDF functions (Oren-Nayar)
// Ported from MaterialX GLSL to WGSL

// Lambertian diffuse (simple, no roughness)
fn mx_lambert_diffuse(color: vec3<f32>) -> vec3<f32> {
    return color * M_PI_INV;
}

// Oren-Nayar diffuse BRDF
// Handles rough surfaces better than Lambert
fn mx_oren_nayar_diffuse(NdotV: f32, NdotL: f32, LdotV: f32, roughness: f32) -> f32 {
    let s = LdotV - NdotL * NdotV;
    let sigma2 = roughness * roughness;

    // A and B coefficients
    let A = 1.0 - 0.5 * sigma2 / (sigma2 + 0.33);
    let B = 0.45 * sigma2 / (sigma2 + 0.09);

    // Compute t based on s sign
    var t: f32;
    if s > 0.0 {
        t = max(NdotL, NdotV);
    } else {
        t = 1.0;
    }

    return A + B * s / t;
}

// Oren-Nayar with energy compensation
fn mx_oren_nayar_compensated_diffuse(
    NdotV: f32,
    NdotL: f32,
    LdotV: f32,
    roughness: f32,
    color: vec3<f32>
) -> vec3<f32> {
    let sigma2 = roughness * roughness;

    // Directional albedo approximation
    let dir_albedo = 1.0 - 0.5 * sigma2 / (sigma2 + 0.51);
    let energy_comp = 1.0 / dir_albedo;

    let diffuse = mx_oren_nayar_diffuse(NdotV, NdotL, LdotV, roughness);
    return color * diffuse * energy_comp;
}

// Directional albedo for Oren-Nayar (used in IBL)
fn mx_oren_nayar_diffuse_dir_albedo(NdotV: f32, roughness: f32) -> f32 {
    let sigma2 = roughness * roughness;
    // Approximation from MaterialX
    let albedo = 1.0 - 0.5 * sigma2 / (sigma2 + 0.51);
    return albedo;
}

// Burley diffuse (Disney)
// Alternative to Oren-Nayar, used in some engines
fn mx_burley_diffuse(NdotV: f32, NdotL: f32, VdotH: f32, roughness: f32) -> f32 {
    let FD90 = 0.5 + 2.0 * roughness * VdotH * VdotH;
    let FdV = 1.0 + (FD90 - 1.0) * pow(1.0 - NdotV, 5.0);
    let FdL = 1.0 + (FD90 - 1.0) * pow(1.0 - NdotL, 5.0);
    return FdV * FdL * M_PI_INV;
}
