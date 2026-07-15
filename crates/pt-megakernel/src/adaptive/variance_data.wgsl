// Shared per-pixel Welford state.
// Rust mirror: adaptive::VarianceData (32 bytes).
struct VarianceData {
    mean: vec3<f32>,
    _pad0: u32,
    m2: vec3<f32>,
    count: u32,
}
