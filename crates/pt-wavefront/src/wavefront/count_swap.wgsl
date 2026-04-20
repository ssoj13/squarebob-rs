// Wavefront count swap pass.
// Moves count_out -> count_in and clears count_out.

@group(0) @binding(0) var<storage, read_write> counts: array<atomic<u32>>;

@compute @workgroup_size(1)
fn main(@builtin(global_invocation_id) gid: vec3<u32>) {
    if gid.x != 0u { return; }
    let out_count = atomicLoad(&counts[1]);
    atomicStore(&counts[0], out_count);
    atomicStore(&counts[1], 0u);
}
