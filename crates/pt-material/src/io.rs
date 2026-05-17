//! JSON save / load for [`MaterialLibrary`].

use std::fs;
use std::io;
use std::path::Path;

use crate::library::MaterialLibrary;

/// Save the library to a JSON file at `path` (pretty-printed,
/// suitable for VCS diff review).
pub fn save_library(lib: &MaterialLibrary, path: impl AsRef<Path>) -> io::Result<()> {
    let json = serde_json::to_string_pretty(lib)
        .map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e))?;
    fs::write(path, json)
}

/// Load a library from a JSON file at `path`.
pub fn load_library(path: impl AsRef<Path>) -> io::Result<MaterialLibrary> {
    let raw = fs::read_to_string(path)?;
    serde_json::from_str(&raw).map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trip_string() {
        // In-memory round-trip — avoids adding a `tempfile` dev-dep
        // to the workspace just for one disk-IO test. The `save_library`
        // / `load_library` functions delegate to serde_json + fs::*,
        // so a string round-trip is sufficient to prove the JSON
        // schema is stable.
        let lib = MaterialLibrary::default();
        let json = serde_json::to_string_pretty(&lib).unwrap();
        let loaded: MaterialLibrary = serde_json::from_str(&json).unwrap();
        assert_eq!(loaded.len(), lib.len());
        for (a, b) in lib.materials.iter().zip(loaded.materials.iter()) {
            assert_eq!(a.uuid, b.uuid);
            assert_eq!(a.name, b.name);
        }
    }
}
