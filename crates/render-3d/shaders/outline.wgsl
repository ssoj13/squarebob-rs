// Hover highlight post-process shader
// Reads object ID texture and applies outline and/or tint effects
// Mode: 0=none, 1=outline, 2=tint, 3=both
// Selected objects have SELECTED_BIT set in object_id texture
// Ported from alembic-rs

/// Bit flag for selected objects (matches cube_object_id.wgsl)
const SELECTED_BIT: u32 = 0x80000000u;

struct HoverParams {
    hovered_id: u32,           // ID of hovered object (0 = none)
    mode: u32,                 // 0=none, 1=outline, 2=tint, 3=both
    outline_width: f32,        // Outline thickness in pixels
    _pad0: f32,
    outline_color: vec4<f32>,  // Outline color (orange by default)
    tint_color: vec4<f32>,     // Tint overlay color
    viewport_size: vec2<f32>,  // Viewport dimensions
    _pad1: vec2<f32>,
}

@group(0) @binding(0) var id_texture: texture_2d<u32>;
@group(0) @binding(1) var<uniform> params: HoverParams;

struct VertexOutput {
    @builtin(position) position: vec4<f32>,
    @location(0) uv: vec2<f32>,
}

// Fullscreen triangle (3 vertices cover entire screen)
@vertex
fn vs_main(@builtin(vertex_index) vertex_idx: u32) -> VertexOutput {
    var out: VertexOutput;
    let x = f32(i32(vertex_idx & 1u) * 4 - 1);
    let y = f32(i32(vertex_idx >> 1u) * 4 - 1);
    out.position = vec4<f32>(x, y, 0.0, 1.0);
    out.uv = vec2<f32>((x + 1.0) * 0.5, (1.0 - y) * 0.5);
    return out;
}

// Sample object ID with bounds checking
fn sample_id(pos: vec2<i32>) -> u32 {
    if pos.x < 0 || pos.y < 0 ||
       pos.x >= i32(params.viewport_size.x) ||
       pos.y >= i32(params.viewport_size.y) {
        return 0u;
    }
    return textureLoad(id_texture, pos, 0).r;
}

/// Check if pixel is selected (has SELECTED_BIT)
fn is_selected_at(pos: vec2<i32>) -> f32 {
    let id = sample_id(pos);
    if (id & SELECTED_BIT) != 0u { return 1.0; }
    return 0.0;
}

/// Check if pixel is hovered (matches hovered_id, not selected)
fn is_hovered_at(pos: vec2<i32>) -> f32 {
    let id = sample_id(pos);
    // Don't count as hovered if it's selected (selected takes priority for that pixel)
    if (id & SELECTED_BIT) != 0u { return 0.0; }
    if id == params.hovered_id && params.hovered_id != 0u { return 1.0; }
    return 0.0;
}

/// Compute outline alpha for a coverage field
fn compute_outline(coverage: f32, width: f32) -> f32 {
    let edge_dist = abs(coverage - 0.5);
    let outline_band = 0.5 * (1.0 - 1.0 / (width + 1.0));
    return smoothstep(outline_band, 0.0, edge_dist);
}

@fragment
fn fs_main(in: VertexOutput) -> @location(0) vec4<f32> {
    if params.mode == 0u { return vec4<f32>(0.0); }

    let pixel = vec2<i32>(in.uv * params.viewport_size);
    let center_id = sample_id(pixel);
    
    let pixel_selected = (center_id & SELECTED_BIT) != 0u;
    let pixel_hovered = center_id == params.hovered_id && params.hovered_id != 0u && !pixel_selected;

    var result = vec4<f32>(0.0);
    let do_tint = (params.mode & 2u) != 0u;
    let do_outline = (params.mode & 1u) != 0u;

    // Tint: bright yellow for selected, orange for hovered
    if do_tint {
        if pixel_selected {
            result = vec4<f32>(1.0, 0.95, 0.1, 0.25);  // Bright yellow fill
        } else if pixel_hovered {
            result = params.tint_color;
        }
    }

    // Outline: compute separately for selected (blue) and hovered (orange)
    if do_outline {
        let width = params.outline_width;
        let search_radius = i32(ceil(width)) + 1;
        let sigma = width * 0.5 + 0.5;

        var sel_sum = 0.0;
        var hov_sum = 0.0;
        var weight_total = 0.0;

        for (var dy = -search_radius; dy <= search_radius; dy = dy + 1) {
            for (var dx = -search_radius; dx <= search_radius; dx = dx + 1) {
                let dist = length(vec2<f32>(f32(dx), f32(dy)));
                if dist <= f32(search_radius) {
                    let neighbor = pixel + vec2<i32>(dx, dy);
                    let weight = exp(-dist * dist / (2.0 * sigma * sigma));
                    
                    sel_sum += is_selected_at(neighbor) * weight;
                    hov_sum += is_hovered_at(neighbor) * weight;
                    weight_total += weight;
                }
            }
        }

        let sel_coverage = sel_sum / max(weight_total, 0.001);
        let hov_coverage = hov_sum / max(weight_total, 0.001);
        
        // Bright yellow outline for selected
        let sel_alpha = compute_outline(sel_coverage, width) * params.outline_color.a;
        if sel_alpha > 0.01 {
            result = mix(result, vec4<f32>(1.0, 0.9, 0.0, 1.0), sel_alpha);  // Bright yellow
        }
        
        // Orange outline for hovered (on top)
        let hov_alpha = compute_outline(hov_coverage, width) * params.outline_color.a;
        if hov_alpha > 0.01 {
            result = mix(result, vec4<f32>(params.outline_color.rgb, 1.0), hov_alpha);
        }
    }

    return result;
}
