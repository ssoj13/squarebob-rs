// Microfacet BRDF functions (GGX/Smith)
// Ported from MaterialX GLSL to WGSL

// GGX Normal Distribution Function (NDF)
// Disney/GGX with anisotropic support
fn mx_ggx_NDF(H: vec3<f32>, alpha: vec2<f32>) -> f32 {
    let He = vec2<f32>(H.x, H.y) / alpha;
    let denom = dot(He, He) + H.z * H.z;
    return 1.0 / (M_PI * alpha.x * alpha.y * denom * denom);
}

// Isotropic GGX NDF
fn mx_ggx_NDF_iso(NdotH: f32, alpha: f32) -> f32 {
    let alpha2 = alpha * alpha;
    let d = NdotH * NdotH * (alpha2 - 1.0) + 1.0;
    return alpha2 / (M_PI * d * d);
}

// Smith G1 (single direction masking)
fn mx_ggx_smith_G1(cos_theta: f32, alpha: f32) -> f32 {
    let cos_theta2 = cos_theta * cos_theta;
    let tan_theta2 = (1.0 - cos_theta2) / cos_theta2;
    return 2.0 / (1.0 + sqrt(1.0 + alpha * alpha * tan_theta2));
}

// Height-correlated Smith G2 (masking-shadowing)
// More accurate than separable G1*G1
fn mx_ggx_smith_G2(NdotL: f32, NdotV: f32, alpha: f32) -> f32 {
    let alpha2 = alpha * alpha;
    let lambda_L = sqrt(alpha2 + (1.0 - alpha2) * NdotL * NdotL);
    let lambda_V = sqrt(alpha2 + (1.0 - alpha2) * NdotV * NdotV);
    return 2.0 * NdotL * NdotV / (lambda_L * NdotV + lambda_V * NdotL);
}

// Average alpha for anisotropic roughness
fn mx_average_alpha(alpha: vec2<f32>) -> f32 {
    return sqrt(alpha.x * alpha.y);
}

// Convert roughness + anisotropy to alpha pair
fn mx_roughness_to_alpha(roughness: f32, anisotropy: f32) -> vec2<f32> {
    let aspect = sqrt(1.0 - 0.9 * anisotropy);
    return vec2<f32>(
        max(roughness / aspect, M_FLOAT_EPS),
        max(roughness * aspect, M_FLOAT_EPS)
    );
}

// GGX directional albedo (analytical fit)
// Used for energy compensation
fn mx_ggx_dir_albedo_analytic(NdotV: f32, alpha: f32, F0: vec3<f32>, F90: vec3<f32>) -> vec3<f32> {
    let x = NdotV;
    let y = alpha;
    let x2 = x * x;
    let y2 = y * y;

    // Rational quadratic fit to Monte Carlo data
    let r = vec4<f32>(0.1003, 0.9345, 1.0, 1.0) +
            vec4<f32>(-0.6303, -2.323, -1.765, 0.2281) * x +
            vec4<f32>(9.748, 2.229, 8.263, 15.94) * y +
            vec4<f32>(-2.038, -3.748, 11.53, -55.83) * x * y +
            vec4<f32>(29.34, 1.424, 28.96, 13.08) * x2 +
            vec4<f32>(-8.245, -0.7684, -7.507, 41.26) * y2 +
            vec4<f32>(-26.44, 1.436, -36.11, 54.9) * x2 * y +
            vec4<f32>(19.99, 0.2913, 15.86, 300.2) * x * y2 +
            vec4<f32>(-5.448, 0.6286, 33.37, -285.1) * x2 * y2;

    let AB = clamp(r.xy / r.zw, vec2<f32>(0.0), vec2<f32>(1.0));
    return F0 * AB.x + F90 * AB.y;
}

// Energy compensation for multiple scattering
// https://blog.selfshadow.com/publications/turquin/ms_comp_final.pdf
fn mx_ggx_energy_compensation(NdotV: f32, alpha: f32, Fss: vec3<f32>) -> vec3<f32> {
    let Ess = mx_ggx_dir_albedo_analytic(NdotV, alpha, vec3<f32>(1.0), vec3<f32>(1.0)).x;
    return vec3<f32>(1.0) + Fss * (1.0 - Ess) / Ess;
}

// Importance sample GGX VNDF (visible normal distribution)
// https://ggx-research.github.io/publication/2023/06/09/publication-ggx.html
fn mx_ggx_importance_sample_VNDF(Xi: vec2<f32>, V: vec3<f32>, alpha: vec2<f32>) -> vec3<f32> {
    // Transform view to hemisphere config
    let V_h = normalize(vec3<f32>(V.xy * alpha, V.z));

    // Sample spherical cap
    let phi = M_2PI * Xi.x;
    let z = (1.0 - Xi.y) * (1.0 + V_h.z) - V_h.z;
    let sin_theta = sqrt(max(0.0, 1.0 - z * z));
    let x = sin_theta * cos(phi);
    let y = sin_theta * sin(phi);

    // Compute microfacet normal
    var H = vec3<f32>(x, y, z) + V_h;

    // Transform back
    H = normalize(vec3<f32>(H.xy * alpha, max(H.z, 0.0)));

    return H;
}

// Full specular BRDF evaluation
fn mx_ggx_specular_brdf(
    NdotL: f32,
    NdotV: f32,
    NdotH: f32,
    VdotH: f32,
    alpha: f32,
    F: vec3<f32>
) -> vec3<f32> {
    let D = mx_ggx_NDF_iso(NdotH, alpha);
    let G = mx_ggx_smith_G2(NdotL, NdotV, alpha);

    // Cook-Torrance: D * F * G / (4 * NdotL * NdotV)
    // Note: NdotL cancels with the cosine in rendering equation
    return D * F * G / (4.0 * NdotV);
}
