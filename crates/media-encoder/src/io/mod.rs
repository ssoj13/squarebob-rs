#[derive(Debug)]
pub enum IoError {
    Exr(String),
    Image(String),
    LoadError(String),
    UnsupportedFormat(String),
}

impl std::fmt::Display for IoError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            IoError::Exr(s)
            | IoError::Image(s)
            | IoError::LoadError(s)
            | IoError::UnsupportedFormat(s) => f.write_str(s),
        }
    }
}

impl std::error::Error for IoError {}

#[cfg(feature = "exr")]
pub mod exr_layered;
