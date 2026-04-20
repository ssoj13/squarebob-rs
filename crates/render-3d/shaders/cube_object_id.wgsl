// Object ID shader for hover picking
// Renders instanced cubes to R32Uint texture, each pixel = object_id
// Used for mouse hover detection and selection
// Selected objects have SELECTED_BIT (0x80000000) set in their ID

/// Bit flag for selected objects in object_id texture
const SELECTED_BIT: u32 = 0x80000000u;

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

/// Selected IDs storage buffer: [count, id0, id1, ...] - unlimited size
struct SelectedIds {
    count: u32,
    ids: array<u32>,  // Runtime-sized array - unlimited!
}

@group(0) @binding(0) var<uniform> camera: Camera;
@group(0) @binding(1) var<storage, read> selected: SelectedIds;

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
}

struct VertexOutput {
    @builtin(position) position: vec4<f32>,
    @location(0) @interpolate(flat) object_id: u32,
}

@vertex
fn vs_main(v: VertexInput, i: InstanceInput) -> VertexOutput {
    let model = mat4x4<f32>(i.model_0, i.model_1, i.model_2, i.model_3);
    let wp = model * vec4<f32>(v.position, 1.0);

    var out: VertexOutput;
    out.position = camera.view_proj * wp;
    out.object_id = i.object_id;
    return out;
}

/// Check if object_id is in selected set (unlimited size)
fn is_selected(id: u32) -> bool {
    let count = selected.count;
    for (var i = 0u; i < count; i = i + 1u) {
        if selected.ids[i] == id {
            return true;
        }
    }
    return false;
}

@fragment
fn fs_main(in: VertexOutput) -> @location(0) u32 {
    // Mark selected objects with SELECTED_BIT
    if is_selected(in.object_id) {
        return in.object_id | SELECTED_BIT;
    }
    return in.object_id;
}
