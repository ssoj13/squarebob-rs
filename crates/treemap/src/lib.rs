/// Squarified treemap layout + cushion shading renderer.
/// Ported from WinDirStat's TreeMap.cpp with parallel rendering.
use dirstat_core::DirEntry;
use log::trace;
use rayon::prelude::*;

#[cfg(feature = "wgpu")]
pub mod wgpu;

#[cfg(feature = "wgpu")]
pub use wgpu::GpuRenderer2D;

/// Treemap rendering options (mirrors WinDirStat's CTreeMap::Options)
#[derive(Debug, Clone)]
pub struct TreeMapOptions {
    pub style: LayoutStyle,
    pub grid: bool,
    pub grid_color: [u8; 3],
    pub brightness: f64,    // 0..1.0 (default 0.88)
    pub height: f64,        // >= 0.0 (default 0.38) - cushion height factor H
    pub scale_factor: f64,  // 0..1.0 (default 0.91) - scale factor F
    pub ambient_light: f64, // 0..1.0 (default 0.13) - ambient Ia
    pub light_x: f64,       // -4..4 (default -1.0) - light source X
    pub light_y: f64,       // -4..4 (default -1.0) - light source Y
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum LayoutStyle {
    KDirStat,    // children laid out in rows
    SequoiaView, // classical squarification
}

impl Default for TreeMapOptions {
    fn default() -> Self {
        Self {
            style: LayoutStyle::KDirStat,
            grid: false,
            grid_color: [0, 0, 0],
            brightness: 0.88,
            height: 0.38,
            scale_factor: 0.91,
            ambient_light: 0.13,
            light_x: -1.0,
            light_y: -1.0,
        }
    }
}

const PALETTE_BRIGHTNESS: f64 = 0.6;

/// Default 18-color palette from WinDirStat
pub const DEFAULT_PALETTE: [[u8; 3]; 18] = [
    [0, 0, 255],     // Blue
    [255, 0, 0],     // Red
    [0, 255, 0],     // Green
    [255, 255, 0],   // Yellow
    [0, 255, 255],   // Cyan
    [255, 0, 255],   // Magenta
    [255, 170, 0],   // Orange
    [0, 85, 255],    // Dodger Blue
    [255, 0, 85],    // Hot Pink
    [85, 255, 0],    // Lime Green
    [170, 0, 255],   // Violet
    [0, 255, 85],    // Spring Green
    [255, 0, 170],   // Deep Pink
    [0, 170, 255],   // Sky Blue
    [255, 85, 0],    // Orange Red
    [0, 255, 170],   // Aquamarine
    [85, 0, 255],    // Indigo
    [255, 255, 255], // White
];

/// Get color for a file extension (hash-based palette index)
pub fn ext_color(ext: &str) -> [u8; 3] {
    if ext.is_empty() {
        return make_bright([128, 128, 128], PALETTE_BRIGHTNESS);
    }
    // Special gray color for free space indicator
    if ext == "__free__" {
        return [80, 80, 80]; // Dark gray, no brightness adjustment
    }
    // Special gray color for excluded items
    if ext == "__excluded__" {
        return [60, 60, 60]; // Darker gray for excluded
    }
    if ext.eq_ignore_ascii_case("mb") {
        return make_bright([200, 150, 60], PALETTE_BRIGHTNESS);
    }
    if ext.eq_ignore_ascii_case("hou") {
        return make_bright([220, 110, 30], PALETTE_BRIGHTNESS);
    }
    if ext.eq_ignore_ascii_case("exr") {
        return make_bright([50, 180, 210], PALETTE_BRIGHTNESS);
    }
    if ext.eq_ignore_ascii_case("tif") || ext.eq_ignore_ascii_case("tiff") {
        return make_bright([180, 80, 170], PALETTE_BRIGHTNESS);
    }
    if ext.eq_ignore_ascii_case("dpx") {
        return make_bright([150, 110, 230], PALETTE_BRIGHTNESS);
    }
    if ext.eq_ignore_ascii_case("raf") {
        return make_bright([110, 170, 80], PALETTE_BRIGHTNESS);
    }
    if ext.eq_ignore_ascii_case("nef") {
        return make_bright([80, 130, 210], PALETTE_BRIGHTNESS);
    }
    let hash = ext
        .bytes()
        .fold(0u32, |acc, b| acc.wrapping_mul(31).wrapping_add(b as u32));
    let idx = (hash as usize) % DEFAULT_PALETTE.len();
    make_bright(DEFAULT_PALETTE[idx], PALETTE_BRIGHTNESS)
}

/// Normalized light direction vector (precomputed)
struct LightVec {
    lx: f64,
    ly: f64,
    lz: f64,
}

impl LightVec {
    fn new(opts: &TreeMapOptions) -> Self {
        let lx = opts.light_x;
        let ly = opts.light_y;
        let lz = 10.0;
        let len = (lx * lx + ly * ly + lz * lz).sqrt();
        Self {
            lx: lx / len,
            ly: ly / len,
            lz: lz / len,
        }
    }
}

/// Assign layout rectangles to all nodes via Cell (no &mut needed)
pub fn layout(root: &DirEntry, x: f32, y: f32, w: f32, h: f32, opts: &TreeMapOptions) {
    root.rect.set([x, y, w, h]);
    if root.size == 0 || !root.is_dir || root.children.is_empty() {
        return;
    }

    match opts.style {
        LayoutStyle::KDirStat => layout_kdirstat(root, opts),
        LayoutStyle::SequoiaView => layout_sequoia(root, opts),
    }
}

/// KDirStat-style row layout
fn layout_kdirstat(parent: &DirEntry, opts: &TreeMapOptions) {
    let [px, py, pw, ph] = parent.rect.get();
    if pw <= 0.0 || ph <= 0.0 {
        return;
    }

    // KDirStat splits the parent rect along its long axis into full-length rows.
    // Two failure modes produce slivers:
    //   (a) the parent itself is already a sliver — every row inherits the bad aspect.
    //   (b) tiny dominant child within a normal parent — full-pw row × tiny row_h
    //       gives the child a horizontal sliver.
    // Squarified layout solves both because rows occupy only the length they need.
    // Pre-check covers (a). For (b) we run KDirStat then post-check children, and
    // re-layout via squarify if any child is too thin.
    const SLIVER_ASPECT: f32 = 4.0;
    let long = pw.max(ph);
    let short = pw.min(ph);
    if short > 0.0 && long / short > SLIVER_ASPECT {
        layout_sequoia(parent, opts);
        return;
    }

    let horizontal = pw >= ph;
    let total_size = parent.size as f64;
    if total_size <= 0.0 {
        return;
    }

    let width_ratio = if horizontal {
        pw as f64 / ph as f64
    } else {
        ph as f64 / pw as f64
    };

    let grid = if opts.grid { 1.0_f32 } else { 0.0 };
    let n = parent.children.len();

    // Calculate rows using KDirStat's min-proportion algorithm.
    // Pre-detect the (b) failure mode here: if any row would assign the dominant
    // child an unreasonable aspect, fall back to squarify before laying out rows.
    let mut rows: Vec<(f64, usize, usize)> = Vec::new();
    let mut next = 0;

    while next < n {
        let (row_h, used) = calc_next_row(&parent.children, next, total_size, width_ratio);
        rows.push((row_h, next, next + used));
        next += used;
    }

    // Predict each row's worst child aspect; if any exceeds the threshold, squarify.
    // row_w_px is full long-side. dominant child width ≈ (cs/row_size) * row_w_px,
    // height = row_h_normalized * short_side. aspect = wide/tall.
    let long_px = if horizontal { pw } else { ph } as f64;
    let short_px = if horizontal { ph } else { pw } as f64;
    for (row_h, start, end) in &rows {
        let row_size: f64 = parent.children[*start..*end]
            .iter()
            .map(|c| c.size as f64)
            .sum();
        if row_size <= 0.0 || *row_h <= 0.0 {
            continue;
        }
        let row_h_px = row_h * short_px;
        if row_h_px <= 0.0 {
            continue;
        }
        // Worst-case is the largest child in the row.
        let max_child = parent.children[*start..*end]
            .iter()
            .map(|c| c.size as f64)
            .fold(0.0_f64, f64::max);
        let child_w_px = (max_child / row_size) * long_px;
        let aspect = (child_w_px / row_h_px).max(row_h_px / child_w_px.max(1e-6));
        if aspect > SLIVER_ASPECT as f64 {
            layout_sequoia(parent, opts);
            return;
        }
    }

    // Assign rectangles
    let mut top = if horizontal { py } else { px };

    for (row_h, start, end) in &rows {
        let row_h_px = (*row_h as f32) * (if horizontal { ph } else { pw });
        let bottom = top + row_h_px;

        let mut left = if horizontal { px } else { py };
        let row_size: f64 = parent.children[*start..*end]
            .iter()
            .map(|c| c.size as f64)
            .sum();

        for i in *start..*end {
            let child_frac = if row_size > 0.0 {
                parent.children[i].size as f64 / row_size
            } else {
                1.0 / (*end - *start) as f64
            };

            let child_w = child_frac as f32 * (if horizontal { pw } else { ph });
            let right = if i == end - 1 {
                if horizontal {
                    px + pw
                } else {
                    py + ph
                }
            } else {
                left + child_w
            };

            let (cx, cy, cw, ch) = if horizontal {
                (
                    left + grid,
                    top + grid,
                    (right - left - grid).max(0.0),
                    (row_h_px - grid).max(0.0),
                )
            } else {
                (
                    top + grid,
                    left + grid,
                    (row_h_px - grid).max(0.0),
                    (right - left - grid).max(0.0),
                )
            };

            layout(&parent.children[i], cx, cy, cw, ch, opts);
            left = right;
        }
        top = bottom;
    }
}

/// Calculate next row for KDirStat layout
fn calc_next_row(children: &[DirEntry], start: usize, total_size: f64, width: f64) -> (f64, usize) {
    const MIN_PROPORTION: f64 = 0.4;
    let n = children.len();
    let mut size_used: u64 = 0;
    let mut row_height = 0.0;
    let mut i = start;

    while i < n {
        let child_size = children[i].size;
        if child_size == 0 {
            break;
        }
        size_used += child_size;
        let virtual_row_h = size_used as f64 / total_size;
        let child_w = (child_size as f64 / total_size) * width / virtual_row_h;

        if child_w / virtual_row_h < MIN_PROPORTION && i > start {
            break;
        }
        row_height = virtual_row_h;
        i += 1;
    }

    // Include trailing zero-size children
    while i < n && children[i].size == 0 {
        i += 1;
    }

    let used = if i > start { i - start } else { 1 };
    (row_height, used)
}

/// SequoiaView-style classical squarification
fn layout_sequoia(parent: &DirEntry, opts: &TreeMapOptions) {
    let [px, py, pw, ph] = parent.rect.get();
    if pw <= 0.0 || ph <= 0.0 {
        return;
    }

    let grid = if opts.grid { 1.0_f32 } else { 0.0 };
    let mut remaining = [px, py, pw, ph];
    let mut remaining_size = parent.size as f64;
    let total_area = pw as f64 * ph as f64;
    let size_per_pixel = remaining_size / total_area;

    let n = parent.children.len();
    let mut head = 0;

    while head < n && remaining[2] > 0.0 && remaining[3] > 0.0 {
        let [rx, ry, rw, rh] = remaining;
        let horizontal = rw >= rh;
        let side = if horizontal { rh } else { rw };
        let hh = (side as f64) * (side as f64) * size_per_pixel;
        if hh <= 0.0 {
            break;
        }

        // Find best row
        let mut row_end = head;
        let mut worst = f64::MAX;
        let rmax = parent.children[head].size as f64;
        let mut sum = 0.0_f64;

        while row_end < n {
            let cs = parent.children[row_end].size as f64;
            if cs <= 0.0 {
                row_end = n;
                break;
            }
            let rmin = cs;
            let new_sum = sum + rmin;
            let ss = new_sum * new_sum;
            let r1 = hh * rmax / ss;
            let r2 = ss / hh / rmin;
            let next_worst = r1.max(r2);
            if next_worst > worst {
                break;
            }
            sum = new_sum;
            row_end += 1;
            worst = next_worst;
        }

        if sum <= 0.0 {
            break;
        }

        // Row width in pixels
        let row_width = if sum < remaining_size {
            ((sum / remaining_size) * (if horizontal { rw } else { rh }) as f64) as f32
        } else if horizontal {
            rw
        } else {
            rh
        };

        // Distribute children in row
        let mut pos = if horizontal { ry } else { rx };
        let row_len = if horizontal { rh } else { rw };

        for i in head..row_end.min(n) {
            let cs = parent.children[i].size as f64;
            let frac = cs / sum;
            let child_len = frac as f32 * row_len;
            let end = if i == row_end - 1 {
                if horizontal {
                    ry + rh
                } else {
                    rx + rw
                }
            } else {
                pos + child_len
            };

            let (cx, cy, cw, ch) = if horizontal {
                (
                    rx + grid,
                    pos + grid,
                    (row_width - grid).max(0.0),
                    (end - pos - grid).max(0.0),
                )
            } else {
                (
                    pos + grid,
                    ry + grid,
                    (end - pos - grid).max(0.0),
                    (row_width - grid).max(0.0),
                )
            };

            layout(&parent.children[i], cx, cy, cw, ch, opts);
            pos = end;
        }

        // Shrink remaining
        if horizontal {
            remaining[0] += row_width;
            remaining[2] -= row_width;
        } else {
            remaining[1] += row_width;
            remaining[3] -= row_width;
        }
        remaining_size -= sum;
        head = row_end;
    }
}

/// A renderable leaf rectangle with all computed properties
#[derive(Clone)]
struct RenderRect {
    lx: usize,
    ly: usize,
    rx: usize,
    ry: usize,
    color: [u8; 3],
    surface: [f64; 4],
}

/// Pairwise disjoint check used by `debug_assert!` in `render`. O(n²); only
/// runs in debug builds (release optimises the dead branch away). Two rects
/// overlap iff both x and y ranges overlap.
#[allow(dead_code)]
fn rects_disjoint(rects: &[RenderRect]) -> bool {
    for i in 0..rects.len() {
        for j in (i + 1)..rects.len() {
            let a = &rects[i];
            let b = &rects[j];
            let x_overlap = a.lx < b.rx && b.lx < a.rx;
            let y_overlap = a.ly < b.ry && b.ly < a.ry;
            if x_overlap && y_overlap {
                return false;
            }
        }
    }
    true
}

/// Render the treemap into an RGBA pixel buffer (parallel version)
pub fn render(root: &DirEntry, width: u32, height: u32, opts: &TreeMapOptions) -> Vec<u8> {
    let w = width as usize;
    let h = height as usize;
    let mut buf = vec![0u8; w * h * 4]; // RGBA

    // Fill background with grid color (parallel)
    let bg = [
        opts.grid_color[0],
        opts.grid_color[1],
        opts.grid_color[2],
        255u8,
    ];
    buf.par_chunks_exact_mut(4).for_each(|pixel| {
        pixel.copy_from_slice(&bg);
    });

    let light = LightVec::new(opts);
    let cushion = is_cushion(opts);
    let grid_w = if opts.grid { 1.0 } else { 0.0 };

    trace!("Rendering treemap {}x{}, cushion={cushion}", width, height);

    // Collect all leaf rectangles
    let mut rects = Vec::with_capacity(1024);
    collect_rects(
        root,
        &mut rects,
        w,
        h,
        opts,
        grid_w,
        [0.0; 4],
        opts.height,
        true,
        0,
    );

    // Layout invariant: rects must not overlap. The solid-mode parallel
    // path writes through a raw pointer relying on this; the cushion-mode
    // row-parallel path also assumes per-rect ranges are independent.
    debug_assert!(
        rects_disjoint(&rects),
        "treemap layout produced overlapping rects — layout regression?"
    );

    // Render based on mode
    if cushion {
        // For cushion: render row by row in parallel for better cache locality
        let brightness = opts.brightness;
        let ambient = opts.ambient_light;

        buf.par_chunks_exact_mut(w * 4)
            .enumerate()
            .for_each(|(y, row)| {
                for rect in &rects {
                    if y >= rect.ly && y < rect.ry {
                        render_cushion_row(row, y, rect, &light, brightness, ambient);
                    }
                }
            });
    } else {
        // For solid: simple parallel fill per rect
        // Since rects don't overlap, we can safely write in parallel
        let brightness = opts.brightness;
        rects.par_iter().for_each(|rect| {
            let factor = brightness / PALETTE_BRIGHTNESS;
            let (r, g, b) = normalize_color(
                (rect.color[0] as f64 * factor) as i32,
                (rect.color[1] as f64 * factor) as i32,
                (rect.color[2] as f64 * factor) as i32,
            );
            // Note: We need unsafe for parallel writes to non-overlapping regions
            // This is safe because rects don't overlap
            unsafe {
                let buf_ptr = buf.as_ptr() as *mut u8;
                for iy in rect.ly..rect.ry {
                    for ix in rect.lx..rect.rx {
                        let idx = (iy * w + ix) * 4;
                        *buf_ptr.add(idx) = r;
                        *buf_ptr.add(idx + 1) = g;
                        *buf_ptr.add(idx + 2) = b;
                        *buf_ptr.add(idx + 3) = 255;
                    }
                }
            }
        });
    }

    buf
}

/// Render one row of a cushion-shaded rectangle
fn render_cushion_row(
    row: &mut [u8],
    y: usize,
    rect: &RenderRect,
    light: &LightVec,
    brightness: f64,
    ambient: f64,
) {
    let is = 1.0 - ambient;
    let cr = rect.color[0] as f64;
    let cg = rect.color[1] as f64;
    let cb = rect.color[2] as f64;
    let surface = &rect.surface;

    for ix in rect.lx..rect.rx {
        let nx = -(2.0 * surface[0] * (ix as f64 + 0.5) + surface[2]);
        let ny = -(2.0 * surface[1] * (y as f64 + 0.5) + surface[3]);
        let len_sq = nx * nx + ny * ny + 1.0;
        let cosa = (nx * light.lx + ny * light.ly + light.lz) / len_sq.sqrt();
        let cosa = cosa.min(1.0);
        let pixel = ((is * cosa).max(0.0) + ambient) * brightness / PALETTE_BRIGHTNESS;

        let (r, g, b) = normalize_color(
            (cr * pixel) as i32,
            (cg * pixel) as i32,
            (cb * pixel) as i32,
        );

        let idx = ix * 4;
        if idx + 3 < row.len() {
            row[idx] = r;
            row[idx + 1] = g;
            row[idx + 2] = b;
            row[idx + 3] = 255;
        }
    }
}

/// Collect all leaf rectangles for parallel rendering
#[allow(clippy::too_many_arguments)]
fn collect_rects(
    node: &DirEntry,
    rects: &mut Vec<RenderRect>,
    bw: usize,
    bh: usize,
    opts: &TreeMapOptions,
    grid_w: f32,
    surface: [f64; 4],
    h: f64,
    is_root: bool,
    dir_hash: u32,
) {
    let [x, y, w, h_px] = node.rect.get();
    if w <= 0.0 || h_px <= 0.0 {
        return;
    }

    let cushion = is_cushion(opts);

    // Add ridge for cushion (not for root)
    let surface = if cushion && !is_root {
        add_ridge(x, y, w, h_px, surface, h)
    } else {
        surface
    };

    if !node.is_dir || node.children.is_empty() {
        // Leaf: add to render list
        let color = dir_tinted_color(&node.ext, dir_hash);
        let lx = (x + grid_w).max(x) as usize;
        let ly = (y + grid_w).max(y) as usize;
        let rx = ((x + w) as usize).min(bw);
        let ry = ((y + h_px) as usize).min(bh);

        if lx < rx && ly < ry {
            rects.push(RenderRect {
                lx,
                ly,
                rx,
                ry,
                color,
                surface,
            });
        }
    } else {
        // Directory: recurse
        let my_hash = path_hash(&node.name, dir_hash);
        let next_h = h * opts.scale_factor;
        for child in &node.children {
            collect_rects(
                child, rects, bw, bh, opts, grid_w, surface, next_h, false, my_hash,
            );
        }
    }
}

fn is_cushion(opts: &TreeMapOptions) -> bool {
    opts.ambient_light < 1.0 && opts.height > 0.0 && opts.scale_factor > 0.0
}

/// Incremental path hash: mix parent hash with directory name
pub fn path_hash(name: &str, parent_hash: u32) -> u32 {
    name.bytes()
        .fold(parent_hash.wrapping_mul(31).wrapping_add(17), |acc, b| {
            acc.wrapping_mul(31).wrapping_add(b as u32)
        })
}

/// Apply a subtle hue shift to ext_color based on parent dir hash.
/// ~12% blend toward a hash-derived tint so sibling files look grouped.
pub fn dir_tinted_color(ext: &str, dir_hash: u32) -> [u8; 3] {
    let base = ext_color(ext);
    // Don't tint special items - keep them neutral gray
    if ext == "__free__" || ext == "__excluded__" {
        return base;
    }
    if dir_hash == 0 {
        return base;
    }
    // Derive tint hue from dir_hash (spread across color wheel)
    let hue = (dir_hash % 360) as f64;
    let (tr, tg, tb) = hue_to_rgb(hue);
    // Blend 12% tint
    const MIX: f64 = 0.12;
    let r = base[0] as f64 * (1.0 - MIX) + tr * 255.0 * MIX;
    let g = base[1] as f64 * (1.0 - MIX) + tg * 255.0 * MIX;
    let b = base[2] as f64 * (1.0 - MIX) + tb * 255.0 * MIX;
    [
        r.clamp(0.0, 255.0) as u8,
        g.clamp(0.0, 255.0) as u8,
        b.clamp(0.0, 255.0) as u8,
    ]
}

/// Convert hue (0-360) to RGB with full saturation and brightness
fn hue_to_rgb(h: f64) -> (f64, f64, f64) {
    let h = h / 60.0;
    let x = 1.0 - (h % 2.0 - 1.0).abs();
    match h as u32 {
        0 => (1.0, x, 0.0),
        1 => (x, 1.0, 0.0),
        2 => (0.0, 1.0, x),
        3 => (0.0, x, 1.0),
        4 => (x, 0.0, 1.0),
        _ => (1.0, 0.0, x),
    }
}

/// Add a parabolic ridge to the cushion surface
fn add_ridge(x: f32, y: f32, w: f32, h: f32, mut s: [f64; 4], height: f64) -> [f64; 4] {
    let w = w as f64;
    let h = h as f64;
    if w <= 0.0 || h <= 0.0 {
        return s;
    }

    let h4 = 4.0 * height;
    let wf = h4 / w;
    s[2] += wf * ((x as f64 + w) + x as f64);
    s[0] -= wf;

    let hf = h4 / h;
    s[3] += hf * ((y as f64 + h) + y as f64);
    s[1] -= hf;
    s
}

/// Make a color have a specific brightness (port of CColorSpace::MakeBrightColor)
fn make_bright(color: [u8; 3], brightness: f64) -> [u8; 3] {
    let r = color[0] as f64 / 255.0;
    let g = color[1] as f64 / 255.0;
    let b = color[2] as f64 / 255.0;
    let sum = r + g + b;
    if sum <= 0.0 {
        let v = (brightness * 255.0) as u8;
        return [v, v, v];
    }
    let f = 3.0 * brightness / sum;
    let (rn, gn, bn) = normalize_color(
        (r * f * 255.0) as i32,
        (g * f * 255.0) as i32,
        (b * f * 255.0) as i32,
    );
    [rn, gn, bn]
}

/// Clamp and redistribute overflow (port of CColorSpace::NormalizeColor)
fn normalize_color(mut r: i32, mut g: i32, mut b: i32) -> (u8, u8, u8) {
    if r > 255 {
        let h = (r - 255) / 2;
        r = 255;
        g += h;
        b += h;
    }
    if g > 255 {
        let h = (g - 255) / 2;
        g = 255;
        r = r.min(255);
        b += h;
    }
    if b > 255 {
        b = 255;
        r = r.min(255);
        g = g.min(255);
    }
    (
        r.clamp(0, 255) as u8,
        g.clamp(0, 255) as u8,
        b.clamp(0, 255) as u8,
    )
}

/// Minimum rectangle size before GPU renderers consolidate into a single rect
pub const MIN_RECT_SIZE: f32 = 3.0;

/// Add a parabolic ridge to the cushion surface (f32 version for GPU renderers)
pub fn add_ridge_f32(x: f32, y: f32, w: f32, h: f32, mut s: [f32; 4], height: f64) -> [f32; 4] {
    let w = w as f64;
    let h = h as f64;
    if w <= 0.0 || h <= 0.0 {
        return s;
    }
    let h4 = 4.0 * height;
    let wf = h4 / w;
    s[2] += (wf * ((x as f64 + w) + x as f64)) as f32;
    s[0] -= wf as f32;
    let hf = h4 / h;
    s[3] += (hf * ((y as f64 + h) + y as f64)) as f32;
    s[1] -= hf as f32;
    s
}

/// Compute size-weighted average color for a directory's descendants (with dir tinting)
pub fn compute_avg_color(node: &DirEntry, dir_hash: u32) -> [u8; 3] {
    let mut total_size: u64 = 0;
    let mut r_sum: u64 = 0;
    let mut g_sum: u64 = 0;
    let mut b_sum: u64 = 0;
    accumulate_colors(
        node,
        dir_hash,
        &mut total_size,
        &mut r_sum,
        &mut g_sum,
        &mut b_sum,
    );
    if total_size == 0 {
        return [128, 128, 128];
    }
    [
        (r_sum / total_size).min(255) as u8,
        (g_sum / total_size).min(255) as u8,
        (b_sum / total_size).min(255) as u8,
    ]
}

fn accumulate_colors(
    node: &DirEntry,
    dir_hash: u32,
    total_size: &mut u64,
    r_sum: &mut u64,
    g_sum: &mut u64,
    b_sum: &mut u64,
) {
    if !node.is_dir || node.children.is_empty() {
        let color = dir_tinted_color(&node.ext, dir_hash);
        let size = node.size.max(1);
        *total_size += size;
        *r_sum += color[0] as u64 * size;
        *g_sum += color[1] as u64 * size;
        *b_sum += color[2] as u64 * size;
    } else {
        let my_hash = path_hash(&node.name, dir_hash);
        for child in &node.children {
            accumulate_colors(child, my_hash, total_size, r_sum, g_sum, b_sum);
        }
    }
}

/// Find the leaf node at pixel position (x, y)
pub fn hit_test(node: &DirEntry, x: f32, y: f32) -> Option<&DirEntry> {
    let [nx, ny, nw, nh] = node.rect.get();
    if x < nx || y < ny || x >= nx + nw || y >= ny + nh {
        return None;
    }

    if !node.is_dir || node.children.is_empty() {
        return Some(node);
    }

    for child in &node.children {
        if let Some(hit) = hit_test(child, x, y) {
            return Some(hit);
        }
    }

    Some(node)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    fn file(name: &str, size: u64) -> DirEntry {
        DirEntry::new_file(
            name.to_string(),
            PathBuf::from(name),
            size,
            "txt".to_string(),
            None,
        )
    }

    fn dir_with(name: &str, mut children: Vec<DirEntry>) -> DirEntry {
        let mut d = DirEntry::new_dir(name.to_string(), PathBuf::from(name));
        for c in &children {
            d.size += c.size;
            d.file_count += c.file_count;
        }
        // layout requires children sorted by size descending
        children.sort_unstable_by_key(|c| std::cmp::Reverse(c.size));
        d.children = children;
        d
    }

    fn rect_area(r: [f32; 4]) -> f32 {
        r[2] * r[3]
    }

    fn rects_overlap(a: [f32; 4], b: [f32; 4]) -> bool {
        let ax2 = a[0] + a[2];
        let ay2 = a[1] + a[3];
        let bx2 = b[0] + b[2];
        let by2 = b[1] + b[3];
        // Overlap iff both x and y ranges overlap (open intervals).
        a[0] < bx2 && b[0] < ax2 && a[1] < by2 && b[1] < ay2
    }

    #[test]
    fn layout_two_equal_children_cover_parent_no_overlap() {
        let root = dir_with("root", vec![file("a", 50), file("b", 50)]);
        let opts = TreeMapOptions::default();
        layout(&root, 0.0, 0.0, 100.0, 100.0, &opts);

        let r0 = root.children[0].rect.get();
        let r1 = root.children[1].rect.get();

        // Children together cover parent area (this is the cube-gaps regression test).
        let parent_area = 100.0 * 100.0;
        let area_sum = rect_area(r0) + rect_area(r1);
        assert!(
            (area_sum - parent_area).abs() < 1.0,
            "child area sum {area_sum} != parent area {parent_area}; rects {r0:?} {r1:?}"
        );

        // No overlap.
        assert!(
            !rects_overlap(r0, r1),
            "two children overlap: {r0:?} vs {r1:?}"
        );
    }

    #[test]
    fn layout_three_unequal_children_sum_to_parent() {
        let root = dir_with(
            "root",
            vec![file("big", 70), file("med", 20), file("small", 10)],
        );
        let opts = TreeMapOptions::default();
        layout(&root, 0.0, 0.0, 200.0, 100.0, &opts);

        let parent_area = 200.0 * 100.0;
        let area_sum: f32 = root.children.iter().map(|c| rect_area(c.rect.get())).sum();
        assert!(
            (area_sum - parent_area).abs() < 1.0,
            "child area sum {area_sum} != parent area {parent_area}"
        );

        // Pairwise no-overlap.
        for i in 0..root.children.len() {
            for j in (i + 1)..root.children.len() {
                let a = root.children[i].rect.get();
                let b = root.children[j].rect.get();
                assert!(!rects_overlap(a, b), "child {i} overlaps child {j}");
            }
        }
    }

    #[test]
    fn layout_empty_dir_keeps_assigned_rect() {
        // size==0 short-circuits in `layout`; rect should still be set.
        let root = DirEntry::new_dir("empty".to_string(), PathBuf::from("empty"));
        let opts = TreeMapOptions::default();
        layout(&root, 5.0, 10.0, 20.0, 30.0, &opts);
        assert_eq!(root.rect.get(), [5.0, 10.0, 20.0, 30.0]);
    }

    #[test]
    fn layout_sequoia_style_also_covers_parent() {
        let root = dir_with(
            "root",
            vec![file("a", 40), file("b", 30), file("c", 20), file("d", 10)],
        );
        let opts = TreeMapOptions {
            style: LayoutStyle::SequoiaView,
            ..TreeMapOptions::default()
        };
        layout(&root, 0.0, 0.0, 300.0, 200.0, &opts);

        let parent_area = 300.0 * 200.0;
        let area_sum: f32 = root.children.iter().map(|c| rect_area(c.rect.get())).sum();
        assert!(
            (area_sum - parent_area).abs() < 2.0,
            "sequoia: child area sum {area_sum} != parent area {parent_area}"
        );
    }

    #[test]
    fn rects_disjoint_helper_detects_overlap() {
        let a = RenderRect {
            lx: 0,
            ly: 0,
            rx: 10,
            ry: 10,
            color: [0; 3],
            surface: [0.0; 4],
        };
        let b = RenderRect {
            lx: 5,
            ly: 5,
            rx: 15,
            ry: 15,
            color: [0; 3],
            surface: [0.0; 4],
        };
        let c = RenderRect {
            lx: 20,
            ly: 20,
            rx: 30,
            ry: 30,
            color: [0; 3],
            surface: [0.0; 4],
        };

        assert!(!rects_disjoint(&[a.clone(), b.clone()]), "a/b overlap");
        assert!(rects_disjoint(&[a, c]), "a/c are disjoint");
    }
}
