//! Megakernel PT backend implementation.

use crate::{compute_slice_normal, compute_slice_position, geometry, Renderer3D};
use log::{debug, info, trace};
use render_core::gpu;
use render_shared::{HashTransformEffect, OrbitCamera, Render3DOptions};

mod render;
mod render_no_readback;

pub(crate) use render::render_path_traced;
pub(crate) use render_no_readback::render_path_traced_no_readback;

pub fn frame_count(renderer: &Renderer3D) -> u32 {
    renderer.pt_frame_count_impl()
}

pub fn pick(renderer: &mut Renderer3D, origin: glam::Vec3, dir: glam::Vec3) -> Option<(u32, f32)> {
    renderer.pt_pick_impl(origin, dir)
}
