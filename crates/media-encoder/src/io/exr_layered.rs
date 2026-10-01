use std::path::Path;

use crate::io::IoError;

pub use vfx_core::{AttrValue, DataFormat};
pub use vfx_io::{ChannelData, ChannelKind, ChannelSamples, ImageLayer, LayeredImage, Metadata};

fn map_layers_err(e: vfx_io::IoError) -> IoError {
    IoError::Exr(e.to_string())
}

pub fn write_exr_layers(path: impl AsRef<Path>, layered: &LayeredImage) -> Result<(), IoError> {
    vfx_io::exr::write_layers(path.as_ref(), layered).map_err(map_layers_err)
}

pub fn read_exr_layers_passthrough(path: impl AsRef<Path>) -> Result<LayeredImage, IoError> {
    vfx_io::exr::read_layers_passthrough(path.as_ref()).map_err(map_layers_err)
}

pub fn write_exr_layers_passthrough(
    path: impl AsRef<Path>,
    layered: &LayeredImage,
) -> Result<(), IoError> {
    vfx_io::exr::write_layers_passthrough(path.as_ref(), layered).map_err(map_layers_err)
}
