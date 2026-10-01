//! LED color calibration for OmaAura.
//!
//! Monitors are sRGB emissive surfaces: a "pastel" color (e.g. Catppuccin's
//! red `#ed8796`, saturation ~0.43) still reads clearly as *red* on screen.
//!
//! Addressable LEDs (ARGB strips, Aura headers) behave differently: they run at
//! a much higher luminance and mix additively, so a low-saturation color bleeds
//! into neighbouring channels and is perceived as *washed-out white with a hint
//! of color*.
//!
//! To make the physical LED match the widget swatch we normalise every color
//! before writing it to hardware: clamp **saturation** to a floor (pastel ->
//! vivid) and drive **value** to full brightness.
//!
//! The transform is **idempotent** (floors, not multipliers), so it can run
//! twice (UI preview + apply time) without drifting.

/// A color below this saturation is considered a grey/white and is emitted as
/// pure white (there is no hue worth preserving).
pub const GREY_SATURATION: f64 = 0.15;

pub const DEFAULT_SATURATION_FLOOR: f64 = 1.0;
pub const DEFAULT_VALUE_TARGET: f64 = 1.0;
pub const DEFAULT_FALLBACK: &str = "89b4fa";

/// Return normalised `(r, g, b)` floats in the 0.0-1.0 range.
pub fn parse_hex(hex_code: &str) -> Result<(f64, f64, f64), String> {
    let clean = hex_code.trim().trim_start_matches('#');
    if clean.len() != 6 {
        return Err(format!("Invalid hex color: {hex_code:?}"));
    }
    let byte = |s: &str| u8::from_str_radix(s, 16).map_err(|_| format!("Invalid hex color: {hex_code:?}"));
    Ok((
        f64::from(byte(&clean[0..2])?) / 255.0,
        f64::from(byte(&clean[2..4])?) / 255.0,
        f64::from(byte(&clean[4..6])?) / 255.0,
    ))
}

/// RGB (0..1) -> HSV (h, s, v) in 0..1.
#[must_use]
pub fn rgb_to_hsv(r: f64, g: f64, b: f64) -> (f64, f64, f64) {
    let max = r.max(g).max(b);
    let min = r.min(g).min(b);
    let delta = max - min;

    let h = if delta.abs() < f64::EPSILON {
        0.0
    } else if (max - r).abs() < f64::EPSILON {
        (g - b) / delta % 6.0
    } else if (max - g).abs() < f64::EPSILON {
        (b - r) / delta + 2.0
    } else {
        (r - g) / delta + 4.0
    };
    let h = if h < 0.0 { h + 6.0 } else { h } / 6.0;
    let s = if max.abs() < f64::EPSILON { 0.0 } else { delta / max };
    (h, s, max)
}

/// HSV (h, s, v) in 0..1 -> RGB (0..1).
#[must_use]
pub fn hsv_to_rgb(h: f64, s: f64, v: f64) -> (f64, f64, f64) {
    if s.abs() < f64::EPSILON {
        return (v, v, v);
    }
    let i = (h * 6.0).floor();
    let f = h * 6.0 - i;
    let p = v * (1.0 - s);
    let q = v * (1.0 - f * s);
    let t = v * (1.0 - (1.0 - f) * s);
    match (i as i64).rem_euclid(6) {
        0 => (v, t, p),
        1 => (q, v, p),
        2 => (p, v, t),
        3 => (p, q, v),
        4 => (t, p, v),
        _ => (v, p, q),
    }
}

/// Calibrate a hex color for LED hardware, returning `rrggbb` (no `#`).
///
/// Idempotent: `calibrate_hex(calibrate_hex(x)) == calibrate_hex(x)`.
#[must_use]
pub fn calibrate_hex(
    hex_code: &str,
    saturation_floor: f64,
    value_target: f64,
    fallback: &str,
) -> String {
    let Ok((r, g, b)) = parse_hex(hex_code) else {
        return fallback.trim_start_matches('#').to_ascii_lowercase();
    };

    let (h, s, v) = rgb_to_hsv(r, g, b);
    if s < GREY_SATURATION {
        return "ffffff".to_string();
    }

    let s = s.max(saturation_floor).clamp(0.0, 1.0);
    let v = v.max(value_target).clamp(0.0, 1.0);

    let (rf, gf, bf) = hsv_to_rgb(h, s, v);
    format!(
        "{:02x}{:02x}{:02x}",
        (rf * 255.0).round() as u8,
        (gf * 255.0).round() as u8,
        (bf * 255.0).round() as u8
    )
}

/// Same as [`calibrate_hex`] but returns a `#rrggbb` string.
#[must_use]
pub fn calibrate_hash(
    hex_code: &str,
    saturation_floor: f64,
    value_target: f64,
    fallback: &str,
) -> String {
    format!("#{}", calibrate_hex(hex_code, saturation_floor, value_target, fallback))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn saturation_floor_makes_colors_vivid() {
        // With floor=1.0 every color becomes fully saturated (min channel 0,
        // max channel 255) so pastels stop looking like "white with a hint".
        for hex in ["#ed8796", "#89b4fa", "#a6e3a1", "#f9e2af"] {
            let out = calibrate_hex(hex, 1.0, 1.0, "89b4fa");
            let (r, g, b) = parse_hex(&out).unwrap();
            let channels = [r, g, b];
            let min = channels.iter().copied().fold(f64::INFINITY, f64::min);
            let max = channels.iter().copied().fold(f64::NEG_INFINITY, f64::max);
            assert!(min < 0.01, "min channel must be drained for {hex}: {out}");
            assert!(max > 0.99, "max channel must be full for {hex}: {out}");
        }
    }

    #[test]
    fn grey_goes_white() {
        assert_eq!(calibrate_hex("#060707", 1.0, 1.0, "89b4fa"), "ffffff");
    }

    #[test]
    fn idempotent() {
        let once = calibrate_hex("#ed8796", 1.0, 1.0, "89b4fa");
        let twice = calibrate_hex(&once, 1.0, 1.0, "89b4fa");
        assert_eq!(once, twice);
    }

    #[test]
    fn invalid_falls_back() {
        assert_eq!(calibrate_hex("nope", 1.0, 1.0, "89b4fa"), "89b4fa");
    }
}
