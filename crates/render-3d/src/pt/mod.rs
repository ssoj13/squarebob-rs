//! Path tracing backend selection and dispatch.

pub(crate) mod megakernel;
mod spectral;
mod wavefront;

use crate::{geometry, Renderer3D};
use render_shared::{OrbitCamera, Render3DOptions, SpectralMode};

#[derive(Debug, Copy, Clone, Eq, PartialEq)]
pub enum PtBackendKind {
    Megakernel,
    Wavefront,
    #[allow(dead_code)]
    Spectral,
}

pub fn backend_from_opts(opts: &Render3DOptions) -> PtBackendKind {
    if opts.pt_spectral_mode != SpectralMode::Off {
        PtBackendKind::Spectral
    } else if opts.pt_wavefront {
        PtBackendKind::Wavefront
    } else {
        PtBackendKind::Megakernel
    }
}

pub fn render_path_traced_no_readback(
    kind: PtBackendKind,
    renderer: &mut Renderer3D,
    instances: &[geometry::CubeInstance],
    camera: &OrbitCamera,
    opts: &Render3DOptions,
    width: u32,
    height: u32,
) {
    match kind {
        PtBackendKind::Megakernel => {
            megakernel::render_path_traced_no_readback(renderer, instances, camera, opts, width, height);
        }
        PtBackendKind::Wavefront => {
            wavefront::render_path_traced_no_readback(renderer, instances, camera, opts, width, height);
        }
        PtBackendKind::Spectral => {
            spectral::render_path_traced_no_readback(renderer, instances, camera, opts, width, height);
        }
    }
}

pub fn render_path_traced(
    kind: PtBackendKind,
    renderer: &mut Renderer3D,
    instances: &[geometry::CubeInstance],
    camera: &OrbitCamera,
    opts: &Render3DOptions,
    width: u32,
    height: u32,
) -> Vec<u8> {
    match kind {
        PtBackendKind::Megakernel => {
            megakernel::render_path_traced(renderer, instances, camera, opts, width, height)
        }
        PtBackendKind::Wavefront => {
            wavefront::render_path_traced(renderer, instances, camera, opts, width, height)
        }
        PtBackendKind::Spectral => {
            spectral::render_path_traced(renderer, instances, camera, opts, width, height)
        }
    }
}

pub fn frame_count(kind: PtBackendKind, renderer: &Renderer3D) -> u32 {
    match kind {
        PtBackendKind::Megakernel => megakernel::frame_count(renderer),
        PtBackendKind::Wavefront => wavefront::frame_count(renderer),
        PtBackendKind::Spectral => spectral::frame_count(renderer),
    }
}

pub fn pick(kind: PtBackendKind, renderer: &mut Renderer3D, origin: glam::Vec3, dir: glam::Vec3) -> Option<(u32, f32)> {
    match kind {
        PtBackendKind::Megakernel => megakernel::pick(renderer, origin, dir),
        PtBackendKind::Wavefront => wavefront::pick(renderer, origin, dir),
        PtBackendKind::Spectral => spectral::pick(renderer, origin, dir),
    }
}
