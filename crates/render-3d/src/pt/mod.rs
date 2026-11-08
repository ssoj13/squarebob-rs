//! Path tracing backend selection and dispatch.
//!
//! Enforces strict no-silent-fallback for explicit [`PtComputeBackend`] choices:
//! - [`PtComputeBackend::Auto`] prefers `pt-cuda` when compiled and available (and spectral is off),
//!   falling back to `pt-megakernel` WGPU compute otherwise, reporting what actually ran via
//!   [`Renderer3D::active_backend`].
//! - [`PtComputeBackend::Wgpu`] runs on WGPU (`pt-megakernel`).
//! - [`PtComputeBackend::Cuda`] runs strictly on `pt-cuda` and returns an error if unavailable or
//!   unsupported (e.g. spectral mode active or compiled without `--features cuda`).
//! - [`PtComputeBackend::Cpu`] returns an explicit error because `Renderer3D` is GPU-backed.

pub mod blit;
pub(crate) mod megakernel;
mod spectral;
pub(crate) mod wavefront;

use crate::{Renderer3D, geometry};
use render_settings::PtComputeBackend;
use render_shared::{OrbitCamera, Render3DOptions, SpectralMode};

#[derive(Debug, Copy, Clone, Eq, PartialEq)]
pub enum PtBackendKind {
    WgpuMegakernel,
    WgpuSpectral,
    CudaAuto,
    CudaExplicit,
    CpuUnsupported,
}

pub fn backend_from_opts(opts: &Render3DOptions) -> PtBackendKind {
    match opts.pt_compute_backend {
        PtComputeBackend::Cuda => PtBackendKind::CudaExplicit,
        PtComputeBackend::Cpu => PtBackendKind::CpuUnsupported,
        PtComputeBackend::Wgpu => {
            if opts.pt_spectral_mode != SpectralMode::Off {
                PtBackendKind::WgpuSpectral
            } else {
                PtBackendKind::WgpuMegakernel
            }
        }
        PtComputeBackend::Auto => {
            if opts.pt_spectral_mode != SpectralMode::Off {
                PtBackendKind::WgpuSpectral
            } else {
                PtBackendKind::CudaAuto
            }
        }
    }
}

fn cpu_backend_unsupported() -> render_core::ReadbackError {
    render_core::ReadbackError::SceneBuild(
        "CPU path-tracing backend is not supported in Renderer3D; select Auto, WGPU, or CUDA"
            .into(),
    )
}

pub fn render_path_traced_no_readback(
    kind: PtBackendKind,
    renderer: &mut Renderer3D,
    instances: &[geometry::CubeInstance],
    camera: &OrbitCamera,
    opts: &Render3DOptions,
    width: u32,
    height: u32,
) -> Result<(), render_core::ReadbackError> {
    match kind {
        PtBackendKind::WgpuMegakernel => megakernel::render_path_traced_no_readback(
            renderer, instances, camera, opts, width, height,
        ),
        PtBackendKind::WgpuSpectral => spectral::render_path_traced_no_readback(
            renderer, instances, camera, opts, width, height,
        ),
        PtBackendKind::CudaAuto => wavefront::render_path_traced_no_readback(
            renderer, instances, camera, opts, width, height, false,
        ),
        PtBackendKind::CudaExplicit => wavefront::render_path_traced_no_readback(
            renderer, instances, camera, opts, width, height, true,
        ),
        PtBackendKind::CpuUnsupported => Err(cpu_backend_unsupported()),
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
) -> Result<Vec<u8>, render_core::ReadbackError> {
    match kind {
        PtBackendKind::WgpuMegakernel => {
            megakernel::render_path_traced(renderer, instances, camera, opts, width, height)
        }
        PtBackendKind::WgpuSpectral => {
            spectral::render_path_traced(renderer, instances, camera, opts, width, height)
        }
        PtBackendKind::CudaAuto => {
            wavefront::render_path_traced(renderer, instances, camera, opts, width, height, false)
        }
        PtBackendKind::CudaExplicit => {
            wavefront::render_path_traced(renderer, instances, camera, opts, width, height, true)
        }
        PtBackendKind::CpuUnsupported => Err(cpu_backend_unsupported()),
    }
}

pub fn frame_count(kind: PtBackendKind, renderer: &Renderer3D) -> u32 {
    match kind {
        PtBackendKind::WgpuMegakernel => megakernel::frame_count(renderer),
        PtBackendKind::WgpuSpectral => spectral::frame_count(renderer),
        PtBackendKind::CudaAuto | PtBackendKind::CudaExplicit => wavefront::frame_count(renderer),
        PtBackendKind::CpuUnsupported => 0,
    }
}

pub fn pick(
    kind: PtBackendKind,
    renderer: &mut Renderer3D,
    origin: glam::Vec3,
    dir: glam::Vec3,
) -> Option<(u32, f32)> {
    match kind {
        PtBackendKind::WgpuMegakernel => megakernel::pick(renderer, origin, dir),
        PtBackendKind::WgpuSpectral => spectral::pick(renderer, origin, dir),
        PtBackendKind::CudaAuto | PtBackendKind::CudaExplicit => {
            wavefront::pick(renderer, origin, dir)
        }
        PtBackendKind::CpuUnsupported => None,
    }
}
