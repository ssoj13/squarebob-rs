//! CPU ray-AABB picking against the cube tree.
//!
//! Extracted from `lib.rs` in the post-sprint-3 modularization
//! pass. `cpu_pick` and its helper `pick_recursive` is still a method of `Renderer3D` —
//! `impl Renderer3D` is re-opened here.

use glam::{Vec3, Vec4};

use dirstat_core::DirEntry;
use render_shared::{hash_transform_offset, OrbitCamera, Render3DOptions};
use treemap::TreeMapOptions;

use crate::{ray_aabb_intersect, CpuPickHit, Renderer3D};

impl Renderer3D {
    #[allow(clippy::too_many_arguments)]
    pub fn cpu_pick(
        &self,
        root: &DirEntry,
        width: u32,
        height: u32,
        camera: &OrbitCamera,
        opts: &Render3DOptions,
        treemap_opts: &TreeMapOptions,
        screen_x: f32,
        screen_y: f32,
    ) -> Option<CpuPickHit> {
        if width == 0 || height == 0 {
            return None;
        }

        // Ensure layout matches the logical 3D scene, not the current render target size.
        let (layout_w, layout_h) = self.current_scene_layout_size();
        treemap::layout(
            root,
            0.0,
            0.0,
            layout_w as f32,
            layout_h as f32,
            treemap_opts,
        );

        let rel_x = (screen_x / width as f32).clamp(0.0, 1.0);
        let rel_y = (screen_y / height as f32).clamp(0.0, 1.0);

        let aspect = width as f32 / height as f32;
        let view = camera.view_matrix();
        let proj = camera.projection_matrix(aspect);
        let inv_view_proj = (proj * view).inverse();

        // NDC: x in [-1,1], y in [-1,1], z in [0,1]
        let ndc_x = rel_x * 2.0 - 1.0;
        let ndc_y = 1.0 - rel_y * 2.0;

        let near = Vec4::new(ndc_x, ndc_y, 0.0, 1.0);
        let far = Vec4::new(ndc_x, ndc_y, 1.0, 1.0);

        let near_world4 = inv_view_proj * near;
        let far_world4 = inv_view_proj * far;
        let near_world = near_world4.truncate() / near_world4.w;
        let far_world = far_world4.truncate() / far_world4.w;

        let ray_origin = near_world;
        let ray_dir = (far_world - near_world).normalize_or_zero();
        if ray_dir == Vec3::ZERO {
            return None;
        }

        let world_center = Vec3::new(layout_w as f32 / 2.0, -(layout_h as f32 / 2.0), 0.0);
        let mut hit: Option<CpuPickHit> = None;
        self.pick_recursive(
            root,
            0,
            0,
            opts,
            treemap_opts,
            world_center,
            ray_origin,
            ray_dir,
            &mut hit,
        );
        hit
    }

    #[allow(clippy::too_many_arguments)]
    fn pick_recursive(
        &self,
        node: &DirEntry,
        depth: u32,
        dir_hash: u32,
        opts: &Render3DOptions,
        _treemap_opts: &TreeMapOptions,
        world_center: Vec3,
        ray_origin: Vec3,
        ray_dir: Vec3,
        hit: &mut Option<CpuPickHit>,
    ) {
        let [x, y, w, h] = node.rect.get();
        if w < 1.0 || h < 1.0 || node.size == 0 {
            return;
        }

        let too_small = w < treemap::MIN_RECT_SIZE || h < treemap::MIN_RECT_SIZE;

        if !node.is_dir || node.children.is_empty() || too_small {
            let base_height = Self::compute_cube_height(node, depth, opts);

            // Mirror instance_collect cube placement.
            let pos = Vec3::new(x + w / 2.0, -(y + h / 2.0), -base_height / 2.0);
            let offset = hash_transform_offset(
                &node.name,
                pos,
                world_center,
                opts.hash_effect,
                opts.active_hash_strength(),
                opts.active_hash_time(),
            );
            let center = pos + offset;
            let scale = Vec3::new(w.max(0.5), h.max(0.5), base_height.max(0.5));
            let half = scale * 0.5;
            let min = center - half;
            let max = center + half;

            if let Some(t) = ray_aabb_intersect(ray_origin, ray_dir, min, max) {
                let closer = hit.as_ref().is_none_or(|h| t < h.t);
                if closer {
                    *hit = Some(CpuPickHit {
                        path: node.path.clone(),
                        t,
                    });
                }
            }
        } else {
            let my_hash = treemap::path_hash(&node.name, dir_hash);
            for child in &node.children {
                self.pick_recursive(
                    child,
                    depth + 1,
                    my_hash,
                    opts,
                    _treemap_opts,
                    world_center,
                    ray_origin,
                    ray_dir,
                    hit,
                );
            }
        }
    }

    // ========================================================================
    // Uniform updates
    // ========================================================================
}

#[cfg(test)]
mod tests {
    //! Tests for the pure `ray_aabb_intersect` function. The full
    //! `cpu_pick` method requires a constructed `Renderer3D` (which
    //! requires a wgpu Device) so it's not unit-testable in isolation;
    //! the picking algorithm's correctness reduces to ray-AABB
    //! intersection correctness, which is what these tests pin down.
    use super::*;

    #[test]
    fn hit_in_front() {
        // Ray from origin pointing +Z, AABB at z=5..6.
        let t = ray_aabb_intersect(
            Vec3::ZERO,
            Vec3::new(0.0, 0.0, 1.0),
            Vec3::new(-1.0, -1.0, 5.0),
            Vec3::new(1.0, 1.0, 6.0),
        );
        assert!(t.is_some(), "expected hit");
        assert!((t.unwrap() - 5.0).abs() < 1e-5, "t = {t:?}");
    }

    #[test]
    fn miss_when_ray_misses_aabb() {
        // Ray going +Z, AABB offset to the side along X.
        let t = ray_aabb_intersect(
            Vec3::ZERO,
            Vec3::new(0.0, 0.0, 1.0),
            Vec3::new(10.0, -1.0, 5.0),
            Vec3::new(11.0, 1.0, 6.0),
        );
        assert!(t.is_none(), "expected miss, got {t:?}");
    }

    #[test]
    fn miss_when_ray_points_away() {
        // Ray from origin pointing -Z; AABB is at +Z.
        let t = ray_aabb_intersect(
            Vec3::ZERO,
            Vec3::new(0.0, 0.0, -1.0),
            Vec3::new(-1.0, -1.0, 5.0),
            Vec3::new(1.0, 1.0, 6.0),
        );
        assert!(t.is_none(), "ray pointing away should miss, got {t:?}");
    }

    #[test]
    fn ray_inside_aabb() {
        // Ray origin inside AABB: standard convention is t = 0 or
        // the entry distance for backward-traced hit. The algorithm
        // returns Some with t computed against the leaving plane.
        let t = ray_aabb_intersect(
            Vec3::new(0.5, 0.5, 0.5),
            Vec3::new(1.0, 0.0, 0.0),
            Vec3::ZERO,
            Vec3::ONE,
        );
        assert!(t.is_some(), "ray inside AABB should hit (degenerate or boundary)");
    }

    #[test]
    fn diagonal_hit() {
        // 45° ray from origin into a corner of an AABB at (5,5,5)..(6,6,6).
        let dir = Vec3::new(1.0, 1.0, 1.0).normalize();
        let t = ray_aabb_intersect(
            Vec3::ZERO,
            dir,
            Vec3::new(5.0, 5.0, 5.0),
            Vec3::new(6.0, 6.0, 6.0),
        );
        assert!(t.is_some(), "diagonal ray to far corner should hit");
        // Distance from origin to nearest corner = sqrt(75) ≈ 8.660
        let expected = (5.0_f32 * 5.0 + 5.0 * 5.0 + 5.0 * 5.0).sqrt();
        assert!(
            (t.unwrap() - expected).abs() < 1e-3,
            "t = {:?}, expected ~{}",
            t,
            expected
        );
    }

    #[test]
    fn negative_direction_component_hits() {
        // Ray from +X side back toward origin AABB.
        let t = ray_aabb_intersect(
            Vec3::new(10.0, 0.0, 0.0),
            Vec3::new(-1.0, 0.0, 0.0),
            Vec3::new(-1.0, -1.0, -1.0),
            Vec3::new(1.0, 1.0, 1.0),
        );
        assert!(t.is_some());
        assert!((t.unwrap() - 9.0).abs() < 1e-5, "t = {t:?}");
    }

    #[test]
    fn parallel_ray_misses_when_offset() {
        // Ray parallel to X axis, but Y is offset above the AABB top.
        let t = ray_aabb_intersect(
            Vec3::new(0.0, 5.0, 0.5),
            Vec3::new(1.0, 0.0, 0.0),
            Vec3::ZERO,
            Vec3::ONE,
        );
        assert!(t.is_none(), "parallel ray above AABB should miss, got {t:?}");
    }

    #[test]
    fn unit_aabb_axis_aligned_hit() {
        // Closest face hit: AABB centered at +Z=10, ray straight ahead.
        let t = ray_aabb_intersect(
            Vec3::ZERO,
            Vec3::new(0.0, 0.0, 1.0),
            Vec3::new(-0.5, -0.5, 9.5),
            Vec3::new(0.5, 0.5, 10.5),
        );
        assert!(t.is_some());
        assert!((t.unwrap() - 9.5).abs() < 1e-5);
    }
}
