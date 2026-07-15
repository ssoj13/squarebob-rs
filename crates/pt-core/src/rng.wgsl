// Shared path-tracing random-number contract.
//
// Streams are keyed by stable sample coordinates. Repeated rand() calls advance
// the dimension through the hashed state. The 24-bit conversion is exact in
// f32 and guarantees [0, 1); converting a full u32 can round 0xffffffff to 2^32
// and incorrectly produce 1.0.
fn pcg_hash(input: u32) -> u32 {
    var state = input * 747796405u + 2891336453u;
    let word = ((state >> ((state >> 28u) + 4u)) ^ state) * 277803737u;
    return (word >> 22u) ^ word;
}

fn rng_seed(
    pixel_id: u32,
    frame_id: u32,
    sample_id: u32,
    bounce: u32,
    stream: u32,
) -> u32 {
    var key = pcg_hash(pixel_id ^ 0x9e3779b9u);
    key = pcg_hash(key ^ frame_id * 0x85ebca6bu);
    key = pcg_hash(key ^ sample_id * 0xc2b2ae35u);
    key = pcg_hash(key ^ bounce * 0x27d4eb2fu);
    return pcg_hash(key ^ stream * 0x165667b1u);
}

fn rand(state: ptr<function, u32>) -> f32 {
    *state = pcg_hash(*state);
    return f32(*state >> 8u) * (1.0 / 16777216.0);
}

fn hash01(input: u32) -> f32 {
    return f32(pcg_hash(input) >> 8u) * (1.0 / 16777216.0);
}
