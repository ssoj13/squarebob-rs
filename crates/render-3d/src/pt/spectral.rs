//! Spectral PT backend (stub).

use crate::{geometry, Renderer3D};
use log::warn;
use render_shared::{OrbitCamera, Render3DOptions};

pub fn render_path_traced_no_readback(
    renderer: &mut Renderer3D,
    instances: &[geometry::CubeInstance],
    camera: &OrbitCamera,
    opts: &Render3DOptions,
    width: u32,
    height: u32,
) {
    if opts.pt_wavefront {
        warn!("Spectral backend stub: forcing megakernel path (pt_wavefront=false)");
    }
    let mut local_opts = opts.clone();
    local_opts.pt_wavefront = false;
    if local_opts.pt_spectral_samples < 1 {
        local_opts.pt_spectral_samples = 1;
    }
    renderer.render_path_traced_no_readback(instances, camera, &local_opts, width, height);
}

pub fn render_path_traced(
    renderer: &mut Renderer3D,
    instances: &[geometry::CubeInstance],
    camera: &OrbitCamera,
    opts: &Render3DOptions,
    width: u32,
    height: u32,
) -> Vec<u8> {
    if opts.pt_wavefront {
        warn!("Spectral backend stub: forcing megakernel path (pt_wavefront=false)");
    }
    let mut local_opts = opts.clone();
    local_opts.pt_wavefront = false;
    if local_opts.pt_spectral_samples < 1 {
        local_opts.pt_spectral_samples = 1;
    }
    renderer.render_path_traced(instances, camera, &local_opts, width, height)
}

pub fn frame_count(renderer: &Renderer3D) -> u32 {
    renderer.pt_frame_count_impl()
}

pub fn pick(renderer: &mut Renderer3D, origin: glam::Vec3, dir: glam::Vec3) -> Option<(u32, f32)> {
    renderer.pt_pick_impl(origin, dir)
}
