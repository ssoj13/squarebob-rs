pub mod dialogs;
pub mod frame;
pub mod io;
pub mod progress;
pub mod source;

#[cfg(feature = "ffmpeg")]
pub use playa_ffmpeg as ffmpeg;

pub use dialogs::encode::*;
pub use frame::*;
pub use source::*;

#[cfg(feature = "ffmpeg")]
pub fn init_ffmpeg() -> Result<(), Box<dyn std::error::Error>> {
    playa_ffmpeg::init().map_err(Box::<dyn std::error::Error>::from)
}

#[cfg(not(feature = "ffmpeg"))]
pub fn init_ffmpeg() -> Result<(), Box<dyn std::error::Error>> {
    Err("media-encoder was built without the ffmpeg feature".into())
}
