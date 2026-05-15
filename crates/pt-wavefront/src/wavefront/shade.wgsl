// Wavefront Shading pass.
// Evaluate BSDF, generate next rays, accumulate emission.

// GPU instance: 96 bytes, matches GpuInstance in Rust.
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

// Must match GpuMaterial (144 bytes = 9 x vec4)
struct Material {
    base_color_weight: vec4<f32>,      // rgb=color, a=weight
    specular_color_weight: vec4<f32>,
    transmission_color_weight: vec4<f32>,
    subsurface_color_weight: vec4<f32>,
    coat_color_weight: vec4<f32>,
    emission_color_weight: vec4<f32>,  // rgb=emission, a=intensity
    opacity: vec4<f32>,
    params1: vec4<f32>,  // x=diffuse_rough, y=metalness, z=spec_rough, w=spec_IOR
    params2: vec4<f32>,  // x=anisotropy, y=coat_rough, z=coat_IOR, w=visible
}

struct Ray {
    origin: vec3<f32>,
    pixel_id: u32,
    dir: vec3<f32>,
    bounce: u32,
    throughput: vec3<f32>,
    flags: u32,
}

// Layout must match Rust WfHit (32 bytes)
struct Hit {
    t: f32,              // offset 0
    instance_id: u32,    // offset 4
    _pad: vec2<u32>,     // offset 8 (padding for vec3 alignment)
    normal: vec3<f32>,   // offset 16
    hit: u32,            // offset 28
}

struct Params {
    width: u32,
    height: u32,
    max_bounces: u32,
    frame_count: u32,
    time: f32,
    guide_weight: f32,
    guide_warmup: u32,
    guide_enabled: u32,
    guide_product: u32,
    rr_enabled: u32,
    spectral_mode: u32,
    spectral_samples: u32,
    spectral_dispersion: u32,
}

// Environment params (matches megakernel EnvParams)
struct EnvParams {
    intensity: f32,
    rotation: f32,
    enabled: f32,
    use_importance_sampling: f32,
    env_width: f32,
    env_height: f32,
    global_opacity: f32,
    time: f32,
}

@group(0) @binding(0) var<storage, read> instances: array<Instance>;
@group(0) @binding(1) var<storage, read> materials: array<Material>;
@group(0) @binding(2) var<storage, read> rays_in: array<Ray>;
@group(0) @binding(3) var<storage, read> hits: array<Hit>;
@group(0) @binding(4) var<storage, read_write> rays_out: array<Ray>;
@group(0) @binding(5) var<storage, read_write> accum: array<vec4<f32>>;
@group(0) @binding(6) var<storage, read_write> counts: array<atomic<u32>>;
@group(0) @binding(7) var<uniform> params: Params;
@group(0) @binding(8) var env_map: texture_2d<f32>;
@group(0) @binding(9) var env_sampler: sampler;
@group(0) @binding(10) var<uniform> env: EnvParams;
@group(0) @binding(11) var<storage, read_write> guide: array<u32>;
// AOVs for OIDN denoiser. Written only on primary hit (ray.bounce == 0).
// Race writes across samples are safe because primary hits are deterministic.
@group(0) @binding(12) var<storage, read_write> albedo_aov: array<vec4<f32>>;
@group(0) @binding(13) var<storage, read_write> normal_aov: array<vec4<f32>>;

fn guide_base(idx: u32) -> u32 {
    return idx * 6u;
}

fn load_vec4(base: u32) -> vec4<f32> {
    let a = guide[base];
    let b = guide[base + 1u];
    let v0 = unpack2x16float(a);
    let v1 = unpack2x16float(b);
    return vec4<f32>(v0.x, v0.y, v1.x, v1.y);
}

fn store_vec4(base: u32, v: vec4<f32>) {
    guide[base] = pack2x16float(v.xy);
    guide[base + 1u] = pack2x16float(v.zw);
}

// PCG hash
fn pcg(n: u32) -> u32 {
    var h = n * 747796405u + 2891336453u;
    h = ((h >> ((h >> 28u) + 4u)) ^ h) * 277803737u;
    return (h >> 22u) ^ h;
}

fn rand(seed: ptr<function, u32>) -> f32 {
    *seed = pcg(*seed);
    return f32(*seed) / 4294967295.0;
}

const PI: f32 = 3.14159265359;

fn wavelength_to_rgb(lambda: f32) -> vec3<f32> {
    var r: f32 = 0.0;
    var g: f32 = 0.0;
    var b: f32 = 0.0;

    if lambda >= 380.0 && lambda < 440.0 {
        r = -(lambda - 440.0) / (440.0 - 380.0);
        g = 0.0;
        b = 1.0;
    } else if lambda >= 440.0 && lambda < 490.0 {
        r = 0.0;
        g = (lambda - 440.0) / (490.0 - 440.0);
        b = 1.0;
    } else if lambda >= 490.0 && lambda < 510.0 {
        r = 0.0;
        g = 1.0;
        b = -(lambda - 510.0) / (510.0 - 490.0);
    } else if lambda >= 510.0 && lambda < 580.0 {
        r = (lambda - 510.0) / (580.0 - 510.0);
        g = 1.0;
        b = 0.0;
    } else if lambda >= 580.0 && lambda < 645.0 {
        r = 1.0;
        g = -(lambda - 645.0) / (645.0 - 580.0);
        b = 0.0;
    } else if lambda >= 645.0 && lambda <= 720.0 {
        r = 1.0;
        g = 0.0;
        b = 0.0;
    }

    var factor: f32 = 1.0;
    if lambda > 700.0 {
        factor = 0.3 + 0.7 * (720.0 - lambda) / 20.0;
    } else if lambda < 420.0 {
        factor = 0.3 + 0.7 * (lambda - 380.0) / 40.0;
    }

    return vec3<f32>(r, g, b) * factor;
}

fn spectral_tint(seed: ptr<function, u32>, mode: u32, samples: u32, dispersion: u32, depth: u32) -> vec3<f32> {
    if mode == 0u {
        return vec3<f32>(1.0);
    }
    let n = max(1u, samples);
    var sum = vec3<f32>(0.0);
    for (var i = 0u; i < n; i++) {
        let l = mix(380.0, 720.0, rand(seed));
        sum += wavelength_to_rgb(l);
    }
    let base = sum / f32(n);
    let mix_amt = select(0.0, f32(depth) * 0.15, dispersion != 0u);
    let cool = vec3<f32>(base.z, base.y, base.x);
    return mix(base, cool, clamp(mix_amt, 0.0, 1.0));
}

// Direction to equirectangular UV
fn dir_to_equirect_uv(dir: vec3<f32>, rotation: f32) -> vec2<f32> {
    let phi = atan2(dir.z, dir.x) + rotation;
    let theta = asin(clamp(dir.y, -1.0, 1.0));
    return vec2<f32>(
        0.5 + phi / (2.0 * PI),
        0.5 - theta / PI
    );
}

// Atmospheric sky with Rayleigh + Mie scattering + day/night cycle
fn atmospheric_sky(dir: vec3<f32>, time: f32) -> vec3<f32> {
    // Sun orbits around - full cycle every ~60 seconds
    let sun_angle = time * 0.1;  // radians per second
    let sun_height = sin(sun_angle) * 0.8 + 0.1; // -0.7 to 0.9
    let sun_x = cos(sun_angle) * 0.6;
    let sun_z = sin(sun_angle * 0.7) * 0.4;
    let sun_dir = normalize(vec3<f32>(sun_x, sun_height, sun_z));
    let sun_dot = max(dot(dir, sun_dir), 0.0);
    
    // Rayleigh scattering (blue sky)
    let rayleigh_coeff = vec3<f32>(5.8e-6, 13.5e-6, 33.1e-6); // wavelength-dependent
    let zenith = max(dir.y, 0.0);
    let rayleigh = rayleigh_coeff * (1.0 + zenith * zenith) * 0.75;
    
    // Mie scattering (sun halo, haze)
    let mie_g = 0.76; // anisotropy
    let mie_phase = (1.0 - mie_g * mie_g) / pow(1.0 + mie_g * mie_g - 2.0 * mie_g * sun_dot, 1.5);
    let mie = mie_phase * 0.003;
    
    // Combine scattering
    let optical_depth = 1.0 / max(dir.y + 0.15, 0.05); // thicker at horizon
    let extinction = exp(-optical_depth * 0.3);
    
    // Day/night factor based on sun height
    let day_factor = clamp(sun_dir.y * 2.0 + 0.5, 0.0, 1.0);
    let sunset_factor = clamp(1.0 - abs(sun_dir.y) * 3.0, 0.0, 1.0);
    
    // Sky color from scattering - varies with time of day
    let day_sky = vec3<f32>(0.3, 0.5, 0.9);
    let sunset_sky = vec3<f32>(0.9, 0.4, 0.2);
    let night_sky = vec3<f32>(0.02, 0.02, 0.05);
    let sky_base = mix(night_sky, mix(day_sky, sunset_sky, sunset_factor), day_factor);
    let sky_blue = sky_base * (1.0 - extinction) * 2.0;
    
    // Horizon glow - stronger at sunset
    let horizon_color = mix(vec3<f32>(0.3, 0.2, 0.3), vec3<f32>(1.0, 0.5, 0.2), sunset_factor);
    let horizon_glow = horizon_color * (1.0 - zenith) * extinction * (0.3 + sunset_factor * 0.7);
    
    // Sun disk - color changes with height
    let sun_color = mix(vec3<f32>(1.0, 0.3, 0.1), vec3<f32>(1.0, 0.95, 0.9), clamp(sun_dir.y * 2.0, 0.0, 1.0));
    let sun_visible = select(0.0, 1.0, sun_dir.y > -0.1); // hide when below horizon
    let sun_disk = pow(sun_dot, 512.0) * sun_color * 20.0 * sun_visible;
    // Sun corona
    let corona = pow(sun_dot, 8.0) * sun_color * mie * 50.0 * sun_visible;
    // Sun glow
    let glow = pow(sun_dot, 2.0) * mix(vec3<f32>(0.5, 0.2, 0.1), vec3<f32>(0.4, 0.3, 0.2), day_factor) * (1.0 - zenith) * sun_visible;
    
    // Ground reflection - darker at night
    let ground_color = mix(vec3<f32>(0.02, 0.02, 0.03), vec3<f32>(0.1, 0.08, 0.06), day_factor);
    let ground = max(-dir.y, 0.0) * ground_color;
    
    // Stars at night
    let star_hash = fract(sin(dot(dir.xz, vec2<f32>(12.9898, 78.233))) * 43758.5453);
    let stars = select(0.0, star_hash * star_hash * 2.0, star_hash > 0.997) * (1.0 - day_factor);
    
    return sky_blue + horizon_glow + sun_disk + corona + glow + ground + vec3<f32>(stars);
}

// Unified sky color: EXR env map or procedural sky
fn sky_color(dir: vec3<f32>) -> vec3<f32> {
    if env.enabled > 0.5 {
        let uv = dir_to_equirect_uv(dir, env.rotation);
        let color = textureSampleLevel(env_map, env_sampler, uv, 0.0).rgb;
        return color * env.intensity;
    } else {
        return atmospheric_sky(dir, env.time) * env.intensity;
    }
}

// Cosine-weighted hemisphere sampling
fn sample_cosine(n: vec3<f32>, seed: ptr<function, u32>) -> vec3<f32> {
    let u1 = rand(seed);
    let u2 = rand(seed);
    let r = sqrt(u1);
    let theta = 6.283185 * u2;
    let x = r * cos(theta);
    let y = r * sin(theta);
    let z = sqrt(1.0 - u1);

    // Build TBN from normal
    var up = vec3<f32>(0.0, 1.0, 0.0);
    if abs(n.y) > 0.999 { up = vec3<f32>(1.0, 0.0, 0.0); }
    let t = normalize(cross(up, n));
    let b = cross(n, t);

    return t * x + b * y + n * z;
}

@compute @workgroup_size(64)
fn main(@builtin(global_invocation_id) gid: vec3<u32>) {
    let ray_count = atomicLoad(&counts[0]);
    if gid.x >= ray_count { return; }

    let ray = rays_in[gid.x];
    let hit = hits[gid.x];
    let pixel_id = ray.pixel_id;

    var seed = pixel_id ^ (params.frame_count * 1973u) ^ (ray.bounce * 7919u);

    // Skip inactive rays (e.g. pixels that reached SPP limit).
    if (ray.flags & 1u) == 0u {
        return;
    }

    // Clear primary sample slot by default to avoid stale data on misses
    if ray.bounce == 0u {
        let base = guide_base(pixel_id);
        store_vec4(base + 2u, vec4<f32>(0.0));
        store_vec4(base + 4u, vec4<f32>(0.0));
    }

    // Miss - accumulate sky color and end path
    if hit.hit == 0u {
        if ray.bounce == 0u {
            // Primary miss: zero out AOVs so OIDN sees a deterministic
            // "background" (the sky is the visible color; the denoiser is
            // told there's no surface here).
            albedo_aov[pixel_id] = vec4<f32>(0.0);
            normal_aov[pixel_id] = vec4<f32>(0.0);
        }
        let sky = sky_color(ray.dir);
        let tint = spectral_tint(&seed, params.spectral_mode, params.spectral_samples, params.spectral_dispersion, ray.bounce);
        let contrib = ray.throughput * sky * tint;
        accum[pixel_id] += vec4<f32>(contrib, 1.0);  // +1.0 for sample count
        return;
    }

    let inst = instances[hit.instance_id];
    let mat = materials[inst.material_id];
    let hit_pos = ray.origin + ray.dir * hit.t;

    // Primary-hit AOVs for OIDN. base_color * instance tint = surface albedo
    // before any tone curve. World-space normal goes through unchanged; OIDN
    // accepts both view-space and world-space as long as it's consistent.
    if ray.bounce == 0u {
        let primary_albedo = mat.base_color_weight.rgb * inst.color.rgb;
        albedo_aov[pixel_id] = vec4<f32>(primary_albedo, 1.0);
        normal_aov[pixel_id] = vec4<f32>(hit.normal, 1.0);
    }

    // Accumulate emission (if any)
    let emission = mat.emission_color_weight.rgb * mat.emission_color_weight.a;
    if length(emission) > 0.0 {
        let tint = spectral_tint(&seed, params.spectral_mode, params.spectral_samples, params.spectral_dispersion, ray.bounce);
        accum[pixel_id] += vec4<f32>(ray.throughput * emission * tint, 0.0);  // don't add to sample count yet
    }

    // Check max bounces - terminate path
    if ray.bounce >= params.max_bounces {
        // Path ended at max bounces - add black contribution (no light found)
        accum[pixel_id] += vec4<f32>(0.0, 0.0, 0.0, 1.0);  // +1.0 for sample count
        return;
    }

    // Russian roulette (after first bounce)
    var continue_prob = 1.0;
    if params.rr_enabled != 0u && ray.bounce > 0u {
        continue_prob = min(max(ray.throughput.x, max(ray.throughput.y, ray.throughput.z)), 0.95);
        if rand(&seed) > continue_prob {
            // Path terminated by RR - add black contribution
            accum[pixel_id] += vec4<f32>(0.0, 0.0, 0.0, 1.0);  // +1.0 for sample count
            return;
        }
    }

    let go = env.global_opacity;
    let transmission_weight = mat.transmission_color_weight.a * go;
    let transmission_color = mat.transmission_color_weight.rgb;
    let ior = mat.params1.w;
    let dispersion = clamp(mat.params2.x, 0.0, 1.0);
    let ior_r = ior * (1.0 + dispersion * 0.15);
    let ior_g = ior;
    let ior_b = ior * (1.0 - dispersion * 0.15);

    // Sample new direction (diffuse + optional transmission, with optional guiding)
    var new_dir = sample_cosine(hit.normal, &seed);
    var pdf = 0.0;
    let use_guided = params.guide_enabled != 0u && params.frame_count >= params.guide_warmup;
    var guided_dir = vec3<f32>(0.0);
    var guided_pdf = 0.0;
    if use_guided && ray.bounce == 0u {
        let base = guide_base(pixel_id);
        let guided = load_vec4(base);
        if dot(guided.xyz, guided.xyz) > 0.0001 {
            guided_dir = normalize(guided.xyz);
            guided_pdf = max(guided.w, 1e-5);
        }
    }

    if use_guided && ray.bounce == 0u && guided_pdf > 0.0 {
        if params.guide_product != 0u {
            new_dir = guided_dir;
            let cos_g = max(dot(new_dir, hit.normal), 0.0);
            pdf = max(guided_pdf * cos_g, 1e-5);
        } else if params.guide_weight > 0.0 {
            let pick = rand(&seed);
            if pick < params.guide_weight {
                new_dir = guided_dir;
                pdf = max(params.guide_weight * guided_pdf, 1e-5);
            }
        }
    }
    let cos_theta = dot(new_dir, hit.normal);
    if pdf == 0.0 {
        let mix_w = select(1.0 - params.guide_weight, 1.0, params.guide_product != 0u);
        pdf = max(mix_w * (cos_theta / PI), 1e-5);
    }

    // Transmission branch (simple refraction)
    if transmission_weight > 0.0 && rand(&seed) < transmission_weight {
        let eta = select(ior, 1.0 / ior, dot(hit.normal, ray.dir) < 0.0);
        let cos_i = dot(-ray.dir, hit.normal);
        let sin2_t = eta * eta * (1.0 - cos_i * cos_i);
        if sin2_t > 1.0 {
            // Total internal reflection
            new_dir = reflect(ray.dir, hit.normal);
        } else {
            let cos_t = sqrt(1.0 - sin2_t);
            new_dir = normalize(eta * ray.dir + (eta * cos_i - cos_t) * hit.normal);
        }
        let trans_tint = vec3<f32>(ior_r, ior_g, ior_b) / max(ior, 0.0001);
        // Spectral wavelength tint at transmission events (parity with
        // megakernel). When spectral_mode==Off this is (1,1,1) no-op.
        let spec_tint = spectral_tint(&seed, params.spectral_mode, params.spectral_samples, params.spectral_dispersion, ray.bounce);
        let transmission_color_disp = transmission_color * trans_tint * spec_tint;
        let new_throughput = ray.throughput * transmission_color_disp / max(transmission_weight * continue_prob, 1e-5);
        let out_idx = atomicAdd(&counts[1], 1u);
        rays_out[out_idx] = Ray(
            hit_pos - hit.normal * 0.001,
            pixel_id,
            new_dir,
            ray.bounce + 1u,
            new_throughput,
            1u
        );
        return;
    }

    // Update throughput (diffuse)
    let albedo = mat.base_color_weight.rgb * inst.color.rgb;
    let brdf = albedo / PI;
    let new_throughput = ray.throughput * brdf * cos_theta / (pdf * continue_prob);

    // Write new ray
    let out_idx = atomicAdd(&counts[1], 1u);
    rays_out[out_idx] = Ray(
        hit_pos + hit.normal * 0.001,
        pixel_id,
        new_dir,
        ray.bounce + 1u,
        new_throughput,
        1u
    );

    // Record path sample for guiding (primary hit only)
    if ray.bounce == 0u {
        let guide_radiance = emission + sky_color(new_dir);
        let base = guide_base(pixel_id);
        store_vec4(base + 2u, vec4<f32>(hit_pos, 0.0));
        store_vec4(base + 4u, vec4<f32>(guide_radiance * ray.throughput, 0.0));
    }
}
