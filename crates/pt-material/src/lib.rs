//! `pt-material` — per-scene material library for the squarebob PT.
//!
//! Replaces the previous `MaterialClass`-enum-driven discrete library
//! with a continuous, edit-in-place model. The data model is:
//!
//! * [`Material`] — a single material slot. Wraps a base
//!   [`standard_surface::StandardSurfaceParams`] with a parallel
//!   `variance` struct of the same layout; the variance fields are
//!   "± spread per attribute" applied per-cube.
//! * [`MaterialLibrary`] — the ordered collection of materials a
//!   scene exposes. Cubes carry a `material_index` (u32) into this
//!   library. JSON serialisable via serde.
//!
//! At materialise-time each cube hashes its instance id together with
//! the library entry's variance and produces a single resolved
//! `StandardSurfaceParams` via [`Material::resolve_for_cube`]. The
//! resolver is deterministic in the cube hash so re-renders match.
//!
//! Layout-wise, [`StandardSurfaceParams`] is GPU-ready (Pod+Zeroable,
//! same `vec4`-packed layout the WGSL `Material` struct expects), so
//! the resolved struct is uploaded directly as the per-cube GPU
//! material record — no separate `GpuMaterial` mirror needed.

pub mod material;
pub mod library;
pub mod io;
pub mod presets;

pub use library::MaterialLibrary;
pub use material::Material;
pub use standard_surface::StandardSurfaceParams;
