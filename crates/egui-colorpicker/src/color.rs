//! Color conversion math.
//!
//! All values flow through linear-sRGB internally. Conversions cover:
//! - linear ↔ sRGB EOTF / OETF (the common "gamma-corrected display
//!   value")
//! - linear ↔ HSV (V allowed > 1.0 for HDR)
//! - linear ↔ `#RRGGBB` hex strings (sRGB-encoded, clamped to LDR)

/// sRGB EOTF (display-encoded → linear). Per IEC 61966-2-1.
#[inline]
pub fn srgb_to_linear(x: f32) -> f32 {
    if x <= 0.04045 {
        x / 12.92
    } else {
        ((x + 0.055) / 1.055).powf(2.4)
    }
}

/// sRGB OETF (linear → display-encoded). Per IEC 61966-2-1.
#[inline]
pub fn linear_to_srgb(x: f32) -> f32 {
    if x <= 0.0031308 {
        x * 12.92
    } else {
        1.055 * x.powf(1.0 / 2.4) - 0.055
    }
}

/// Linear RGB → HSV. `V` mirrors the maximum channel so HDR colors
/// (any channel > 1.0) come back as `V > 1.0`. `H` is in degrees
/// `[0, 360)`, `S` in `[0, 1]`.
pub fn rgb_to_hsv(r: f32, g: f32, b: f32) -> (f32, f32, f32) {
    let max = r.max(g).max(b);
    let min = r.min(g).min(b);
    let delta = max - min;
    let v = max;
    let s = if v > 0.0 { delta / v } else { 0.0 };
    let h = if delta <= 0.0 {
        0.0
    } else if (max - r).abs() < f32::EPSILON {
        ((g - b) / delta).rem_euclid(6.0) * 60.0
    } else if (max - g).abs() < f32::EPSILON {
        ((b - r) / delta + 2.0) * 60.0
    } else {
        ((r - g) / delta + 4.0) * 60.0
    };
    (h.rem_euclid(360.0), s.clamp(0.0, 1.0), v.max(0.0))
}

/// HSV → linear RGB. `V > 1.0` propagates straight into the largest
/// channel, so the picker's HDR slider lane keeps working.
pub fn hsv_to_rgb(h: f32, s: f32, v: f32) -> (f32, f32, f32) {
    let s = s.clamp(0.0, 1.0);
    let v = v.max(0.0);
    let c = v * s;
    let h_p = h.rem_euclid(360.0) / 60.0;
    let x = c * (1.0 - (h_p.rem_euclid(2.0) - 1.0).abs());
    let (r1, g1, b1) = if h_p < 1.0 {
        (c, x, 0.0)
    } else if h_p < 2.0 {
        (x, c, 0.0)
    } else if h_p < 3.0 {
        (0.0, c, x)
    } else if h_p < 4.0 {
        (0.0, x, c)
    } else if h_p < 5.0 {
        (x, 0.0, c)
    } else {
        (c, 0.0, x)
    };
    let m = v - c;
    (r1 + m, g1 + m, b1 + m)
}

/// Linear RGB → `#RRGGBB`. HDR channels are sRGB-encoded then
/// clamped to `[0, 1]` and rounded to 8 bits — round-trip is lossy
/// past LDR by design (hex strings can't carry > 1.0).
pub fn linear_to_hex(r: f32, g: f32, b: f32) -> String {
    let q = |x: f32| {
        (linear_to_srgb(x).clamp(0.0, 1.0) * 255.0).round() as u8
    };
    format!("#{:02X}{:02X}{:02X}", q(r), q(g), q(b))
}

/// `#RRGGBB` / `RRGGBB` → linear RGB. Returns `None` when the
/// string isn't a 6-digit hex triplet.
pub fn hex_to_linear(hex: &str) -> Option<(f32, f32, f32)> {
    let trimmed = hex.trim().trim_start_matches('#');
    if trimmed.len() != 6 {
        return None;
    }
    let r = u8::from_str_radix(&trimmed[0..2], 16).ok()? as f32 / 255.0;
    let g = u8::from_str_radix(&trimmed[2..4], 16).ok()? as f32 / 255.0;
    let b = u8::from_str_radix(&trimmed[4..6], 16).ok()? as f32 / 255.0;
    Some((srgb_to_linear(r), srgb_to_linear(g), srgb_to_linear(b)))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn srgb_round_trip() {
        for x in [0.0, 0.1, 0.25, 0.5, 0.75, 1.0] {
            let v = srgb_to_linear(linear_to_srgb(x));
            assert!((v - x).abs() < 1e-4, "round trip failed for {x}");
        }
    }

    #[test]
    fn hsv_round_trip() {
        for &(r, g, b) in &[
            (0.5, 0.25, 0.75),
            (1.0, 0.0, 0.0),
            (0.0, 1.0, 0.0),
            (0.0, 0.0, 1.0),
            (1.2, 0.3, 2.5), // HDR
        ] {
            let (h, s, v) = rgb_to_hsv(r, g, b);
            let (rr, gg, bb) = hsv_to_rgb(h, s, v);
            assert!(
                (r - rr).abs() < 1e-4 && (g - gg).abs() < 1e-4 && (b - bb).abs() < 1e-4,
                "({r}, {g}, {b}) round-trip to ({rr}, {gg}, {bb})"
            );
        }
    }

    #[test]
    fn hex_round_trip_ldr() {
        let (r, g, b) = (0.5, 0.25, 0.75);
        let hex = linear_to_hex(r, g, b);
        let (rr, gg, bb) = hex_to_linear(&hex).unwrap();
        // 8-bit quantisation tolerance.
        assert!((r - rr).abs() < 0.01);
        assert!((g - gg).abs() < 0.01);
        assert!((b - bb).abs() < 0.01);
    }

    #[test]
    fn hex_parse_rejects_bad_inputs() {
        assert!(hex_to_linear("").is_none());
        assert!(hex_to_linear("#12345").is_none());
        assert!(hex_to_linear("#GGGGGG").is_none());
    }
}
