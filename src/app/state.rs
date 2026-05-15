//! App state definitions: App struct, PersistState, defaults.

use std::collections::HashSet;
use std::path::PathBuf;
use std::sync::atomic::AtomicBool;
use std::sync::Arc;

use crossbeam_channel::Receiver;
use eframe::egui;
use egui_dock::DockState;
use serde::{Deserialize, Serialize};

use super::DockTab;

use super::presets::RenderPreset;
use crate::events::EventBus;
use crate::exclusions::Exclusions;
use crate::renderer::{OrbitCamera, Render3DOptions, RenderBackend, RenderMode};
use crate::scanner::ScanMsg;
use squarebob_core::DirEntry;
use render_3d::Renderer3D;
use render_core::gpu::GpuContext;
use render_core::Viewport;
use treemap::GpuRenderer2D;
use treemap::TreeMapOptions;

/// Scanner backend selection.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub enum ScannerMode {
    Standard,
    Ntfs,
}

/// Settings panel tabs.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
pub enum SettingsTab {
    #[default]
    General,
    Rendering,
    Exclusions,
    Extensions,
}

/// Persistent settings saved between sessions
#[derive(Serialize, Deserialize)]
pub(super) struct PersistState {
    pub scan_path: String,
    pub show_settings: bool,
    pub dark_mode: bool,
    pub scanner_mode: ScannerMode,
    pub filter_auto_rebuild: bool,
    pub path_history: Vec<String>,
    pub opts: SavedOpts,
    #[serde(default = "default_tree_width")]
    pub tree_panel_width: f32,
    #[serde(default = "default_settings_width")]
    pub settings_panel_width: f32,
    #[serde(default)]
    pub show_free_space: bool,
    #[serde(default)]
    pub render_backend: RenderBackend,
    #[serde(default)]
    pub render_mode: RenderMode,
    #[serde(default)]
    pub render_3d_opts: Render3DOptions,
    #[serde(default = "crate::app::dock::default_dock_state")]
    pub dock_state: DockState<DockTab>,
    #[serde(default = "default_font_size")]
    pub font_size: f32,
    #[serde(default)]
    pub settings_tab: SettingsTab,
    #[serde(default)]
    pub ext_filter: Vec<String>,
    #[serde(default)]
    pub ext_filter_invert: bool,
    #[serde(default = "default_settings_tint_mix")]
    pub settings_tint_mix: f32,
    #[serde(default = "default_settings_section_header_height")]
    pub settings_section_header_height: f32,
    #[serde(default = "default_settings_panel_font_body")]
    pub settings_panel_font_body: f32,
    #[serde(default = "default_settings_panel_font_heading")]
    pub settings_panel_font_heading: f32,
    #[serde(default = "default_settings_panel_font_subheading")]
    pub settings_panel_font_subheading: f32,
    #[serde(default = "default_settings_panel_font_small")]
    pub settings_panel_font_small: f32,
    #[serde(default = "default_settings_panel_font_button")]
    pub settings_panel_font_button: f32,
    #[serde(default = "default_settings_panel_font_monospace")]
    pub settings_panel_font_monospace: f32,
    #[serde(default)]
    pub preset_autosave: bool,
    #[serde(default = "default_autosave_interval")]
    pub autosave_interval_secs: f32,
    #[serde(default)]
    pub encode_dialog_settings: media_encoder::EncodeDialogSettings,
    /// Merge files outside size range into LoD buckets (see View → Size filter).
    #[serde(default)]
    pub filter_merge_outside: bool,
}

pub(super) fn default_autosave_interval() -> f32 {
    5.0
}

pub(super) fn default_font_size() -> f32 {
    12.0
}
pub(super) fn default_settings_tint_mix() -> f32 {
    0.12
}

pub(super) fn default_settings_section_header_height() -> f32 {
    15.0
}

pub(super) fn default_settings_panel_font_body() -> f32 {
    10.0
}

pub(super) fn default_settings_panel_font_heading() -> f32 {
    11.0
}

pub(super) fn default_settings_panel_font_subheading() -> f32 {
    9.0
}

pub(super) fn default_settings_panel_font_small() -> f32 {
    8.0
}

pub(super) fn default_settings_panel_font_button() -> f32 {
    10.0
}

pub(super) fn default_settings_panel_font_monospace() -> f32 {
    10.0
}

pub(super) fn default_tree_width() -> f32 {
    200.0
}
pub(super) fn default_settings_width() -> f32 {
    280.0
}

#[derive(Serialize, Deserialize)]
pub(super) struct SavedOpts {
    pub style: String,
    pub grid: bool,
    pub brightness: f64,
    pub height: f64,
    pub scale_factor: f64,
    pub ambient_light: f64,
    pub light_x: f64,
    pub light_y: f64,
}

pub struct App {
    pub(super) events: EventBus,
    pub(super) scan_path: String,
    pub(super) scan_rx: Option<Receiver<ScanMsg>>,
    pub(super) scan_cancel: Option<Arc<AtomicBool>>,
    pub(super) tree: Option<DirEntry>,
    pub(super) filtered_tree: Option<DirEntry>,
    pub(super) treemap_tex: Option<egui::TextureHandle>,
    pub(super) last_render_size: (u32, u32),
    pub(super) opts: TreeMapOptions,
    pub(super) progress: ScanProgress,
    pub(super) hovered: Option<HoverInfo>,
    pub(super) last_wheel_zoom: std::time::Instant,
    pub(super) selected_path: Option<PathBuf>,
    pub(super) scroll_to_selected: bool,
    pub(super) show_settings: bool,
    pub(super) expanded: std::collections::HashSet<PathBuf>,
    pub(super) needs_layout: bool,
    pub(super) needs_render_3d: bool,
    pub(super) dark_mode: bool,
    pub(super) scanner_mode: ScannerMode,
    pub(super) ctx_menu_path: Option<PathBuf>,
    pub(super) ctx_menu_pos: Option<egui::Pos2>,
    pub(super) pending_trash_path: Option<PathBuf>,
    pub(super) ext_stats: Vec<(String, u64, u64)>,
    pub(super) zoom_path: Option<PathBuf>,
    pub(super) path_history: Vec<String>,
    pub(super) filter_min: u64,
    pub(super) filter_max: u64,
    pub(super) filter_invert: bool,
    /// When true (and not inverted), files smaller than min or larger than max are merged into synthetic leaves.
    pub(super) filter_merge_outside: bool,
    /// LoD buckets the user has expanded (double-click); paths are `…/__squarebob_lod_small` / `…/__squarebob_lod_large`.
    pub(super) lod_expanded_paths: HashSet<PathBuf>,
    pub(super) scan_min_size: u64,
    pub(super) scan_max_size: u64,
    pub(super) needs_filter_rebuild: bool,
    pub(super) filter_auto_rebuild: bool,
    pub(super) filter_changed_at: Option<std::time::Instant>,
    pub(super) ext_sort: (usize, bool),
    pub(super) search_text: String,
    pub(super) show_search: bool,
    pub(super) ext_search_text: String,
    pub(super) ext_filter: Vec<String>,
    pub(super) ext_filter_invert: bool,
    pub(super) settings_tint_mix: f32,
    /// Pixel height of the click row for tinted + compact collapsing headers in the settings panel.
    pub(super) settings_section_header_height: f32,
    /// Settings sidebar only — `TextStyle::Body` (labels, default text).
    pub(super) settings_panel_font_body: f32,
    /// Settings sidebar — `TextStyle::Heading` (top-level section titles via `RichText::heading()`).
    pub(super) settings_panel_font_heading: f32,
    /// Settings sidebar — nested collapsibles (Geometry subsections, ramp headers), explicit pt.
    pub(super) settings_panel_font_subheading: f32,
    /// Settings sidebar — `TextStyle::Small`.
    pub(super) settings_panel_font_small: f32,
    /// Settings sidebar — `TextStyle::Button` (tab bar, buttons).
    pub(super) settings_panel_font_button: f32,
    /// Settings sidebar — `TextStyle::Monospace`.
    pub(super) settings_panel_font_monospace: f32,
    pub(super) dock_state: DockState<DockTab>,
    pub(super) tree_panel_width: f32,
    pub(super) settings_panel_width: f32,
    pub(super) cache_age: Option<u64>,
    pub(super) show_free_space: bool,
    pub(super) display_tree_cache: Option<DirEntry>,
    pub(super) exclusions: Exclusions,
    pub(super) show_excluded: bool,
    pub(super) viewport: Viewport,
    pub(super) render_backend: RenderBackend,
    pub(super) render_mode: RenderMode,
    pub(super) render_3d_opts: Render3DOptions,
    // Presets
    pub(super) presets: std::collections::HashMap<String, RenderPreset>,
    pub(super) preset_name: String,
    pub(super) preset_dropdown_open: bool,
    pub(super) preset_autosave: bool,
    pub(super) autosave_interval_secs: f32,
    pub(super) preset_dirty: bool,
    pub(super) preset_last_save: std::time::Instant,
    pub(super) orbit_camera: OrbitCamera,
    pub(super) gpu_context: Option<Arc<GpuContext>>,
    /// Lazy-built OIDN denoiser. Replaces the previous à-trous filter.
    /// Materialised the first time the render loop sees a non-`Off` mode
    /// and a built PT pipeline.
    pub(super) oidn_denoiser: Option<pt_denoise_oidn::OidnDenoiser>,
    /// One-shot trigger: UI sets this to true ("Denoise now"), the render
    /// loop honours it once on the next frame and clears it.
    pub(super) oidn_run_requested: bool,
    /// Wallclock for the last successful OIDN pass, shown in the UI.
    pub(super) oidn_last_latency_ms: Option<f32>,
    /// True if OIDN has already run once during the current PT accumulation.
    /// Reset whenever `pt_frame_count` drops (camera/scene change).
    pub(super) oidn_denoised_this_accumulation: bool,
    /// Last observed PT frame counter — used to detect accumulation resets.
    pub(super) oidn_last_frame_count: u32,
    /// True when the egui-registered texture currently points at the OIDN
    /// result rather than the raw PT target. Drives transition logic in
    /// `treemap_view::render_3d_callback` so we re-`update_egui_texture`
    /// only on the raw↔oidn edge.
    pub(super) oidn_display_is_denoised: bool,
    /// Mirror of `oidn_display_is_denoised` from the previous frame, used
    /// to detect raw↔oidn transitions even when this frame chose the same
    /// state again.
    pub(super) oidn_last_display_was_denoised: bool,
    /// `pt_frame_count` at the moment of the last periodic OIDN fire.
    /// Used by the `pt_oidn_interval` trigger to space periodic re-runs
    /// without firing every frame at multiples of the interval.
    pub(super) oidn_last_interval_spp: u32,
    pub(super) wgpu_render_state: Option<egui_wgpu::RenderState>,
    pub(super) renderer_2d_gpu: Option<GpuRenderer2D>,
    pub(super) renderer_3d: Option<Renderer3D>,
    /// Zero-copy texture ID registered with egui
    pub(super) render_texture_id: Option<egui::TextureId>,
    pub(super) hovered_3d_id: u32,
    /// Selected object IDs in 3D mode (left click adds/removes)
    pub(super) selected_3d_ids: std::collections::HashSet<u32>,
    /// Marquee selection start point (shift+drag)
    pub(super) marquee_start: Option<egui::Pos2>,
    /// Snapshot of `selected_3d_ids` at the moment the marquee drag
    /// started. Each frame the live preview resets back to this baseline
    /// and re-adds the cubes inside the current rectangle, so the
    /// highlight tracks cursor swings without permanently capturing
    /// cubes that were briefly inside earlier in the drag.
    pub(super) marquee_baseline: Option<std::collections::HashSet<u32>>,
    pub(super) last_hover_pos_3d: Option<(f32, f32)>,
    /// Throttle: last time a hover pick was issued
    pub(super) last_pick_time_3d: std::time::Instant,
    /// Sticky tooltip: cached path/size shown when mouse stops moving
    pub(super) sticky_hover: Option<(PathBuf, u64)>,
    pub(super) frame_count: u32,
    pub(super) file_mask_text: String,
    pub(super) use_file_mask: bool,
    pub(super) last_search_text: String,
    pub(super) last_file_mask_text: String,
    pub(super) last_use_file_mask: bool,
    pub(super) filtered_paths_cache: Option<std::collections::HashSet<PathBuf>>,
    /// Global UI font size
    pub(super) font_size: f32,
    pub(super) settings_tab: SettingsTab,

    // Screenshot/testing state
    pub(super) screenshot_delay: Option<f32>,
    pub(super) screenshot_path: Option<String>,
    pub(super) exit_after_screenshot: bool,
    pub(super) screenshot_start_time: Option<std::time::Instant>,
    pub(super) screenshot_taken: bool,
    pub(super) last_render_frame_3d: u32,
    pub(super) last_render_instant_3d: Option<std::time::Instant>,
    pub(super) render_tick_3d: bool,
    pub(super) last_frame_ms: f32,
    pub(super) last_fps: f32,
    pub(super) last_samples_per_sec: f32,
    pub(super) last_samples_per_frame: u32,
    /// Sliding 1-second window of (timestamp, frame_ms) pairs. Used to compute
    /// `avg_frame_ms` / `avg_fps` shown in the status bar — a stable bench display
    /// that ignores per-frame jitter.
    pub(super) frame_history: std::collections::VecDeque<(std::time::Instant, f32)>,
    pub(super) avg_frame_ms: f32,
    pub(super) avg_fps: f32,
    pub(super) mem_used_mb: u64,
    pub(super) mem_total_mb: u64,
    pub(super) last_mem_update: std::time::Instant,
    pub(super) sys: sysinfo::System,
    pub(super) wgpu_error_flag: Arc<AtomicBool>,
    pub(super) pt_auto_spp_tick: std::time::Instant,
    pub(super) show_encode_panel: bool,
    pub(super) encode_dialog: media_encoder::EncodeDialog,
    pub(super) encode_source: Option<media_encoder::Comp>,
    pub(super) encode_sequence_source: Option<Arc<crate::app::image_sequence::SquarebobEncodeSource>>,
    pub(super) encode_source_size: (u32, u32),
    pub(super) encode_active_frame: Option<crate::app::image_sequence::EncodeFrameRequest>,
    pub(super) encode_render_state_active: bool,
    pub(super) encode_restore_render_mode: RenderMode,
    pub(super) encode_base_animation_time: f32,
    pub(super) encode_base_env_time: f32,
    pub(super) encode_restore_animate: bool,
    pub(super) encode_restore_env_animate: bool,
    /// Wall-clock anchor for advancing `animation_time` / `env_time`.
    /// Set to `None` after a long idle (or first launch) so the next
    /// frame produces `dt = 0` instead of catching up on lost time. Each
    /// active frame clamps `(now - last).min(33ms)` before accumulating.
    pub(super) last_anim_tick: Option<std::time::Instant>,
}

#[derive(Default)]
pub(super) struct ScanProgress {
    pub files: u64,
    pub dirs: u64,
    pub bytes: u64,
    pub errors: u64,
    pub scanning: bool,
    /// Which backend this run uses (shown in status bar / title).
    pub scan_engine_label: Option<String>,
    pub error: Option<String>,
    pub start_time: Option<std::time::Instant>,
    pub elapsed_secs: f32,
}

pub(super) struct HoverInfo {
    pub path: String,
    pub size: u64,
}

impl Default for App {
    fn default() -> Self {
        Self {
            events: EventBus::new(),
            scan_path: String::new(),
            scan_rx: None,
            scan_cancel: None,
            tree: None,
            filtered_tree: None,
            treemap_tex: None,
            last_render_size: (0, 0),
            opts: TreeMapOptions::default(),
            progress: ScanProgress::default(),
            hovered: None,
            last_wheel_zoom: std::time::Instant::now(),
            selected_path: None,
            scroll_to_selected: false,
            show_settings: true,
            expanded: std::collections::HashSet::new(),
            needs_layout: false,
            needs_render_3d: false,
            dark_mode: true,
            scanner_mode: ScannerMode::Standard,
            ctx_menu_path: None,
            ctx_menu_pos: None,
            pending_trash_path: None,
            ext_stats: Vec::new(),
            zoom_path: None,
            path_history: Vec::new(),
            filter_min: 0,
            filter_max: u64::MAX,
            filter_invert: false,
            filter_merge_outside: false,
            lod_expanded_paths: HashSet::new(),
            scan_min_size: 0,
            scan_max_size: 0,
            needs_filter_rebuild: false,
            filter_auto_rebuild: true,
            filter_changed_at: None,
            ext_sort: (1, false),
            search_text: String::new(),
            show_search: false,
            ext_search_text: String::new(),
            ext_filter: Vec::new(),
            ext_filter_invert: false,
            settings_tint_mix: default_settings_tint_mix(),
            settings_section_header_height: default_settings_section_header_height(),
            settings_panel_font_body: default_settings_panel_font_body(),
            settings_panel_font_heading: default_settings_panel_font_heading(),
            settings_panel_font_subheading: default_settings_panel_font_subheading(),
            settings_panel_font_small: default_settings_panel_font_small(),
            settings_panel_font_button: default_settings_panel_font_button(),
            settings_panel_font_monospace: default_settings_panel_font_monospace(),
            dock_state: crate::app::dock::default_dock_state(),
            tree_panel_width: 200.0,
            settings_panel_width: 280.0,
            cache_age: None,
            show_free_space: false,
            display_tree_cache: None,
            exclusions: Exclusions::new(""),
            show_excluded: false,
            viewport: Viewport::default(),
            render_backend: RenderBackend::default(),
            render_mode: RenderMode::Mode3D,
            render_3d_opts: super::presets::factory_render_3d_options(),
            presets: super::presets::load_all_presets(),
            preset_name: super::presets::DEFAULT_PRESET_NAME.to_string(),
            preset_dropdown_open: false,
            preset_autosave: false,
            autosave_interval_secs: default_autosave_interval(),
            preset_dirty: false,
            preset_last_save: std::time::Instant::now(),
            orbit_camera: OrbitCamera::default(),
            gpu_context: None,
            oidn_denoiser: None,
            oidn_run_requested: false,
            oidn_last_latency_ms: None,
            oidn_denoised_this_accumulation: false,
            oidn_last_frame_count: 0,
            oidn_display_is_denoised: false,
            oidn_last_display_was_denoised: false,
            oidn_last_interval_spp: 0,
            wgpu_render_state: None,
            renderer_2d_gpu: None,
            renderer_3d: None,
            render_texture_id: None,
            hovered_3d_id: 0,
            selected_3d_ids: std::collections::HashSet::new(),
            marquee_start: None,
            marquee_baseline: None,
            last_hover_pos_3d: None,
            last_pick_time_3d: std::time::Instant::now(),
            sticky_hover: None,
            frame_count: 0,
            file_mask_text: String::new(),
            use_file_mask: false,
            last_search_text: String::new(),
            last_file_mask_text: String::new(),
            last_use_file_mask: false,
            filtered_paths_cache: None,
            font_size: default_font_size(),
            settings_tab: SettingsTab::default(),
            screenshot_delay: None,
            screenshot_path: None,
            exit_after_screenshot: false,
            screenshot_start_time: None,
            screenshot_taken: false,
            last_render_frame_3d: 0,
            last_render_instant_3d: None,
            render_tick_3d: true,
            last_frame_ms: 0.0,
            last_fps: 0.0,
            frame_history: std::collections::VecDeque::with_capacity(256),
            avg_frame_ms: 0.0,
            avg_fps: 0.0,
            last_samples_per_sec: 0.0,
            last_samples_per_frame: 0,
            mem_used_mb: 0,
            mem_total_mb: 0,
            last_mem_update: std::time::Instant::now(),
            sys: sysinfo::System::new(),
            wgpu_error_flag: Arc::new(AtomicBool::new(false)),
            pt_auto_spp_tick: std::time::Instant::now(),
            show_encode_panel: false,
            encode_dialog: media_encoder::EncodeDialog::load_from_settings(
                &media_encoder::EncodeDialogSettings::default(),
            ),
            encode_source: None,
            encode_sequence_source: None,
            encode_source_size: (0, 0),
            encode_active_frame: None,
            encode_render_state_active: false,
            encode_restore_render_mode: RenderMode::default(),
            encode_base_animation_time: 0.0,
            encode_base_env_time: 0.0,
            encode_restore_animate: false,
            encode_restore_env_animate: false,
            last_anim_tick: None,
        }
    }
}
