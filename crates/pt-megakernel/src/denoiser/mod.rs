//! À-trous edge-aware denoiser for path-tracer output.
//!
//! Stage D.2 of the TODO4 roadmap. MVP: color-only edge stopping
//! (no G-buffer guidance). See `pipeline.rs` for the algorithm and
//! `atrous.wgsl` for the WGSL implementation.

pub mod pipeline;

pub use pipeline::DenoiserPipeline;
