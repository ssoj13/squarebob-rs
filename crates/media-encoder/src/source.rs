use std::path::PathBuf;
use std::sync::Arc;

use crate::frame::Frame;

#[derive(Clone, Debug)]
pub struct ExrLayerInfo {
    pub marker: String,
    pub name: String,
    pub compression: String,
}

#[derive(Clone, Debug)]
pub struct ExrSourceInfo {
    pub path: PathBuf,
    pub layer_count: usize,
    pub layers: Vec<ExrLayerInfo>,
}

pub trait FrameSource: Send + Sync + std::fmt::Display {
    fn play_range(&self, clamp_to_available: bool) -> (i32, i32);
    fn get_frame(&self, frame_idx: i32, blocking: bool) -> Option<Frame>;

    fn exr_source_path(&self, _frame_idx: i32) -> Option<PathBuf> {
        None
    }

    fn exr_source_info(&self) -> Option<ExrSourceInfo> {
        None
    }
}

pub type Comp = Arc<dyn FrameSource>;

#[derive(Clone, Debug, Default)]
pub struct Project;
