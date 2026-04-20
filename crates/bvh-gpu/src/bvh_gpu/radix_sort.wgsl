// Stable radix sort for Morton codes (block-level scatter).
//
// Pass 1: per-block histogram into block_hist (size = num_blocks * 256).
// Pass 2: CPU computes block_offsets (exclusive prefix by blocks per digit).
// Pass 3: per-block stable scatter in increasing index order.

struct MortonPrimitive {
    code: u32,
    index: u32,
};

struct Params {
    count: u32,
    pass_id: u32,   // 0-3 for each 8-bit radix pass
    _pad: vec2<u32>,
};

@group(0) @binding(0) var<storage, read> input: array<MortonPrimitive>;
@group(0) @binding(1) var<storage, read_write> output: array<MortonPrimitive>;
@group(0) @binding(2) var<storage, read_write> block_hist: array<atomic<u32>>;
@group(0) @binding(3) var<storage, read_write> block_offsets: array<u32>;
@group(0) @binding(4) var<uniform> params: Params;

const RADIX_BITS: u32 = 8u;
const RADIX_SIZE: u32 = 256u; // 2^8
const WG_SIZE: u32 = 256u;

var<workgroup> local_histogram: array<atomic<u32>, 256>;
// Extract radix digit from key
fn get_digit(key: u32, pass_id: u32) -> u32 {
    let shift = pass_id * RADIX_BITS;
    return (key >> shift) & (RADIX_SIZE - 1u);
}

// Phase 1: Count per-block histogram
@compute @workgroup_size(256)
fn count_histogram(@builtin(global_invocation_id) gid: vec3<u32>, @builtin(local_invocation_id) lid: vec3<u32>, @builtin(workgroup_id) wid: vec3<u32>) {
    // Clear local histogram
    local_histogram[lid.x] = 0u;
    workgroupBarrier();
    
    let idx = gid.x;
    if idx < params.count {
        let digit = get_digit(input[idx].code, params.pass_id);
        atomicAdd(&local_histogram[digit], 1u);
    }
    workgroupBarrier();
    
    // Write to per-block histogram (block_hist[block_id * 256 + digit])
    if lid.x < RADIX_SIZE {
        let block_base = wid.x * RADIX_SIZE;
        atomicStore(&block_hist[block_base + lid.x], atomicLoad(&local_histogram[lid.x]));
    }
}

// Phase 2: Stable scatter per block (single thread per block).
// block_offsets is computed on CPU: for each digit, exclusive prefix by blocks.
@compute @workgroup_size(1)
fn scatter(@builtin(workgroup_id) wid: vec3<u32>) {
    let block_id = wid.x;
    let block_start = block_id * WG_SIZE;
    let block_end = min(block_start + WG_SIZE, params.count);
    if block_start >= params.count {
        return;
    }

    // Local offsets per digit for this block
    var local_offsets: array<u32, 256>;
    for (var d = 0u; d < RADIX_SIZE; d++) {
        let base = block_id * RADIX_SIZE + d;
        local_offsets[d] = block_offsets[base];
    }

    // Stable: iterate in increasing index order within block
    for (var idx = block_start; idx < block_end; idx++) {
        let item = input[idx];
        let digit = get_digit(item.code, params.pass_id);
        let pos = local_offsets[digit];
        output[pos] = item;
        local_offsets[digit] = pos + 1u;
    }
}
