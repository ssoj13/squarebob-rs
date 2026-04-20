// Wavefront Finalize pass.
// Copy accumulated radiance from buffer to output texture.
// Normalize by frame count (total number of samples per pixel).

@group(0) @binding(0) var<storage, read> accum: array<vec4<f32>>;
@group(0) @binding(1) var output: texture_storage_2d<rgba32float, write>;
@group(0) @binding(2) var<uniform> params: vec4<u32>;  // width, height, frame_count, _pad

@compute @workgroup_size(8, 8)
fn main(@builtin(global_invocation_id) gid: vec3<u32>) {
    let width = params.x;
    let height = params.y;
    let frame_count = f32(params.z);

    if gid.x >= width || gid.y >= height { return; }

    let pixel_idx = gid.y * width + gid.x;
    let accumulated = accum[pixel_idx];

    // Normalize by frame count (number of samples per pixel)
    // accumulated.w contains the total number of path terminations for this pixel
    // which should equal frame_count for a proper path tracer
    var color = accumulated.rgb;
    let sample_count = accumulated.w;
    
    // Use sample_count if available, otherwise fall back to frame_count
    let divisor = max(sample_count, 1.0);
    color = color / divisor;

    // Clamp to prevent fireflies
    color = clamp(color, vec3<f32>(0.0), vec3<f32>(100.0));

    textureStore(output, vec2<i32>(gid.xy), vec4<f32>(color, 1.0));
}
