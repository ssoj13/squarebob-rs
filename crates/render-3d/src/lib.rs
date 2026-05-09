//! 3D Treemap Renderer — modular multi-pass PBR pipeline
//!
//! Modules:
//! - geometry: cube mesh, instance data
//! - pipelines: pipeline/layout creation (deduplicated)
//! - targets: render target textures, dynamic bind groups
//! - picking: object ID readback + path mapping
//! - env_map: environment map loading

pub mod env_map;
pub mod geometry;
pub mod picking;
pub mod pipelines;
mod pt;
mod renderer3d;
pub mod targets;

use glam::{Mat4, Vec3, Vec4};
use log::{debug, info, trace, warn};
use std::sync::Arc;
use wgpu::util::DeviceExt;

use dirstat_core::DirEntry;
use pt_core::bvh::GpuMaterial;
use pt_mats::{MaterialLibrary, MaterializeMode};
use render_core::gpu::{self, GpuContext};
use render_shared::{
    hash_transform_offset, CameraUniform, CubeHeightMode,
    EnvParamsUniform, HoverMode, HoverParamsUniform, LightRigUniform,
    OrbitCamera, Render3DOptions,
};
use treemap::{self, TreeMapOptions};

use geometry::{CubeInstance, CUBE_INDICES, NUM_INDICES};
use pipelines::{BindGroupLayouts, Pipelines};
use renderer3d::material_cache::{mat_settings_hash, MatGlobalUniform, MaterialCache};
use targets::{DynamicBindGroups, RenderTargets};

const DEFAULT_SCENE_LAYOUT_SIZE: (u32, u32) = (1024, 1024);

pub(crate) struct PtState {
    pub path_tracer: Option<pt_megakernel::PathTraceCompute>,
    pub pt_backend_kind: pt::PtBackendKind,
    pub pt_scene_dirty: bool,
    pub pt_env_dirty: bool,
    pub pt_samples_per_update: u32,
    pub pt_last_render_ms: f32,
    pub pt_camera_snap_time: std::time::Instant,
    pub pt_snap_inv_view: Mat4,
    pub pt_snap_inv_proj: Mat4,
    pub pt_snap_pos: Vec3,
    pub pt_snap_valid: bool,
    pub pt_snap_anim_time: f32,
    pub pt_last_anim_time: f32,
    pub pt_anim_log_frame: u64,
}

impl Default for PtState {
    fn default() -> Self {
        Self {
            path_tracer: None,
            pt_backend_kind: pt::PtBackendKind::Megakernel,
            pt_scene_dirty: false,
            pt_env_dirty: true,
            pt_samples_per_update: 1,
            pt_last_render_ms: 0.0,
            pt_camera_snap_time: std::time::Instant::now(),
            pt_snap_inv_view: Mat4::IDENTITY,
            pt_snap_inv_proj: Mat4::IDENTITY,
            pt_snap_pos: Vec3::ZERO,
            pt_snap_valid: false,
            pt_snap_anim_time: 0.0,
            pt_last_anim_time: 0.0,
            pt_anim_log_frame: 0,
        }
    }
}

/// 3D Renderer with multi-pass PBR pipeline
pub struct Renderer3D {
    ctx: Arc<GpuContext>,

    // Geometry buffers (static)
    vertex_buffer: wgpu::Buffer,
    index_buffer: wgpu::Buffer,
    instance_buffer: Option<wgpu::Buffer>,
    instance_buffer_capacity: usize, // Max instances buffer can hold
    instance_count: u32,

    // Uniform buffers
    camera_buf: wgpu::Buffer,
    light_rig_buf: wgpu::Buffer,
    /// Storage buffer holding the full `MaterialLibrary` (array of GpuMaterial).
    /// Indexed per cube via `CubeInstance.material_id` in the PBR shader. Owned by
    /// the renderer so the bind group keeps a live GPU handle (dead-code lint can't
    /// see the bind group reference).
    #[allow(dead_code)]
    materials_buf: wgpu::Buffer,
    /// UBO with global mat params (currently `materialize_mix`).
    mat_global_buf: wgpu::Buffer,
    env_params_buf: wgpu::Buffer,
    hover_params_buf: wgpu::Buffer,
    /// Selected IDs buffer for object_id shader (marks selected with SELECTED_BIT)
    selected_ids_buf: wgpu::Buffer,

    // Static bind groups (don't depend on render targets)
    pbr_bg0: wgpu::BindGroup,    // Camera + lights + material
    env_bg: wgpu::BindGroup,     // Env map + sampler + params
    obj_id_bg0: wgpu::BindGroup, // Camera only

    // Dynamic bind groups (recreated on resize or env map change)
    dyn_bgs: Option<DynamicBindGroups>,

    // Layouts (kept for recreating dynamic bind groups)
    layouts: BindGroupLayouts,

    // Pipelines
    pipes: Pipelines,

    // Render targets
    targets: Option<RenderTargets>,

    // Environment map
    pub env: env_map::EnvMap,

    // Object ID picking (async, 1-frame latency)
    pub picking: picking::PickingState,
    mouse_pos: Option<(u32, u32)>,

    // Cached light rig
    light_rig: LightRigUniform,

    // Path tracer state (lazy init)
    pt: PtState,

    // Material presets (for PT materializer)
    material_library: MaterialLibrary,

    // Per-path classification cache; invalidated only on mat-settings change.
    mat_cache: MaterialCache,

    // Instance cache (for static scenes without animation)
    cached_instances: Option<Arc<Vec<geometry::CubeInstance>>>,
    /// Total number of times `collect_cubes` ran since renderer construction.
    /// Used by Stage A.1 verification to confirm that toggling
    /// `materialize_mix` (now a shader-side uniform) does NOT trigger an
    /// instance rebuild — the counter should not advance when only the
    /// mix slider changes between frames. Read via `instance_rebuild_count`.
    cached_instances_rebuild_count: u64,
    cached_opts_hash: u64,
    cached_layout_size: (u32, u32),
    scene_layout_size: Option<(u32, u32)>,

    // Selected object IDs for outline rendering
    selected_ids: std::collections::HashSet<u32>,
}

/// Result of a CPU pick query
pub struct CpuPickHit {
    pub path: std::path::PathBuf,
    pub t: f32,
}

/// Compute slice plane normal from options (either from axis or custom vector)
fn compute_slice_normal(opts: &Render3DOptions) -> [f32; 3] {
    if opts.slice_use_vector {
        opts.slice_normal
    } else {
        match opts.slice_axis {
            0 => [1.0, 0.0, 0.0], // X
            1 => [0.0, 1.0, 0.0], // Y
            _ => [0.0, 0.0, 1.0], // Z
        }
    }
}

fn compute_slice_position(opts: &Render3DOptions) -> f32 {
    if opts.slice_use_vector {
        opts.slice_position_vector
    } else {
        opts.slice_position
    }
}

fn lerp(a: f32, b: f32, t: f32) -> f32 {
    a + (b - a) * t
}

fn lerp4(a: [f32; 4], b: [f32; 4], t: f32) -> [f32; 4] {
    [
        lerp(a[0], b[0], t),
        lerp(a[1], b[1], t),
        lerp(a[2], b[2], t),
        lerp(a[3], b[3], t),
    ]
}

fn hash_f32(hash: u32, salt: u32) -> f32 {
    let h = hash
        .wrapping_mul(1664525)
        .wrapping_add(salt)
        .wrapping_mul(1013904223);
    (h as f32) / (u32::MAX as f32)
}

fn mix_material(base: GpuMaterial, glass: GpuMaterial, t: f32) -> GpuMaterial {
    let t = t.clamp(0.0, 1.0);
    if t <= 0.0 {
        return base;
    }
    if t >= 1.0 {
        let mut out = glass;
        // Preserve emission so lights remain visible even at full transparency.
        for i in 0..4 {
            out.emission_color_weight[i] =
                out.emission_color_weight[i].max(base.emission_color_weight[i]);
        }
        return out;
    }
    let mut out = GpuMaterial {
        base_color_weight: lerp4(base.base_color_weight, glass.base_color_weight, t),
        specular_color_weight: lerp4(base.specular_color_weight, glass.specular_color_weight, t),
        transmission_color_weight: lerp4(
            base.transmission_color_weight,
            glass.transmission_color_weight,
            t,
        ),
        subsurface_color_weight: lerp4(
            base.subsurface_color_weight,
            glass.subsurface_color_weight,
            t,
        ),
        coat_color_weight: lerp4(base.coat_color_weight, glass.coat_color_weight, t),
        emission_color_weight: lerp4(base.emission_color_weight, glass.emission_color_weight, t),
        opacity: lerp4(base.opacity, glass.opacity, t),
        params1: lerp4(base.params1, glass.params1, t),
        params2: lerp4(base.params2, glass.params2, t),
    };
    for i in 0..4 {
        out.emission_color_weight[i] =
            out.emission_color_weight[i].max(base.emission_color_weight[i]);
    }
    out
}

fn kelvin_to_rgb(kelvin: f32) -> [f32; 3] {
    let k = kelvin.clamp(1000.0, 40000.0) / 100.0;
    let (mut r, mut g, mut b);
    if k <= 66.0 {
        r = 255.0;
        g = 99.470_8 * k.ln() - 161.119_57;
        b = if k <= 19.0 {
            0.0
        } else {
            138.517_73 * (k - 10.0).ln() - 305.044_8
        };
    } else {
        r = 329.698_73 * (k - 60.0).powf(-0.133_204_76);
        g = 288.122_16 * (k - 60.0).powf(-0.075_514_846);
        b = 255.0;
    }
    r = r.clamp(0.0, 255.0);
    g = g.clamp(0.0, 255.0);
    b = b.clamp(0.0, 255.0);
    [r / 255.0, g / 255.0, b / 255.0]
}

fn apply_glass_controls(mut glass: GpuMaterial, opts: &Render3DOptions) -> GpuMaterial {
    let spec = opts.pt_glass_specular.clamp(0.0, 1.0);
    let base = opts.pt_glass_base.clamp(0.0, 1.0);
    let rough = opts.pt_glass_roughness.clamp(0.0, 1.0);
    let ior = opts.pt_glass_ior.clamp(1.0, 3.0);
    let dispersion = opts.pt_glass_dispersion.clamp(0.0, 1.0);
    let temp = opts.pt_glass_temp.clamp(1000.0, 12000.0);
    let tint = kelvin_to_rgb(temp);

    glass.base_color_weight[3] *= base;
    glass.specular_color_weight[3] *= spec;
    glass.params1[2] = rough;
    glass.params1[3] = ior;
    glass.params2[0] = dispersion;

    glass.transmission_color_weight[0] *= tint[0];
    glass.transmission_color_weight[1] *= tint[1];
    glass.transmission_color_weight[2] *= tint[2];

    // When global transparency is high, bias glass toward transmission clarity.
    // This keeps it "glassier" without switching to a non-physical ghost mode.
    let clarity = opts.pt_global_transparency.clamp(0.0, 1.0);
    if clarity > 0.0 {
        let spec_scale = 1.0 - 0.6 * clarity;
        let base_scale = 1.0 - 0.9 * clarity;
        glass.specular_color_weight[3] *= spec_scale.max(0.05);
        glass.base_color_weight[3] *= base_scale.max(0.0);
        let r = glass.params1[2];
        glass.params1[2] = (r * (1.0 - 0.7 * clarity)).max(0.0);
    }

    if opts.pt_glass_thin {
        glass.base_color_weight[3] = 0.0;
        glass.opacity = [1.0, 1.0, 1.0, 0.0];
    }

    glass
}

impl Renderer3D {
    /// Invalidate cached instances (forces geometry rebuild on next render)
    pub fn invalidate_instances(&mut self) {
        self.cached_instances = None;
        self.cached_opts_hash = 0;
    }

    /// Reset render targets (call when switching modes)
    pub fn reset_render_targets(&mut self) {
        // Must wait before dropping GPU resources
        let _ = self.ctx.device.poll(wgpu::PollType::wait_indefinitely());
        self.targets = None;
        self.dyn_bgs = None;
        self.instance_buffer = None;
        self.instance_count = 0;
        // Clear instance cache to force rebuild
        self.cached_instances = None;
        self.cached_opts_hash = 0;
        self.cached_layout_size = (0, 0);
        self.scene_layout_size = None;
    }

    /// Drop PT resources and force re-init on next render.
    pub fn reset_path_tracer(&mut self) {
        self.pt.path_tracer = None;
        self.pt.pt_scene_dirty = true;
        self.pt.pt_env_dirty = true;
        self.pt.pt_samples_per_update = 1;
        self.pt.pt_last_render_ms = 0.0;
    }

    pub fn mark_pt_scene_dirty(&mut self) {
        self.pt.pt_scene_dirty = true;
    }

    pub fn mark_pt_env_dirty(&mut self) {
        self.pt.pt_env_dirty = true;
    }

    pub fn reset_pt_accumulation(&mut self) {
        if let Some(pt) = &mut self.pt.path_tracer {
            pt.reset_accumulation();
        }
    }

    /// Create a new 3D renderer
    pub fn new(ctx: Arc<GpuContext>) -> Self {
        let device = &ctx.device;

        // Bind group layouts
        let layouts = BindGroupLayouts::new(device);

        // Pipelines
        let pipes = Pipelines::new(device, &layouts);

        // Uniform buffers
        let camera_buf = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("Camera UBO"),
            size: std::mem::size_of::<CameraUniform>() as u64,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });

        let light_rig = LightRigUniform::default();
        let light_rig_buf = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("LightRig UBO"),
            contents: bytemuck::bytes_of(&light_rig),
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
        });

        // Per-instance material library: storage buffer with full GpuMaterial array.
        // Sized & filled from the in-memory MaterialLibrary used by both PBR and PT.
        let material_library = MaterialLibrary::new();
        let materials_buf = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("Materials Storage"),
            contents: bytemuck::cast_slice(material_library.materials()),
            usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_DST,
        });
        let mat_global_buf = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("MatGlobal UBO"),
            contents: bytemuck::bytes_of(&MatGlobalUniform::default()),
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
        });

        let env_params_buf = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("EnvParams UBO"),
            contents: bytemuck::bytes_of(&EnvParamsUniform::default()),
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
        });

        let hover_params_buf = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("HoverParams UBO"),
            contents: bytemuck::bytes_of(&HoverParamsUniform::default()),
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
        });

        // Storage buffer for selected IDs (4MB = 1 million objects)
        let selected_ids_buf = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("SelectedIds Storage"),
            size: 4 * 1024 * 1024, // 4MB = 1M u32 = count + 1M IDs
            usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });

        // Environment map (default placeholder)
        let env = env_map::EnvMap::new_default(&ctx);

        // Static bind groups
        let pbr_bg0 = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("PBR BG0"),
            layout: &layouts.pbr_group0,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: camera_buf.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: light_rig_buf.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 2,
                    resource: materials_buf.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 3,
                    resource: mat_global_buf.as_entire_binding(),
                },
            ],
        });

        let env_bg = Self::create_env_bind_group(device, &layouts.env, &env, &env_params_buf);

        let obj_id_bg0 = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("ObjID BG0"),
            layout: &layouts.object_id_group0,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: camera_buf.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: selected_ids_buf.as_entire_binding(),
                },
            ],
        });

        // Geometry
        let vertex_buffer = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("Cube VBO"),
            contents: bytemuck::cast_slice(geometry::CUBE_VERTICES),
            usage: wgpu::BufferUsages::VERTEX,
        });
        let index_buffer = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("Cube IBO"),
            contents: bytemuck::cast_slice(CUBE_INDICES),
            usage: wgpu::BufferUsages::INDEX,
        });

        Self {
            ctx,
            vertex_buffer,
            index_buffer,
            instance_buffer: None,
            instance_buffer_capacity: 0,
            instance_count: 0,
            camera_buf,
            light_rig_buf,
            materials_buf,
            mat_global_buf,
            env_params_buf,
            hover_params_buf,
            selected_ids_buf,
            pbr_bg0,
            env_bg,
            obj_id_bg0,
            dyn_bgs: None,
            layouts,
            pipes,
            targets: None,
            env,
            picking: picking::PickingState::new(),
            mouse_pos: None,
            light_rig,
            pt: PtState::default(),
            material_library,
            mat_cache: MaterialCache::default(),
            cached_instances: None,
            cached_instances_rebuild_count: 0,
            cached_opts_hash: 0,
            cached_layout_size: (0, 0),
            scene_layout_size: None,
            selected_ids: std::collections::HashSet::new(),
        }
    }

    fn scene_layout_size(&mut self) -> (u32, u32) {
        *self
            .scene_layout_size
            .get_or_insert(DEFAULT_SCENE_LAYOUT_SIZE)
    }

    /// Current logical 3D scene size. This is intentionally separate from the render target size.
    pub fn current_scene_layout_size(&self) -> (u32, u32) {
        self.scene_layout_size.unwrap_or(DEFAULT_SCENE_LAYOUT_SIZE)
    }

    /// Compute a hash of options that affect cube geometry
    fn opts_hash(
        opts: &Render3DOptions,
        w: u32,
        h: u32,
        camera: &OrbitCamera,
        screen_height: u32,
    ) -> u64 {
        use std::hash::{Hash, Hasher};
        let mut hasher = std::collections::hash_map::DefaultHasher::new();
        // Include all options that affect cube generation
        (opts.height_mode as u8).hash(&mut hasher);
        opts.height_power_enabled.hash(&mut hasher);
        opts.height_power.to_bits().hash(&mut hasher);
        opts.height_scale.to_bits().hash(&mut hasher);
        (opts.color_mode as u8).hash(&mut hasher);
        (opts.folder_color_mode as u8).hash(&mut hasher);
        opts.folder_tint.to_bits().hash(&mut hasher);
        (opts.hash_effect as u8).hash(&mut hasher);
        opts.hash_effect_strength.to_bits().hash(&mut hasher);
        // Only include animation_time if animated
        if opts.animate {
            opts.animation_time.to_bits().hash(&mut hasher);
        }
        opts.animate.hash(&mut hasher);
        // LOD settings
        opts.lod_enabled.hash(&mut hasher);
        if opts.lod_enabled {
            opts.lod_min_screen_size.to_bits().hash(&mut hasher);
            screen_height.hash(&mut hasher);
            camera.fov.to_bits().hash(&mut hasher);
            let p = camera.position();
            // Camera LOD changes with view position. Quantize to avoid rebuilding on sub-pixel jitter.
            ((p.x * 4.0).round() as i32).hash(&mut hasher);
            ((p.y * 4.0).round() as i32).hash(&mut hasher);
            ((p.z * 4.0).round() as i32).hash(&mut hasher);
        }
        w.hash(&mut hasher);
        h.hash(&mut hasher);
        // Material settings (mode/seed/lights/glass/etc.) need to bake into the instance
        // hash because they change per-cube `material_id`. `materialize_mix` is excluded
        // from this hash (handled by shader uniform), so the slider stays live.
        mat_settings_hash(opts).hash(&mut hasher);
        hasher.finish()
    }

    // ========================================================================
    // Zero-copy rendering API
    // ========================================================================

    /// Get render texture for egui registration (zero-copy path)
    pub fn get_render_texture(&self) -> Option<&wgpu::Texture> {
        self.targets.as_ref().map(|t| &t.render_texture)
    }

    /// Render to GPU texture without CPU readback (zero-copy)
    pub fn render_to_view(
        &mut self,
        root: &DirEntry,
        width: u32,
        height: u32,
        camera: &OrbitCamera,
        opts: &Render3DOptions,
        treemap_opts: &TreeMapOptions,
    ) {
        use log::{debug, info, warn};
        let render_start = std::time::Instant::now();
        info!(
            "=== render_to_view START: {}x{}, PT={}, wire={} ===",
            width, height, opts.path_tracing, opts.show_wireframe
        );

        // Force xray_alpha = 1.0 for PBR mode (transparency only in PT)
        let mut opts = opts.clone();
        if !opts.path_tracing {
            opts.xray_alpha = 1.0;
        }
        self.pt.pt_backend_kind = pt::backend_from_opts(&opts);

        // Freeze animation time for PT auto-SPP/camera snap between updates
        if opts.path_tracing {
            let snap_enabled = opts.pt_auto_spp || opts.pt_camera_snap;
            if snap_enabled {
                let frame_count = pt::frame_count(self.pt.pt_backend_kind, self);
                let snap_interval = 1.0 / opts.pt_target_fps.max(1.0);
                let elapsed = self.pt.pt_camera_snap_time.elapsed().as_secs_f32();
                let allow_update =
                    elapsed >= snap_interval || !self.pt.pt_snap_valid || frame_count == 0;
                if !allow_update {
                    opts.animation_time = self.pt.pt_snap_anim_time;
                    opts.animate = false;
                }
            }
        }
        let opts = &opts;

        if width == 0 || height == 0 {
            warn!("render_to_view: zero size, skipping");
            return;
        }

        self.ensure_targets(width, height);

        let (layout_w, layout_h) = self.scene_layout_size();

        // Check if we can reuse cached instances. The cache depends on the logical scene layout,
        // not the output texture size, so resizing the window only updates camera/render targets.
        let opts_hash = Self::opts_hash(opts, layout_w, layout_h, camera, height);
        let cache_valid = !opts.animate
            && self.cached_instances.is_some()
            && self.cached_opts_hash == opts_hash
            && self.cached_layout_size == (layout_w, layout_h);

        trace!("cache_valid: {}, opts_hash: 0x{:x}", cache_valid, opts_hash);

        // Own an `Arc` so we can call `pick_from_existing` (`&mut self`) without borrowing `cached_instances`.
        let instances_arc: Arc<Vec<CubeInstance>> = if cache_valid {
            Arc::clone(self.cached_instances.as_ref().unwrap())
        } else {
            log::debug!(
                "cache MISS: animate={}, has_cache={}, hash_match={}, size_match={}",
                opts.animate,
                self.cached_instances.is_some(),
                self.cached_opts_hash == opts_hash,
                self.cached_layout_size == (layout_w, layout_h)
            );
            // Layout only on cache miss — rect values are already set when cache is valid
            treemap::layout(
                root,
                0.0,
                0.0,
                layout_w as f32,
                layout_h as f32,
                treemap_opts,
            );
            let world_center = Vec3::new(layout_w as f32 / 2.0, -(layout_h as f32 / 2.0), 0.0);
            let new_instances = self.collect_cubes(
                root,
                opts,
                treemap_opts,
                world_center,
                camera.position(),
                height as f32,
                camera.fov,
            );
            // PT scene must be rebuilt to match new object IDs in id_map
            self.pt.pt_scene_dirty = true;

            let arc = Arc::new(new_instances);
            if !opts.animate {
                self.cached_instances = Some(Arc::clone(&arc));
                self.cached_opts_hash = opts_hash;
                self.cached_layout_size = (layout_w, layout_h);
            } else {
                self.cached_instances = Some(Arc::clone(&arc));
            }
            arc
        };

        let instances: &[CubeInstance] = instances_arc.as_ref();

        self.instance_count = instances.len() as u32;
        info!(
            "render_to_view: instance_count={}, cache_valid={}",
            self.instance_count, cache_valid
        );
        if instances.is_empty() {
            warn!("render_to_view: no instances, skipping");
            return;
        }

        // Upload instances (also when buffer was reset even if cache valid)
        let need_upload = !cache_valid || self.instance_buffer.is_none();
        info!(
            "render_to_view: need_upload={}, buffer_exists={}",
            need_upload,
            self.instance_buffer.is_some()
        );
        if need_upload {
            let need_realloc =
                self.instance_buffer.is_none() || instances.len() > self.instance_buffer_capacity;
            if need_realloc {
                let new_capacity = (instances.len() * 5 / 4).max(1024);
                let new_size = new_capacity * std::mem::size_of::<CubeInstance>();
                self.instance_buffer =
                    Some(self.ctx.device.create_buffer(&wgpu::BufferDescriptor {
                        label: Some("Instance VBO"),
                        size: new_size as u64,
                        usage: wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::COPY_DST,
                        mapped_at_creation: false,
                    }));
                self.instance_buffer_capacity = new_capacity;
            }
            if let Some(ref buf) = self.instance_buffer {
                self.ctx
                    .queue
                    .write_buffer(buf, 0, bytemuck::cast_slice(instances));
            }
        }

        // Match outline/hover uniforms to the *current* cursor: read last frame's object_id buffer
        // at pending pixel before encoding (full render still ends with readback from *this* frame).
        if !opts.path_tracing && cache_valid && self.instance_count > 0 {
            if let Some((px, py)) = self.picking.pending_pick {
                self.picking.ensure_readback(&self.ctx.device, width);
                self.pick_from_existing();
                self.picking.request_pick(px, py);
            }
        }

        let hovered_id = self.picking.hovered_id;

        self.update_uniforms(camera, opts, width, height, hovered_id);
        let cam_pos = camera.position();
        info!(
            "render_to_view: camera pos=({:.1},{:.1},{:.1}), dist={:.1}",
            cam_pos.x, cam_pos.y, cam_pos.z, camera.distance
        );

        let mut encoder = self
            .ctx
            .device
            .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                label: Some("3D Encoder (zero-copy)"),
            });

        // Path tracing mode
        if opts.path_tracing {
            info!("render_to_view: PATH TRACING mode");
            drop(encoder);
            // Arc clone to break borrow conflict (cheap - only refcount bump)
            let instances_pt = Arc::clone(&instances_arc);
            let num_cubes = instances_pt.len();
            pt::render_path_traced_no_readback(
                self.pt.pt_backend_kind,
                self,
                &instances_pt,
                camera,
                opts,
                width,
                height,
            );

            // Add outline pass for PT mode (render over PT result)
            if opts.hover_mode != HoverMode::None && hovered_id != 0 {
                let targets = self.targets.as_ref().unwrap();
                let dyn_bgs = self.dyn_bgs.as_ref().unwrap();
                let ib = self.instance_buffer.as_ref().unwrap();

                let mut enc =
                    self.ctx
                        .device
                        .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                            label: Some("PT Outline Encoder"),
                        });

                // Object ID pass (needed for outline detection)
                {
                    let mut pass = enc.begin_render_pass(&wgpu::RenderPassDescriptor {
                        label: Some("PT Object ID"),
                        color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                            view: &targets.object_id_view,
                            resolve_target: None,
                            depth_slice: None,
                            ops: wgpu::Operations {
                                load: wgpu::LoadOp::Clear(wgpu::Color::TRANSPARENT),
                                store: wgpu::StoreOp::Store,
                            },
                        })],
                        depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment {
                            view: &targets.depth_view,
                            depth_ops: Some(wgpu::Operations {
                                load: wgpu::LoadOp::Clear(1.0),
                                store: wgpu::StoreOp::Store,
                            }),
                            stencil_ops: None,
                        }),
                        ..Default::default()
                    });
                    pass.set_pipeline(&self.pipes.object_id);
                    pass.set_bind_group(0, &self.obj_id_bg0, &[]);
                    pass.set_vertex_buffer(0, self.vertex_buffer.slice(..));
                    pass.set_vertex_buffer(1, ib.slice(..));
                    pass.set_index_buffer(self.index_buffer.slice(..), wgpu::IndexFormat::Uint16);
                    pass.draw_indexed(0..NUM_INDICES, 0, 0..self.instance_count);
                }

                // Outline pass
                {
                    let mut pass = enc.begin_render_pass(&wgpu::RenderPassDescriptor {
                        label: Some("PT Outline"),
                        color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                            view: &targets.render_view,
                            resolve_target: None,
                            depth_slice: None,
                            ops: wgpu::Operations {
                                load: wgpu::LoadOp::Load,
                                store: wgpu::StoreOp::Store,
                            },
                        })],
                        depth_stencil_attachment: None,
                        ..Default::default()
                    });
                    pass.set_pipeline(&self.pipes.outline);
                    pass.set_bind_group(0, &dyn_bgs.outline, &[]);
                    pass.draw(0..3, 0..1);
                }

                self.ctx.queue.submit(std::iter::once(enc.finish()));
            }

            let total_ms = render_start.elapsed().as_secs_f64() * 1000.0;
            debug!(
                "PT render (zero-copy): {:.2}ms ({} cubes)",
                total_ms, num_cubes
            );
            return;
        }

        // Encode PBR/wireframe passes
        let targets = self.targets.as_ref().unwrap();
        let dyn_bgs = self.dyn_bgs.as_ref().unwrap();
        info!(
            "render_to_view: calling encode_passes, targets {:?}",
            targets.size
        );
        self.encode_passes(&mut encoder, targets, dyn_bgs, opts, hovered_id);
        info!("render_to_view: encode_passes done, submitting");

        // Submit picking readback (uses pending_pick set by set_mouse_pos)
        let targets = self.targets.as_ref().unwrap();
        self.picking
            .submit_readback(&mut encoder, &targets.object_id_texture, targets.size);

        self.ctx.queue.submit(std::iter::once(encoder.finish()));

        // Blocking poll to read pick result (sync like alembic-rs)
        self.picking.poll_result(&self.ctx.device);

        let total_ms = render_start.elapsed().as_secs_f64() * 1000.0;
        let mode = if opts.show_wireframe {
            "Wire"
        } else if opts.xray_alpha < 1.0 {
            "XRay"
        } else {
            "PBR"
        };
        info!(
            "{} render (zero-copy): {:.2}ms ({} cubes)",
            mode,
            total_ms,
            instances.len()
        );
    }

    // NOTE: render_to_command_buffer and render_path_traced_to_buffer removed (unused callback path)

    /// Path tracing without readback (called from render_to_view)
    pub(crate) fn render_path_traced_no_readback(
        &mut self,
        instances: &[geometry::CubeInstance],
        camera: &OrbitCamera,
        opts: &Render3DOptions,
        width: u32,
        height: u32,
    ) {
        pt::megakernel::render_path_traced_no_readback(
            self, instances, camera, opts, width, height,
        );
    }

    // ========================================================================
    // Bind group helpers
    // ========================================================================

    fn create_env_bind_group(
        device: &wgpu::Device,
        layout: &wgpu::BindGroupLayout,
        env: &env_map::EnvMap,
        env_params_buf: &wgpu::Buffer,
    ) -> wgpu::BindGroup {
        device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("Env BG"),
            layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: wgpu::BindingResource::TextureView(&env.view),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: wgpu::BindingResource::Sampler(&env.sampler),
                },
                wgpu::BindGroupEntry {
                    binding: 2,
                    resource: env_params_buf.as_entire_binding(),
                },
            ],
        })
    }

    /// Call after loading a new env map to update bind groups
    pub fn on_env_map_changed(&mut self) {
        self.env_bg = Self::create_env_bind_group(
            &self.ctx.device,
            &self.layouts.env,
            &self.env,
            &self.env_params_buf,
        );
        // Force dynamic bind group recreation (skybox references env)
        self.dyn_bgs = None;
        self.pt.pt_env_dirty = true;
    }

    // ========================================================================
    // Render targets
    // ========================================================================

    fn ensure_targets(&mut self, w: u32, h: u32) {
        let needs_resize = match &self.targets {
            Some(t) => t.size != (w, h),
            None => true,
        };

        if needs_resize {
            let device = &self.ctx.device;
            let targets = RenderTargets::new(device, w, h);
            self.picking.ensure_readback(device, w);

            self.dyn_bgs = Some(DynamicBindGroups::new(
                device,
                &self.layouts,
                &targets,
                &self.camera_buf,
                &self.hover_params_buf,
                &self.env.view,
                &self.env.sampler,
                &self.env_params_buf,
            ));

            self.targets = Some(targets);
        } else if self.dyn_bgs.is_none() {
            // Recreate dynamic bind groups (e.g. after env map change)
            let targets = self.targets.as_ref().unwrap();
            self.dyn_bgs = Some(DynamicBindGroups::new(
                &self.ctx.device,
                &self.layouts,
                targets,
                &self.camera_buf,
                &self.hover_params_buf,
                &self.env.view,
                &self.env.sampler,
                &self.env_params_buf,
            ));
        }
    }

    /// Compute cube height based on node properties and render options.
    fn compute_cube_height(node: &DirEntry, depth: u32, opts: &Render3DOptions) -> f32 {
        let mut height_value = match opts.height_mode {
            CubeHeightMode::FileSize => {
                if node.size > 0 {
                    (node.size as f32).log2().max(1.0)
                } else {
                    1.0
                }
            }
            CubeHeightMode::OwnSize => {
                if node.own_size > 0 {
                    (node.own_size as f32).log2().max(1.0)
                } else {
                    1.0
                }
            }
            CubeHeightMode::FileCount => (node.file_count.max(1) as f32).log2().max(1.0),
            CubeHeightMode::DirCount => (node.dir_count.max(1) as f32).log2().max(1.0),
            CubeHeightMode::Age => {
                if let Some(mtime) = node.modified_time {
                    let now = std::time::SystemTime::now()
                        .duration_since(std::time::UNIX_EPOCH)
                        .map(|d| d.as_secs())
                        .unwrap_or(0);
                    let age_secs = now.saturating_sub(mtime);
                    let year_secs = 365_u64 * 24 * 60 * 60;
                    let age_norm = (age_secs as f32 / year_secs as f32).clamp(0.0, 1.0);
                    1.0 + (1.0 - age_norm) * 9.0
                } else {
                    1.0
                }
            }
            CubeHeightMode::Depth | CubeHeightMode::DepthSquared => depth as f32 + 1.0,
            CubeHeightMode::Constant => 1.0,
        };
        let mut height_power = 1.0;
        if opts.height_power_enabled {
            height_power = opts.height_power.clamp(0.1, 4.0);
        } else if matches!(opts.height_mode, CubeHeightMode::DepthSquared) {
            height_power = 2.0;
        }
        if height_power != 1.0 {
            height_value = height_value.max(0.0).powf(height_power);
        }

        match opts.height_mode {
            CubeHeightMode::FileSize
            | CubeHeightMode::OwnSize
            | CubeHeightMode::FileCount
            | CubeHeightMode::DirCount
            | CubeHeightMode::Age => height_value * opts.height_scale * 2.0,
            CubeHeightMode::Depth | CubeHeightMode::DepthSquared => {
                height_value * opts.height_scale * 5.0
            }
            CubeHeightMode::Constant => height_value * opts.height_scale * 10.0,
        }
    }

    // ========================================================================
    // CPU picking (ray -> cube AABB)
    // ========================================================================

    /// CPU ray pick against cubes (matches render placement)
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

            let pos = Vec3::new(x + w / 2.0, -(y + h / 2.0), -base_height / 2.0);
            let offset = hash_transform_offset(
                &node.name,
                pos,
                world_center,
                opts.hash_effect,
                opts.hash_effect_strength,
                opts.animation_time,
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

    fn update_uniforms(
        &self,
        camera: &OrbitCamera,
        opts: &Render3DOptions,
        width: u32,
        height: u32,
        hovered_id: u32,
    ) {
        let q = &self.ctx.queue;
        let aspect = width as f32 / height as f32;
        let vp = camera.view_projection_matrix(aspect);
        let view = camera.view_matrix();
        let pos = camera.position();

        q.write_buffer(
            &self.camera_buf,
            0,
            bytemuck::bytes_of(&CameraUniform {
                view_proj: vp.to_cols_array_2d(),
                view: view.to_cols_array_2d(),
                inv_view_proj: vp.inverse().to_cols_array_2d(),
                position: [pos.x, pos.y, pos.z],
                xray_alpha: opts.xray_alpha,
                flat_shading: if opts.flat_shading { 1.0 } else { 0.0 },
                slice_enabled: if opts.slice_enabled { 1.0 } else { 0.0 },
                slice_position: compute_slice_position(opts),
                slice_invert: if opts.slice_invert { 1.0 } else { 0.0 },
                slice_normal: compute_slice_normal(opts),
                _pad: [0.0; 5],
            }),
        );

        q.write_buffer(&self.light_rig_buf, 0, bytemuck::bytes_of(&self.light_rig));

        // Per-frame mat-global update. Cheap (16 bytes); enables live materialize_mix
        // slider without rebuilding cube instances.
        let mat_mix = if opts.materialize_mode != MaterializeMode::None {
            opts.materialize_mix.clamp(0.0, 1.0)
        } else {
            0.0
        };
        q.write_buffer(
            &self.mat_global_buf,
            0,
            bytemuck::bytes_of(&MatGlobalUniform {
                materialize_mix: mat_mix,
                _pad: [0.0; 3],
            }),
        );

        q.write_buffer(
            &self.env_params_buf,
            0,
            bytemuck::bytes_of(&EnvParamsUniform {
                intensity: opts.env_map_intensity,
                rotation: opts.env_map_rotation,
                enabled: if opts.env_map_enabled { 1.0 } else { 0.0 },
                _pad: 0.0,
            }),
        );

        // Update selected IDs storage buffer: [count, id0, id1, ...]
        let selected_data: Vec<u32> = std::iter::once(self.selected_ids.len() as u32)
            .chain(self.selected_ids.iter().copied())
            .collect();
        q.write_buffer(
            &self.selected_ids_buf,
            0,
            bytemuck::cast_slice(&selected_data),
        );

        // Determine active ID for outline: prefer selected (if any), else hovered
        let (active_id, outline_color) =
            if let Some(&first_selected) = self.selected_ids.iter().next() {
                // Selected: use blue color
                (first_selected, [0.2, 0.6, 1.0, opts.hover_outline_alpha])
            } else {
                // Hovered: use orange color
                (hovered_id, [1.0, 0.5, 0.0, opts.hover_outline_alpha])
            };

        q.write_buffer(
            &self.hover_params_buf,
            0,
            bytemuck::bytes_of(&HoverParamsUniform {
                hovered_id: active_id,
                mode: opts.hover_mode.to_u32(),
                outline_width: opts.hover_outline_width,
                _pad0: 0.0,
                outline_color,
                tint_color: [1.0, 0.7, 0.2, 0.15],
                viewport_size: [width as f32, height as f32],
                _pad1: [0.0; 2],
            }),
        );
    }

    // ========================================================================
    // Render passes
    // ========================================================================

    /// Encode all render passes into the command encoder
    fn encode_passes(
        &self,
        encoder: &mut wgpu::CommandEncoder,
        targets: &RenderTargets,
        dyn_bgs: &DynamicBindGroups,
        opts: &Render3DOptions,
        hovered_id: u32,
    ) {
        log::trace!(
            "encode_passes: START, instance_count={}, buffer_cap={}",
            self.instance_count,
            self.instance_buffer_capacity
        );
        let ib = match self.instance_buffer.as_ref() {
            Some(b) => {
                log::trace!("encode_passes: instance_buffer size={}", b.size());
                b
            }
            None => {
                log::error!("encode_passes: NO INSTANCE BUFFER!");
                return;
            }
        };

        // Pass 1: Main geometry (PBR / wireframe / transparent)
        {
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("Main"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: &targets.render_view,
                    resolve_target: None,
                    depth_slice: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Clear(wgpu::Color {
                            r: opts.background_color[0] as f64,
                            g: opts.background_color[1] as f64,
                            b: opts.background_color[2] as f64,
                            a: 1.0,
                        }),
                        store: wgpu::StoreOp::Store,
                    },
                })],
                depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment {
                    view: &targets.depth_view,
                    depth_ops: Some(wgpu::Operations {
                        load: wgpu::LoadOp::Clear(1.0),
                        store: wgpu::StoreOp::Store,
                    }),
                    stencil_ops: None,
                }),
                ..Default::default()
            });

            let pipe_name = if opts.show_wireframe {
                "wireframe"
            } else if opts.xray_alpha < 1.0 {
                "transparent"
            } else if opts.double_sided {
                "pbr_double"
            } else {
                "pbr"
            };
            log::trace!(
                "encode_passes: using {} pipeline, {} instances, xray={}, double={}",
                pipe_name,
                self.instance_count,
                opts.xray_alpha,
                opts.double_sided
            );

            let pipe = if opts.show_wireframe {
                &self.pipes.wireframe
            } else if opts.xray_alpha < 1.0 {
                &self.pipes.transparent
            } else if opts.double_sided {
                &self.pipes.pbr_double
            } else {
                &self.pipes.pbr
            };

            pass.set_pipeline(pipe);
            pass.set_bind_group(0, &self.pbr_bg0, &[]);
            pass.set_bind_group(1, &self.env_bg, &[]);
            pass.set_vertex_buffer(0, self.vertex_buffer.slice(..));
            pass.set_vertex_buffer(1, ib.slice(..));
            pass.set_index_buffer(self.index_buffer.slice(..), wgpu::IndexFormat::Uint16);
            log::trace!(
                "encode_passes: draw_indexed indices=0..{}, instances=0..{}",
                NUM_INDICES,
                self.instance_count
            );
            pass.draw_indexed(0..NUM_INDICES, 0, 0..self.instance_count);
            log::trace!("encode_passes: Pass 1 (Main) DONE");
        }

        // Pass 2: Skybox (behind geometry via depth test)
        if opts.env_map_enabled && opts.env_map_visible {
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("Skybox"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: &targets.render_view,
                    resolve_target: None,
                    depth_slice: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Load,
                        store: wgpu::StoreOp::Store,
                    },
                })],
                depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment {
                    view: &targets.depth_view,
                    depth_ops: Some(wgpu::Operations {
                        load: wgpu::LoadOp::Load,
                        store: wgpu::StoreOp::Discard,
                    }),
                    stencil_ops: None,
                }),
                ..Default::default()
            });
            pass.set_pipeline(&self.pipes.skybox);
            pass.set_bind_group(0, &dyn_bgs.skybox, &[]);
            pass.draw(0..3, 0..1);
        }

        // Pass 3: Object ID (for hover picking)
        if opts.hover_mode != HoverMode::None {
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("Object ID"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: &targets.object_id_view,
                    resolve_target: None,
                    depth_slice: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Clear(wgpu::Color::BLACK),
                        store: wgpu::StoreOp::Store,
                    },
                })],
                depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment {
                    view: &targets.depth_view,
                    depth_ops: Some(wgpu::Operations {
                        load: wgpu::LoadOp::Clear(1.0), // Fresh depth for correct picking
                        store: wgpu::StoreOp::Discard,
                    }),
                    stencil_ops: None,
                }),
                ..Default::default()
            });
            if opts.double_sided {
                pass.set_pipeline(&self.pipes.object_id_double);
            } else {
                pass.set_pipeline(&self.pipes.object_id);
            }
            pass.set_bind_group(0, &self.obj_id_bg0, &[]);
            pass.set_vertex_buffer(0, self.vertex_buffer.slice(..));
            pass.set_vertex_buffer(1, ib.slice(..));
            pass.set_index_buffer(self.index_buffer.slice(..), wgpu::IndexFormat::Uint16);
            pass.draw_indexed(0..NUM_INDICES, 0, 0..self.instance_count);
        }

        // Pass 4: Outline/tint overlay (fullscreen post-process)
        let has_active = !self.selected_ids.is_empty() || hovered_id != 0;
        info!(
            "encode_passes: outline condition hover_mode={:?}, hovered_id={}, selected={}",
            opts.hover_mode,
            hovered_id,
            self.selected_ids.len()
        );
        if opts.hover_mode != HoverMode::None && has_active {
            info!("encode_passes: RENDERING OUTLINE PASS");
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("Outline"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: &targets.render_view,
                    resolve_target: None,
                    depth_slice: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Load,
                        store: wgpu::StoreOp::Store,
                    },
                })],
                depth_stencil_attachment: None,
                ..Default::default()
            });
            pass.set_pipeline(&self.pipes.outline);
            pass.set_bind_group(0, &dyn_bgs.outline, &[]);
            pass.draw(0..3, 0..1);
        }
    }

    // ========================================================================
    // Public API
    // ========================================================================

    /// Load env map from file, updating bind groups
    pub fn load_env_map(&mut self, path: &std::path::Path) -> anyhow::Result<()> {
        self.env.load_from_file(&self.ctx, path)?;
        self.on_env_map_changed();
        Ok(())
    }

    /// Set mouse position for picking
    pub fn set_mouse_pos(&mut self, x: u32, y: u32) {
        debug!("set_mouse_pos: ({}, {})", x, y);
        self.mouse_pos = Some((x, y));
        self.picking.request_pick(x, y);
    }

    /// Set hovered object ID directly (for PT mode where picking is done via gpu_pick)
    pub fn set_hovered_id(&mut self, id: u32) {
        self.picking.hovered_id = id;
    }

    /// Set selected object IDs for outline rendering
    pub fn set_selected_ids(&mut self, ids: &std::collections::HashSet<u32>) {
        self.selected_ids = ids.clone();
    }

    /// Total number of times the instance buffer has been rebuilt since
    /// renderer construction. Used by Stage A.1 verification to confirm
    /// that toggling `materialize_mix` (a shader-side uniform) does NOT
    /// trigger an instance rebuild — the counter should hold steady when
    /// only the mix slider changes between frames. Wraps on overflow.
    pub fn instance_rebuild_count(&self) -> u64 {
        self.cached_instances_rebuild_count
    }

    /// Last resolved object under the cursor (updated by GPU readback after `pick_from_existing` / `render_to_view`).
    pub fn hovered_id(&self) -> u32 {
        self.picking.hovered_id
    }

    /// Check if there's a pending pick request (for triggering re-render)
    pub fn has_pending_pick(&self) -> bool {
        self.picking.pending_pick.is_some()
    }

    /// Readback a pixel from the existing object_id texture without re-rendering.
    /// Use when only the mouse moved but the scene (camera, geometry, options) is unchanged.
    pub fn pick_from_existing(&mut self) {
        let Some(targets) = &self.targets else { return };
        if self.instance_count == 0 {
            return;
        }

        let mut encoder = self
            .ctx
            .device
            .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                label: Some("Pick-only readback"),
            });
        self.picking
            .submit_readback(&mut encoder, &targets.object_id_texture, targets.size);
        self.ctx.queue.submit(std::iter::once(encoder.finish()));
        self.picking.poll_result(&self.ctx.device);
    }

    /// Current PT samples-per-update (auto-SPP uses this).
    pub fn pt_samples_per_update(&self) -> u32 {
        self.pt.pt_samples_per_update
    }

    /// Current accumulated sample count (for progress display)
    pub fn pt_frame_count(&self) -> u32 {
        pt::frame_count(self.pt.pt_backend_kind, self)
    }

    fn pt_frame_count_impl(&self) -> u32 {
        self.pt
            .path_tracer
            .as_ref()
            .map(|pt| pt.frame_count)
            .unwrap_or(0)
    }

    /// Read back current render texture pixels (for screenshots)
    pub fn readback_render_texture(&self) -> Vec<u8> {
        let Some(targets) = &self.targets else {
            return Vec::new();
        };
        let (width, height) = targets.size;
        if width == 0 || height == 0 {
            return Vec::new();
        }

        let mut encoder = self
            .ctx
            .device
            .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                label: Some("Readback Encoder"),
            });
        let output_buffer = gpu::readback_texture(
            &self.ctx,
            &mut encoder,
            &targets.render_texture,
            width,
            height,
        );
        self.ctx.queue.submit(std::iter::once(encoder.finish()));
        gpu::map_readback(&self.ctx, &output_buffer, width, height)
    }

    /// Build a world-space ray from screen coordinates
    pub fn screen_ray(
        width: u32,
        height: u32,
        camera: &OrbitCamera,
        screen_x: f32,
        screen_y: f32,
    ) -> Option<(Vec3, Vec3)> {
        if width == 0 || height == 0 {
            return None;
        }
        let rel_x = (screen_x / width as f32).clamp(0.0, 1.0);
        let rel_y = (screen_y / height as f32).clamp(0.0, 1.0);

        let aspect = width as f32 / height as f32;
        let view = camera.view_matrix();
        let proj = camera.projection_matrix(aspect);
        let inv_view_proj = (proj * view).inverse();

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
        Some((ray_origin, ray_dir))
    }

    /// GPU BVH pick (PT mode)
    pub fn pt_pick(&mut self, origin: Vec3, dir: Vec3) -> Option<(u32, f32)> {
        pt::pick(self.pt.pt_backend_kind, self, origin, dir)
    }

    fn pt_pick_impl(&mut self, origin: Vec3, dir: Vec3) -> Option<(u32, f32)> {
        let pt = self.pt.path_tracer.as_mut()?;
        pt.gpu_pick(
            &self.ctx.device,
            &self.ctx.queue,
            origin.to_array(),
            dir.to_array(),
        )
    }

    /// Look up path for an object ID
    pub fn path_for_id(&self, id: u32) -> Option<&std::path::PathBuf> {
        self.picking.path_for_id(id)
    }

    /// Look up object ID for a path (reverse lookup)
    pub fn id_for_path(&self, path: &std::path::Path) -> Option<u32> {
        self.picking.id_for_path(path)
    }

    /// Look up file size for an object ID
    pub fn size_for_id(&self, id: u32) -> Option<u64> {
        self.picking.size_for_id(id)
    }

    /// Get center and size of an instance by object ID
    pub fn instance_center_and_size(&self, id: u32) -> Option<(glam::Vec3, glam::Vec3)> {
        let instances = self.cached_instances.as_ref()?;
        let inst = instances.iter().find(|i| i.object_id == id)?;
        // Extract position from model matrix (translation is in column 3)
        let m = glam::Mat4::from_cols_array_2d(&inst.model);
        let center = m.col(3).truncate();
        // Extract scale from model matrix (length of each axis column)
        let sx = m.col(0).truncate().length();
        let sy = m.col(1).truncate().length();
        let sz = m.col(2).truncate().length();
        Some((center, glam::Vec3::new(sx, sy, sz)))
    }

    /// Get all cached instances (for marquee selection)
    pub fn cached_instances(&self) -> Option<&Vec<geometry::CubeInstance>> {
        self.cached_instances.as_deref()
    }

    /// Render the 3D treemap to a pixel buffer
    pub fn render(
        &mut self,
        root: &DirEntry,
        width: u32,
        height: u32,
        camera: &OrbitCamera,
        opts: &Render3DOptions,
        treemap_opts: &TreeMapOptions,
    ) -> Vec<u8> {
        let render_start = std::time::Instant::now();
        if width == 0 || height == 0 {
            return vec![];
        }

        let hovered_id = self.picking.hovered_id;

        let mut opts = opts.clone();
        self.pt.pt_backend_kind = pt::backend_from_opts(&opts);
        if opts.path_tracing {
            let snap_enabled = opts.pt_auto_spp || opts.pt_camera_snap;
            if snap_enabled {
                let frame_count = pt::frame_count(self.pt.pt_backend_kind, self);
                let snap_interval = 1.0 / opts.pt_target_fps.max(1.0);
                let elapsed = self.pt.pt_camera_snap_time.elapsed().as_secs_f32();
                let allow_update =
                    elapsed >= snap_interval || !self.pt.pt_snap_valid || frame_count == 0;
                if !allow_update {
                    opts.animation_time = self.pt.pt_snap_anim_time;
                    opts.animate = false;
                }
            }
        }
        let opts = &opts;

        // Wait for previous GPU work before starting new frame
        let _ = self.ctx.device.poll(wgpu::PollType::wait_indefinitely());
        self.ensure_targets(width, height);

        let (layout_w, layout_h) = self.scene_layout_size();

        // Check if we can reuse cached instances (only when not animating). Keep geometry stable
        // across output resizes by caching against the logical scene layout size.
        let opts_hash = Self::opts_hash(opts, layout_w, layout_h, camera, height);
        let cache_valid = !opts.animate
            && self.cached_instances.is_some()
            && self.cached_opts_hash == opts_hash
            && self.cached_layout_size == (layout_w, layout_h);

        trace!("cache_valid: {}, opts_hash: 0x{:x}", cache_valid, opts_hash);

        let instances = if cache_valid {
            // Reuse cached instances
            self.cached_instances.as_ref().unwrap()
        } else {
            log::debug!(
                "PT cache MISS: animate={}, has_cache={}, hash_match={}, size_match={}",
                opts.animate,
                self.cached_instances.is_some(),
                self.cached_opts_hash == opts_hash,
                self.cached_layout_size == (layout_w, layout_h)
            );
            // Layout only on cache miss — rect values are already set when cache is valid
            treemap::layout(
                root,
                0.0,
                0.0,
                layout_w as f32,
                layout_h as f32,
                treemap_opts,
            );
            // Collect new instances (this rebuilds id_map with new IDs)
            let world_center = Vec3::new(layout_w as f32 / 2.0, -(layout_h as f32 / 2.0), 0.0);
            let new_instances = self.collect_cubes(
                root,
                opts,
                treemap_opts,
                world_center,
                camera.position(),
                height as f32,
                camera.fov,
            );
            // PT scene must be rebuilt to match new object IDs in id_map
            self.pt.pt_scene_dirty = true;

            // Cache if not animating
            let arc = Arc::new(new_instances);
            if !opts.animate {
                self.cached_instances = Some(arc);
                self.cached_opts_hash = opts_hash;
                self.cached_layout_size = (layout_w, layout_h);
                self.cached_instances.as_ref().unwrap()
            } else {
                // For animated mode, store temporarily and return reference
                self.cached_instances = Some(arc);
                self.cached_instances.as_ref().unwrap()
            }
        };

        self.instance_count = instances.len() as u32;

        if instances.is_empty() {
            return vec![30; (width * height * 4) as usize];
        }

        let buf_size = instances.len() * std::mem::size_of::<CubeInstance>();
        if buf_size > 128 * 1024 * 1024 {
            warn!(
                "Instance buffer {} MB exceeds 128 MB GPU limit!",
                buf_size / 1048576
            );
        }

        // Only update GPU buffer if instances changed
        if !cache_valid {
            let upload_start = std::time::Instant::now();
            // Reuse buffer if capacity is sufficient, otherwise reallocate
            let need_realloc =
                self.instance_buffer.is_none() || instances.len() > self.instance_buffer_capacity;

            if need_realloc {
                // Allocate with 25% growth factor to avoid frequent reallocs
                let new_capacity = (instances.len() * 5 / 4).max(1024);
                let new_size = new_capacity * std::mem::size_of::<CubeInstance>();
                self.instance_buffer =
                    Some(self.ctx.device.create_buffer(&wgpu::BufferDescriptor {
                        label: Some("Instance VBO"),
                        size: new_size as u64,
                        usage: wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::COPY_DST,
                        mapped_at_creation: false,
                    }));
                self.instance_buffer_capacity = new_capacity;
            }

            // Update buffer contents
            if let Some(ref buf) = self.instance_buffer {
                self.ctx
                    .queue
                    .write_buffer(buf, 0, bytemuck::cast_slice(instances));
            }
            let upload_ms = upload_start.elapsed().as_secs_f64() * 1000.0;
            debug!(
                "buffer_upload: {:.2}ms ({:.2} MB)",
                upload_ms,
                buf_size as f64 / 1048576.0
            );
        }

        self.update_uniforms(camera, opts, width, height, hovered_id);

        let mut encoder = self
            .ctx
            .device
            .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                label: Some("3D Encoder"),
            });

        // Path tracing mode
        if opts.path_tracing {
            drop(encoder);
            // Arc clone to break borrow conflict (cheap - only refcount bump)
            let instances_arc = Arc::clone(instances);
            return pt::render_path_traced(
                self.pt.pt_backend_kind,
                self,
                &instances_arc,
                camera,
                opts,
                width,
                height,
            );
        }

        // We need targets and dyn_bgs — borrow them safely
        let passes_start = std::time::Instant::now();
        let targets = self.targets.as_ref().unwrap();
        let dyn_bgs = self.dyn_bgs.as_ref().unwrap();
        self.encode_passes(&mut encoder, targets, dyn_bgs, opts, hovered_id);
        let passes_ms = passes_start.elapsed().as_secs_f64() * 1000.0;
        debug!("render_passes: {:.2}ms", passes_ms);

        // Submit picking readback
        let targets = self.targets.as_ref().unwrap();
        self.picking
            .submit_readback(&mut encoder, &targets.object_id_texture, targets.size);

        let targets = self.targets.as_ref().unwrap();
        let output_buffer = gpu::readback_texture(
            &self.ctx,
            &mut encoder,
            &targets.render_texture,
            width,
            height,
        );

        let submit_start = std::time::Instant::now();
        self.ctx.queue.submit(std::iter::once(encoder.finish()));
        let submit_ms = submit_start.elapsed().as_secs_f64() * 1000.0;
        debug!("  submit: {:.2}ms", submit_ms);

        // Sync pick result
        self.picking.poll_result(&self.ctx.device);

        let readback_start = std::time::Instant::now();
        let result = gpu::map_readback(&self.ctx, &output_buffer, width, height);
        let readback_ms = readback_start.elapsed().as_secs_f64() * 1000.0;
        debug!("  readback: {:.2}ms ({}x{})", readback_ms, width, height);

        let total_ms = render_start.elapsed().as_secs_f64() * 1000.0;
        let render_mode = if opts.show_wireframe {
            "Wireframe"
        } else if opts.xray_alpha < 1.0 {
            "Transparent"
        } else {
            "PBR"
        };
        info!(
            "PBR render: {:.2}ms ({} cubes, {})",
            total_ms,
            instances.len(),
            render_mode
        );

        result
    }

    /// Render using path tracer compute shader.
    pub(crate) fn render_path_traced(
        &mut self,
        instances: &[geometry::CubeInstance],
        camera: &OrbitCamera,
        opts: &Render3DOptions,
        width: u32,
        height: u32,
    ) -> Vec<u8> {
        pt::megakernel::render_path_traced(self, instances, camera, opts, width, height)
    }
}

fn ray_aabb_intersect(ray_origin: Vec3, ray_dir: Vec3, min: Vec3, max: Vec3) -> Option<f32> {
    let mut tmin = f32::NEG_INFINITY;
    let mut tmax = f32::INFINITY;

    for i in 0..3 {
        let origin = ray_origin[i];
        let dir = ray_dir[i];
        let min_i = min[i];
        let max_i = max[i];

        if dir.abs() < 1e-6 {
            if origin < min_i || origin > max_i {
                return None;
            }
            continue;
        }

        let inv = 1.0 / dir;
        let mut t1 = (min_i - origin) * inv;
        let mut t2 = (max_i - origin) * inv;
        if t1 > t2 {
            std::mem::swap(&mut t1, &mut t2);
        }
        tmin = tmin.max(t1);
        tmax = tmax.min(t2);
        if tmax < tmin {
            return None;
        }
    }

    if tmax < 0.0 {
        None
    } else {
        Some(tmin.max(0.0))
    }
}
