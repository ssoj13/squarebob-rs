//! Physical (photographer-style) camera model.
//!
//! Lives next to the manual camera knobs (`pt_aperture` etc.) in
//! [`Render3DOptions`](crate::Render3DOptions) and is selected via the
//! [`CameraType`] enum. When `CameraType::Physical` is active, the
//! render path reads derived values (`aperture_world`, `fov_radians`,
//! `exposure_multiplier`) instead of the manual fields.
//!
//! Conventions:
//! * Focal length & sensor width in millimetres (industry standard).
//! * Shutter in seconds (not "1/N" — easier serialisation).
//! * ISO in 100..6400 typical, sliders go higher for stylised effects.
//! * `exposure_compensation_ev` in stops, applied on top of the
//!   computed EV100.

use serde::{Deserialize, Serialize};

/// Which camera model the renderer reads from. `Manual` keeps the
/// legacy raw-aperture / orbit-fov pair untouched; `Physical` swaps
/// in derived values from [`PhysicalCamera`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
pub enum CameraType {
    #[default]
    Physical,
    Manual,
}

/// Photographer-style camera parameters.
///
/// Defaults mirror a "50mm f/5.6 at ISO 100, 1/125s, EV0" still — a
/// neutral starting point that produces in-the-ballpark exposure for
/// most HDR scenes.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct PhysicalCamera {
    // === L1 — lens ===
    /// `f/N` — controls both aperture (depth-of-field) and exposure
    /// denominator. Lower N = wider aperture = more blur + brighter.
    pub f_number: f32,
    /// Lens focal length in millimetres. Defines the field of view in
    /// combination with [`Self::sensor_width_mm`].
    pub focal_length_mm: f32,
    /// Camera sensor / film width in millimetres. 36mm = 35mm full
    /// frame (default), 24mm = APS-C, 17.3mm = micro four-thirds.
    pub sensor_width_mm: f32,

    // === L2 — exposure ===
    /// Sensor sensitivity. Typical photographic range 100..6400.
    pub iso: f32,
    /// Shutter speed in seconds. e.g. `1.0 / 125.0` = 1/125s.
    pub shutter_seconds: f32,
    /// Manual exposure compensation in stops (EV). Applied on top of
    /// the computed `EV100`; positive values brighten the result.
    pub exposure_compensation_ev: f32,

    // === L3 — character (filled in later phases) ===
    /// Vignetting strength `[0, 1]`. `0` disables.
    pub vignetting: f32,
    /// Chromatic aberration strength `[0, 1]`. `0` disables.
    pub chromatic_aberration: f32,
    /// Aperture blade count for bokeh shape. `0` = perfect disk,
    /// `5..8` = polygonal bokeh.
    pub bokeh_blades: u32,
    /// Anamorphic ratio. `1.0` = circular bokeh, `>1.0` = horizontally
    /// squeezed (vertical bokeh ellipses).
    pub bokeh_anamorphic: f32,
    /// Brown-Conrady lens distortion `k1` coefficient. Negative =
    /// barrel, positive = pincushion. `0` disables.
    pub lens_distortion: f32,
}

impl Default for PhysicalCamera {
    fn default() -> Self {
        // Defaults tuned for the squarebob treemap scene scale: a
        // bright f/1.4 85mm with ISO 300 + 1s shutter lands the
        // exposure multiplier slightly above unity (~1.3×), which
        // keeps the first frame visible instead of crushing emissive
        // surfaces into the noise floor. EV100 ≈ -0.6.
        Self {
            f_number: 1.4,
            focal_length_mm: 85.0,
            sensor_width_mm: 36.0,
            iso: 300.0,
            shutter_seconds: 1.0,
            exposure_compensation_ev: 0.0,
            vignetting: 0.0,
            chromatic_aberration: 0.0,
            bokeh_blades: 0,
            bokeh_anamorphic: 1.0,
            lens_distortion: 0.0,
        }
    }
}

impl PhysicalCamera {
    /// World-space aperture radius (matches the unit conventions of
    /// the existing `pt_aperture` field) — derived from focal length
    /// and f-number. Uses the standard thin-lens relation
    /// `radius = (focal_length / 2N)` converted to metres.
    pub fn aperture_world(&self) -> f32 {
        let f = self.focal_length_mm.max(0.001);
        let n = self.f_number.max(0.001);
        (f * 0.001) / (2.0 * n)
    }

    /// Horizontal field-of-view in radians, derived from sensor width
    /// and focal length. `fov = 2 * atan(W / 2f)`.
    pub fn fov_radians(&self) -> f32 {
        let f = self.focal_length_mm.max(0.001);
        let w = self.sensor_width_mm.max(0.001);
        2.0 * (0.5 * w / f).atan()
    }

    /// EV100 — exposure value referenced to ISO 100 — using the
    /// standard formula `EV = log2(N² / t · 100 / S)`. Lower EV =
    /// brighter scene; higher EV = darker scene. Exposure compensation
    /// shifts the result down by the same number of stops, matching
    /// camera EC behaviour (`+EV` brightens the image).
    pub fn ev100(&self) -> f32 {
        let n2 = self.f_number * self.f_number;
        let t = self.shutter_seconds.max(1e-6);
        let s = self.iso.max(1.0);
        let raw = ((n2 / t) * (100.0 / s)).log2();
        raw - self.exposure_compensation_ev
    }

    /// Scene-linear multiplier converting world radiance into a
    /// display-mid-gray-referenced value. `1 / (1.2 · 2^EV)` is the
    /// standard relation used by Frostbite / Filament / OCIO physical
    /// camera implementations.
    pub fn exposure_multiplier(&self) -> f32 {
        1.0 / (1.2 * 2.0_f32.powf(self.ev100()))
    }
}

/// Common photographic focal-length presets, in millimetres. Used by
/// the Camera UI for one-click recall.
pub const FOCAL_LENGTH_PRESETS_MM: &[f32] = &[14.0, 24.0, 35.0, 50.0, 85.0, 135.0, 200.0];

/// Common f-number presets (one-stop spacing). Used by the Camera UI.
pub const F_NUMBER_PRESETS: &[f32] = &[1.4, 2.0, 2.8, 4.0, 5.6, 8.0, 11.0, 16.0, 22.0];

/// Common sensor widths in millimetres.
pub const SENSOR_WIDTH_PRESETS_MM: &[(f32, &str)] = &[
    (36.0, "FF (35mm)"),
    (28.0, "APS-H"),
    (23.6, "APS-C"),
    (22.3, "APS-C Canon"),
    (17.3, "Micro 4/3"),
    (13.2, "1\" type"),
];
