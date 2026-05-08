//! SAH-based BVH builder.
//!
//! Constructs a flat BVH array from a list of triangles.
//! Uses Surface Area Heuristic for split decisions and
//! produces a compact node array for GPU upload.

use super::bvh::{Aabb, BvhNode, Instance};

/// Number of SAH bins for split evaluation.
const NUM_BINS: usize = 12;

/// Cost ratio: traversal vs intersection (typical GPU values).
const TRAVERSAL_COST: f32 = 1.0;
const INTERSECT_COST: f32 = 1.0;

/// Maximum triangles per leaf before forcing a split.
const MAX_LEAF_SIZE: usize = 4;

/// Built BVH result.
pub struct Bvh {
    /// Flat node array (index 0 = root).
    pub nodes: Vec<BvhNode>,
    /// Reordered triangle indices (leaves reference into this).
    pub tri_indices: Vec<usize>,
}

/// SAH bin for evaluating split candidates.
struct Bin {
    bounds: Aabb,
    count: usize,
}

impl Bin {
    fn new() -> Self {
        Self {
            bounds: Aabb::EMPTY,
            count: 0,
        }
    }
}

/// Build BVH from instances using SAH.
pub fn build_instance_bvh(instances: &[Instance]) -> Bvh {
    let n = instances.len();
    if n == 0 {
        return Bvh {
            nodes: vec![BvhNode {
                aabb_min: [0.0; 3],
                left_or_first: 0,
                aabb_max: [0.0; 3],
                count: 0,
            }],
            tri_indices: vec![],
        };
    }

    let centroids: Vec<[f32; 3]> = instances.iter().map(|inst| inst.centroid()).collect();
    let aabbs: Vec<Aabb> = instances.iter().map(|inst| inst.aabb).collect();

    let mut indices: Vec<usize> = (0..n).collect();
    let mut nodes: Vec<BvhNode> = Vec::with_capacity(2 * n);
    nodes.push(BvhNode {
        aabb_min: [0.0; 3],
        left_or_first: 0,
        aabb_max: [0.0; 3],
        count: 0,
    });

    struct Task {
        node_idx: usize,
        start: usize,
        end: usize,
    }

    let mut stack = vec![Task {
        node_idx: 0,
        start: 0,
        end: n,
    }];

    while let Some(task) = stack.pop() {
        let start = task.start;
        let end = task.end;
        let count = end - start;

        let mut node_aabb = Aabb::EMPTY;
        for &idx in &indices[start..end] {
            node_aabb.grow(&aabbs[idx]);
        }

        if count <= MAX_LEAF_SIZE {
            nodes[task.node_idx] = BvhNode {
                aabb_min: node_aabb.min,
                left_or_first: start as u32,
                aabb_max: node_aabb.max,
                count: count as u32,
            };
            continue;
        }

        let mut centroid_bounds = Aabb::EMPTY;
        for &idx in &indices[start..end] {
            centroid_bounds.grow_point(centroids[idx]);
        }

        let (best_axis, best_split_pos, best_cost) =
            find_best_split(&indices[start..end], &aabbs, &centroids, &centroid_bounds);

        let parent_area = node_aabb.area();
        let leaf_cost = count as f32 * INTERSECT_COST * parent_area;

        if best_cost >= leaf_cost || best_axis == usize::MAX {
            nodes[task.node_idx] = BvhNode {
                aabb_min: node_aabb.min,
                left_or_first: start as u32,
                aabb_max: node_aabb.max,
                count: count as u32,
            };
            continue;
        }

        let mid = partition(&mut indices[start..end], |&idx| {
            centroids[idx][best_axis] < best_split_pos
        }) + start;

        let mid = if mid == start || mid == end {
            (start + end) / 2
        } else {
            mid
        };

        let left_idx = nodes.len();
        let right_idx = left_idx + 1;
        nodes.push(BvhNode {
            aabb_min: [0.0; 3],
            left_or_first: 0,
            aabb_max: [0.0; 3],
            count: 0,
        });
        nodes.push(BvhNode {
            aabb_min: [0.0; 3],
            left_or_first: 0,
            aabb_max: [0.0; 3],
            count: 0,
        });

        nodes[task.node_idx] = BvhNode {
            aabb_min: node_aabb.min,
            left_or_first: left_idx as u32,
            aabb_max: node_aabb.max,
            count: 0,
        };

        stack.push(Task {
            node_idx: right_idx,
            start: mid,
            end,
        });
        stack.push(Task {
            node_idx: left_idx,
            start,
            end: mid,
        });
    }

    Bvh {
        nodes,
        tri_indices: indices,
    }
}

/// SAH binned split search across all 3 axes.
/// Returns (best_axis, split_position, cost). axis=usize::MAX if no valid split.
fn find_best_split(
    indices: &[usize],
    aabbs: &[Aabb],
    centroids: &[[f32; 3]],
    centroid_bounds: &Aabb,
) -> (usize, f32, f32) {
    let mut best_axis = usize::MAX;
    let mut best_pos = 0.0f32;
    let mut best_cost = f32::INFINITY;

    #[allow(clippy::needless_range_loop)]
    for axis in 0..3 {
        let extent = centroid_bounds.max[axis] - centroid_bounds.min[axis];
        if extent < 1e-8 {
            continue; // degenerate axis
        }

        // Initialize bins
        let mut bins = Vec::with_capacity(NUM_BINS);
        for _ in 0..NUM_BINS {
            bins.push(Bin::new());
        }

        let inv_extent = NUM_BINS as f32 / extent;

        // Assign primitives to bins
        for &idx in indices {
            let bin_id = ((centroids[idx][axis] - centroid_bounds.min[axis]) * inv_extent) as usize;
            let bin_id = bin_id.min(NUM_BINS - 1);
            bins[bin_id].bounds.grow(&aabbs[idx]);
            bins[bin_id].count += 1;
        }

        // Sweep from left: compute prefix areas and counts
        let mut left_area = [0.0f32; NUM_BINS - 1];
        let mut left_count = [0usize; NUM_BINS - 1];
        let mut sweep = Aabb::EMPTY;
        let mut sweep_count = 0;
        for i in 0..NUM_BINS - 1 {
            sweep.grow(&bins[i].bounds);
            sweep_count += bins[i].count;
            left_area[i] = sweep.area();
            left_count[i] = sweep_count;
        }

        // Sweep from right and evaluate SAH cost
        sweep = Aabb::EMPTY;
        sweep_count = 0;
        for i in (1..NUM_BINS).rev() {
            sweep.grow(&bins[i].bounds);
            sweep_count += bins[i].count;
            let cost = TRAVERSAL_COST
                + INTERSECT_COST
                    * (left_count[i - 1] as f32 * left_area[i - 1]
                        + sweep_count as f32 * sweep.area());

            if cost < best_cost {
                best_cost = cost;
                best_axis = axis;
                best_pos = centroid_bounds.min[axis] + (i as f32 / NUM_BINS as f32) * extent;
            }
        }
    }

    (best_axis, best_pos, best_cost)
}

/// Partition slice in-place. Returns count of elements where predicate is true.
fn partition<T, F>(slice: &mut [T], pred: F) -> usize
where
    F: Fn(&T) -> bool,
{
    let mut left = 0;
    let mut right = slice.len();
    while left < right {
        if pred(&slice[left]) {
            left += 1;
        } else {
            right -= 1;
            slice.swap(left, right);
        }
    }
    left
}
