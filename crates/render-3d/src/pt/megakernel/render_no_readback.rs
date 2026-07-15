//! Zero-copy output policy for the shared megakernel PT frame state machine.

use super::*;

pub(crate) fn render_path_traced_no_readback(
    renderer: &mut Renderer3D,
    instances: &[geometry::CubeInstance],
    camera: &OrbitCamera,
    opts: &Render3DOptions,
    width: u32,
    height: u32,
) -> Result<(), render_core::ReadbackError> {
    super::render::render_path_traced_frame(
        renderer,
        instances,
        camera,
        opts,
        width,
        height,
        super::render::PtOutput::GpuOnly,
    )?;
    Ok(())
}
