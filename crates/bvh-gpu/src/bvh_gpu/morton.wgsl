// Morton code computation for LBVH construction.
//
// Computes 30-bit Morton codes from instance centroids.
// Morton codes interleave x, y, z bits for spatial locality.

struct Aabb {
    min: vec4<f32>,
    max: vec4<f32>,
};

struct MortonPrimitive {
    code: u32,
    index: u32,
};

struct SceneBounds {
    min: vec4<f32>,
    max: vec4<f32>,
};

struct Params {
    count: u32,
    _pad: vec2<u32>,
};

@group(0) @binding(0) var<storage, read> aabbs: array<Aabb>;
@group(0) @binding(1) var<storage, read_write> morton_codes: array<MortonPrimitive>;
@group(0) @binding(2) var<uniform> bounds: SceneBounds;
@group(0) @binding(3) var<uniform> params: Params;

fn aabb_centroid(aabb: Aabb) -> vec3<f32> {
    return 0.5 * (aabb.min.xyz + aabb.max.xyz);
}

// Expand 10-bit integer to 30 bits by inserting 2 zeros between each bit
fn expand_bits(v: u32) -> u32 {
    var x = v & 0x3FFu; // 10 bits
    x = (x | (x << 16u)) & 0x030000FFu;
    x = (x | (x <<  8u)) & 0x0300F00Fu;
    x = (x | (x <<  4u)) & 0x030C30C3u;
    x = (x | (x <<  2u)) & 0x09249249u;
    return x;
}

// Compute 30-bit Morton code from 3D point in [0,1]^3
fn morton_3d(p: vec3<f32>) -> u32 {
    let x = clamp(p.x, 0.0, 1.0);
    let y = clamp(p.y, 0.0, 1.0);
    let z = clamp(p.z, 0.0, 1.0);
    
    let xi = u32(x * 1023.0);
    let yi = u32(y * 1023.0);
    let zi = u32(z * 1023.0);
    
    return (expand_bits(xi) << 2u) | (expand_bits(yi) << 1u) | expand_bits(zi);
}

@compute @workgroup_size(256)
fn main(@builtin(global_invocation_id) gid: vec3<u32>) {
    let idx = gid.x;
    if idx >= params.count {
        return;
    }
    
    let centroid = aabb_centroid(aabbs[idx]);
    
    // Normalize to [0,1] using scene bounds
    let extent = bounds.max.xyz - bounds.min.xyz;
    let inv_extent = select(vec3<f32>(0.0), 1.0 / extent, extent > vec3<f32>(1e-8));
    let normalized = (centroid - bounds.min.xyz) * inv_extent;
    
    // Compute Morton code
    let code = morton_3d(normalized);
    
    morton_codes[idx] = MortonPrimitive(code, idx);
}
