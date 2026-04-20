// LBVH hierarchy construction (Karras 2012).
//
// Builds binary radix tree from sorted Morton codes.
// Each internal node determined by finding split point using
// longest common prefix (LCP) between adjacent codes.

struct MortonPrimitive {
    code: u32,
    index: u32,
};

struct LbvhNode {
    left: i32,        // left child index (negative = leaf)
    right: i32,       // right child index
    parent: i32,      // parent index
    range_start: u32, // leaf range start
    range_end: u32,   // leaf range end
    atomic_visited: atomic<u32>, // for bottom-up AABB computation
    _pad: vec2<u32>,
};

struct Params {
    count: u32,       // number of primitives (leaves)
    _pad: vec2<u32>,
};

@group(0) @binding(0) var<storage, read> sorted_morton: array<MortonPrimitive>;
@group(0) @binding(1) var<storage, read_write> nodes: array<LbvhNode>;
@group(0) @binding(2) var<uniform> params: Params;
@group(0) @binding(3) var<storage, read_write> leaf_parents: array<u32>;

// Count leading zeros
fn clz(x: u32) -> u32 {
    if x == 0u { return 32u; }
    var v = x;
    var n = 0u;
    if (v & 0xFFFF0000u) == 0u { n += 16u; v <<= 16u; }
    if (v & 0xFF000000u) == 0u { n += 8u; v <<= 8u; }
    if (v & 0xF0000000u) == 0u { n += 4u; v <<= 4u; }
    if (v & 0xC0000000u) == 0u { n += 2u; v <<= 2u; }
    if (v & 0x80000000u) == 0u { n += 1u; }
    return n;
}

// Longest common prefix between Morton codes at indices i and j
fn delta(i: i32, j: i32, n: u32) -> i32 {
    // Out of range returns -1
    if j < 0 || u32(j) >= n {
        return -1;
    }
    
    let code_i = sorted_morton[u32(i)].code;
    let code_j = sorted_morton[u32(j)].code;
    
    // If codes are equal, use index to break ties
    if code_i == code_j {
        // Use position indices i, j as tie-breaker to ensure strict ordering
        // even if original object indices are not sorted.
        return i32(32u + clz(u32(i) ^ u32(j)));
    }
    
    return i32(clz(code_i ^ code_j));
}

// Determine range of keys covered by internal node i (Karras 2012)
fn determine_range(i: i32, n: u32) -> vec2<i32> {
    // Determine direction of the range (+1 or -1)
    let d_left = delta(i, i - 1, n);
    let d_right = delta(i, i + 1, n);
    let d = select(-1, 1, d_right > d_left);

    // Compute upper bound for the length of the range
    let d_min = delta(i, i - d, n);
    var l_max = 2;
    while delta(i, i + l_max * d, n) > d_min {
        l_max = l_max * 2;
        if l_max > i32(n) { break; }
    }

    // Binary search to find the other end
    var l = 0;
    var t = l_max / 2;
    while t > 0 {
        if delta(i, i + (l + t) * d, n) > d_min {
            l = l + t;
        }
        t = t / 2;
    }

    let j = i + l * d;
    return vec2<i32>(min(i, j), max(i, j));
}

// Find split position within range (Karras 2012 / NVIDIA devblog)
fn find_split(first: i32, last: i32, n: u32) -> i32 {
    let common_prefix = delta(first, last, n);
    var split = first;
    var step = last - first;

    loop {
        step = (step + 1) / 2;
        let new_split = split + step;

        if new_split < last {
            let split_delta = delta(first, new_split, n);
            if split_delta > common_prefix {
                split = new_split;
            }
        }

        if step <= 1 {
            break;
        }
    }

    return split;
}

@compute @workgroup_size(256)
fn main(@builtin(global_invocation_id) gid: vec3<u32>) {
    let n = params.count;
    let idx = gid.x;
    
    // n-1 internal nodes for n leaves
    if idx >= n - 1u {
        return;
    }
    
    let i = i32(idx);
    let range = determine_range(i, n);
    let first = range.x;
    let last = range.y;
    
    // Split position γ
    let split_l = find_split(first, last, n);

    // Left child
    var left_child: i32;
    if first == split_l {
        left_child = -(split_l + 1);
        leaf_parents[u32(split_l)] = idx;
    } else {
        left_child = split_l;
    }

    // Right child
    var right_child: i32;
    let first_r = split_l + 1;
    if first_r == last {
        right_child = -(first_r + 1);
        leaf_parents[u32(first_r)] = idx;
    } else {
        right_child = first_r;
    }
    
    // Store internal node
    nodes[idx].left = left_child;
    nodes[idx].right = right_child;
    nodes[idx].range_start = u32(first);
    nodes[idx].range_end = u32(last);
    atomicStore(&nodes[idx].atomic_visited, 0u);
    
    // Set parent pointers for internal children
    if left_child >= 0 {
        nodes[u32(left_child)].parent = i32(idx);
    }
    if right_child >= 0 {
        nodes[u32(right_child)].parent = i32(idx);
    }
}

// Initialize internal nodes (parent = -1, reset fields)
@compute @workgroup_size(256)
fn init_nodes(@builtin(global_invocation_id) gid: vec3<u32>) {
    let n = params.count;
    let idx = gid.x;
    if idx >= n - 1u {
        return;
    }
    nodes[idx].left = 0;
    nodes[idx].right = 0;
    nodes[idx].parent = -1;
    nodes[idx].range_start = 0u;
    nodes[idx].range_end = 0u;
    atomicStore(&nodes[idx].atomic_visited, 0u);
}

// Separate kernel for initializing leaf parent pointers
@compute @workgroup_size(256)
fn init_leaves(@builtin(global_invocation_id) gid: vec3<u32>) {
    let n = params.count;
    let idx = gid.x;
    
    if idx >= n {
        return;
    }
    
    // Leaf nodes conceptually at [n-1, 2n-2]
    // But we track them separately via sorted_morton indices
    // Parent will be set by internal node construction
}
