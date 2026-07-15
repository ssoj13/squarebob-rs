extern crate self as render_core;

#[derive(Debug)]
pub struct GpuLayoutError(&'static str);

impl std::fmt::Display for GpuLayoutError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(self.0)
    }
}

impl std::error::Error for GpuLayoutError {}

pub fn checked_2d_byte_len(
    _label: &'static str,
    width: u32,
    height: u32,
    bytes_per_pixel: u32,
) -> Result<usize, GpuLayoutError> {
    if width == 0 || height == 0 || bytes_per_pixel == 0 {
        return Err(GpuLayoutError("zero-sized 2D layout"));
    }
    let bytes = u64::from(width)
        .checked_mul(u64::from(height))
        .and_then(|pixels| pixels.checked_mul(u64::from(bytes_per_pixel)))
        .ok_or(GpuLayoutError("2D layout overflow"))?;
    usize::try_from(bytes).map_err(|_| GpuLayoutError("2D layout exceeds usize"))
}

#[path = "../../../crates/treemap/src/lib.rs"]
pub mod treemap;
