// Bottom-up AABB computation for LBVH.
//
// Also used for BVH REFIT during animation!
// Each leaf updates its AABB from instance data, then propagates up.
// Uses atomic counter to ensure parent is processed after both children.

struct Aabb {
    min: vec4<f32>,
    max: vec4<f32>,
};

struct MortonPrimitive {
    code: u32,
    index: u32,
};

struct LbvhNode {
    left: i32,
    right: i32,
    parent: i32,
    range_start: u32,
    range_end: u32,
    atomic_visited: atomic<u32>,
    _pad: vec2<u32>,
};

// Output BVH node (matches BvhNode in Rust)
struct BvhNode {
    aabb_min: vec3<f32>,
    left_or_first: u32,
    aabb_max: vec3<f32>,
    count: u32,
};

struct Params {
    count: u32,
    is_refit: u32,  // 1 = refit only (skip hierarchy rebuild)
    _pad: vec2<u32>,
};

@group(0) @binding(0) var<storage, read> aabbs: array<Aabb>;
@group(0) @binding(1) var<storage, read> sorted_indices: array<MortonPrimitive>;
@group(0) @binding(2) var<storage, read_write> lbvh_nodes: array<LbvhNode>;
@group(0) @binding(3) var<storage, read_write> output_nodes: array<BvhNode>;
@group(0) @binding(4) var<uniform> params: Params;
@group(0) @binding(5) var<storage, read> leaf_parents: array<u32>;

// Merge two AABBs
fn merge_aabb(a_min: vec3<f32>, a_max: vec3<f32>, b_min: vec3<f32>, b_max: vec3<f32>) -> array<vec3<f32>, 2> {
    return array<vec3<f32>, 2>(
        min(a_min, b_min),
        max(a_max, b_max),
    );
}

// Walk up the tree from a node, computing internal AABBs
fn walk_up(start_node_idx: u32) {
    var node_idx = start_node_idx;
    var safety_count = 0u;
    
    loop {
        safety_count += 1u;
        if safety_count > 100000u {
            return; // Safety break to prevent TDR/hangs
        }
        
        // Bounds check for internal node index
        if node_idx >= params.count - 1u {
            return;
        }

        // Atomic flag to synchronize children
        // 0: no children visited
        // 1: one child visited
        // 2: both children visited (reset to 0)
        let visited = atomicAdd(&lbvh_nodes[node_idx].atomic_visited, 1u);
        
        if visited == 0u {
            // First child to arrive. Terminate.
            return;
        }
        
        // Second child to arrive. We are responsible for processing this node.
        // Reset flag for next frame
        atomicStore(&lbvh_nodes[node_idx].atomic_visited, 0u);
        
        // Read fields individually to avoid copying atomic field (Metal doesn't support copying atomics)
        let node_left = lbvh_nodes[node_idx].left;
        let node_right = lbvh_nodes[node_idx].right;
        
        // Compute AABB from children
        var left_min: vec3<f32>;
        var left_max: vec3<f32>;
        var right_min: vec3<f32>;
        var right_max: vec3<f32>;
        
        // Load left child AABB
        if node_left < 0 {
            // Leaf
            let leaf_idx = u32(-node_left - 1);
            let leaf_output = output_nodes[params.count - 1u + leaf_idx];
            left_min = leaf_output.aabb_min;
            left_max = leaf_output.aabb_max;
        } else {
            // Internal
            let child = output_nodes[u32(node_left)];
            left_min = child.aabb_min;
            left_max = child.aabb_max;
        }
        
        // Load right child AABB
        if node_right < 0 {
            // Leaf
            let leaf_idx = u32(-node_right - 1);
            let leaf_output = output_nodes[params.count - 1u + leaf_idx];
            right_min = leaf_output.aabb_min;
            right_max = leaf_output.aabb_max;
        } else {
            // Internal
            let child = output_nodes[u32(node_right)];
            right_min = child.aabb_min;
            right_max = child.aabb_max;
        }
        
        // Merge
        let merged = merge_aabb(left_min, left_max, right_min, right_max);
        
        // Store internal node AABB
        output_nodes[node_idx].aabb_min = merged[0];
        output_nodes[node_idx].aabb_max = merged[1];
        output_nodes[node_idx].left_or_first = u32(node_left); // Preserved
        output_nodes[node_idx].count = 0u; // Internal marker
        
        // Move up to parent (root is the node with parent < 0; it is not guaranteed to be 0).
        let parent = lbvh_nodes[node_idx].parent;
        if parent < 0 {
            return; // Should not happen if tree is valid and we are not root
        }
        node_idx = u32(parent);
    }
}

@compute @workgroup_size(256)
fn main(@builtin(global_invocation_id) gid: vec3<u32>) {
    let n = params.count;
    let leaf_idx = gid.x;
    
    if leaf_idx >= n {
        return;
    }
    
    // Get the original instance index from sorted order
    let original_idx = sorted_indices[leaf_idx].index;
    let aabb = aabbs[original_idx];
    let current_min = aabb.min.xyz;
    let current_max = aabb.max.xyz;
    
    // Store leaf node in output (leaves are at indices [n-1, 2n-2])
    let leaf_output_idx = n - 1u + leaf_idx;
    output_nodes[leaf_output_idx].aabb_min = current_min;
    output_nodes[leaf_output_idx].aabb_max = current_max;
    output_nodes[leaf_output_idx].left_or_first = leaf_idx;
    output_nodes[leaf_output_idx].count = 1u; // leaf marker
    
    if n > 1u {
        // Start walking up from parent
        let parent = leaf_parents[leaf_idx];
        walk_up(parent);
    }
}

// REFIT pass: update AABBs only, preserve hierarchy
@compute @workgroup_size(256)
fn refit_leaves(@builtin(global_invocation_id) gid: vec3<u32>) {
    let n = params.count;
    let leaf_idx = gid.x;
    
    if leaf_idx >= n {
        return;
    }
    
    // Get instance from existing sorted order
    let original_idx = sorted_indices[leaf_idx].index;
    let aabb = aabbs[original_idx];
    
    // Update leaf AABB only
    let leaf_output_idx = n - 1u + leaf_idx;
    output_nodes[leaf_output_idx].aabb_min = aabb.min.xyz;
    output_nodes[leaf_output_idx].aabb_max = aabb.max.xyz;
    
    if n > 1u {
        // Start walking up from parent
        let parent = leaf_parents[leaf_idx];
        walk_up(parent);
    }
}
