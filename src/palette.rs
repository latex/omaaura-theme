//! Palette generator for the Quickshell `BarWidget`.
//!
//! Produces JSON with:
//! * `theme`      -> the Omarchy theme palette (accent, red, …)
//! * `background` -> dominant/vibrant colors extracted from the wallpaper
//!
//! Every entry carries the **raw** source color as `hex` and the **calibrated**
//! LED color as `displayHex`, computed with the exact same calibration used when
//! writing to hardware — so the swatch the user clicks is the LED color.

use std::path::PathBuf;
use std::process::Command;

use serde_json::{Value as Json, json};
use toml::Value as TomlValue;

use crate::color::{self, DEFAULT_FALLBACK, DEFAULT_SATURATION_FLOOR, DEFAULT_VALUE_TARGET};
use crate::config::{self, get_f64, get_str};

fn calibration_params(cfg: &TomlValue) -> (f64, f64, String) {
    (
        get_f64(cfg, "theme", "saturation_floor", DEFAULT_SATURATION_FLOOR),
        get_f64(cfg, "theme", "value_target", DEFAULT_VALUE_TARGET),
        get_str(cfg, "theme", "fallback_color", DEFAULT_FALLBACK),
    )
}

fn preview(hex_code: &str, sf: f64, vt: f64, fb: &str) -> String {
    color::calibrate_hash(hex_code, sf, vt, fb)
}

fn entry(name: &str, hex_code: &str, sf: f64, vt: f64, fb: &str) -> Json {
    let hex = format!("#{}", hex_code.trim_start_matches('#').to_lowercase());
    json!({
        "name": name,
        "hex": hex,
        "displayHex": preview(&hex, sf, vt, fb),
    })
}

fn omarchy_state_dir() -> PathBuf {
    config::home().join(".local/state/omarchy/current")
}

fn theme_colors(sf: f64, vt: f64, fb: &str) -> Vec<Json> {
    let path = omarchy_state_dir().join("theme/colors.toml");
    let Ok(text) = std::fs::read_to_string(path) else {
        return Vec::new();
    };
    let Ok(data) = text.parse::<TomlValue>() else {
        return Vec::new();
    };

    const KEYS: [&str; 8] = ["accent", "red", "orange", "yellow", "green", "cyan", "blue", "magenta"];
    KEYS.iter()
        .filter_map(|k| {
            let v = data.get(*k)?.as_str()?;
            v.starts_with('#').then(|| entry(k, v, sf, vt, fb))
        })
        .collect()
}

struct HistEntry {
    hex: String,
    count: u64,
    s: f64,
    v: f64,
}

/// Quantise the current wallpaper to 16 colors and return the histogram.
fn wallpaper_histogram() -> Vec<HistEntry> {
    let bg_path = omarchy_state_dir().join("background");
    let Ok(real_bg) = std::fs::canonicalize(&bg_path) else {
        return Vec::new();
    };

    let output = Command::new("magick")
        .arg(&real_bg)
        .args(["-scale", "64x64!", "-depth", "8", "+dither", "-colors", "16", "-format", "%c", "histogram:info:"])
        .output();
    let Ok(output) = output else {
        return Vec::new();
    };
    let text = String::from_utf8_lossy(&output.stdout);

    let mut entries = Vec::new();
    for line in text.lines() {
        let parts: Vec<&str> = line.split_whitespace().collect();
        if parts.len() < 3 || !parts[2].starts_with('#') {
            continue;
        }
        let hex = parts[2].get(1..7).unwrap_or("").to_lowercase();
        if hex.len() != 6 {
            continue;
        }
        let count = parts[0].trim_end_matches(':').parse::<u64>().unwrap_or(0);
        let Some((r, g, b)) = hex_channels(&hex) else {
            continue;
        };
        let (_, s, v) = color::rgb_to_hsv(r, g, b);
        entries.push(HistEntry { hex: format!("#{hex}"), count, s, v });
    }
    entries
}

fn hex_channels(hex: &str) -> Option<(f64, f64, f64)> {
    Some((
        f64::from(u8::from_str_radix(hex.get(0..2)?, 16).ok()?) / 255.0,
        f64::from(u8::from_str_radix(hex.get(2..4)?, 16).ok()?) / 255.0,
        f64::from(u8::from_str_radix(hex.get(4..6)?, 16).ok()?) / 255.0,
    ))
}

fn background_colors(sf: f64, vt: f64, fb: &str) -> Vec<Json> {
    let entries = wallpaper_histogram();
    if entries.is_empty() {
        return Vec::new();
    }

    let mut out = Vec::new();

    // 1. Most frequent dominant color.
    let Some(dom) = entries.iter().max_by_key(|e| e.count) else {
        return out;
    };
    out.push(entry("Fundo Dominante", &dom.hex, sf, vt, fb));

    // 2. Vibrant accent of the scene.
    let vibrant = entries
        .iter()
        .filter(|e| e.s > 0.15 && e.v > 0.15)
        .max_by(|a, b| {
            let wa = a.count as f64 * a.s.powf(0.7) * a.v.powf(0.7);
            let wb = b.count as f64 * b.s.powf(0.7) * b.v.powf(0.7);
            wa.partial_cmp(&wb).unwrap_or(std::cmp::Ordering::Equal)
        });
    if let Some(vib) = vibrant
        && vib.hex != dom.hex {
            out.push(entry("Destaque Wallpaper", &vib.hex, sf, vt, fb));
        }

    // 3. Frequent secondary color.
    if out.len() < 3
        && let Some(sec) = entries.iter().filter(|e| e.hex != dom.hex).max_by_key(|e| e.count) {
            out.push(entry("Secundária Wallpaper", &sec.hex, sf, vt, fb));
        }

    out
}

/// Build the full palette payload.
#[must_use]
pub fn get_palette(cfg: &TomlValue) -> Json {
    let (sf, vt, fb) = calibration_params(cfg);
    let fb = fb.as_str();
    json!({
        "theme": theme_colors(sf, vt, fb),
        "background": background_colors(sf, vt, fb),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn entry_carries_raw_and_display_hex() {
        let e = entry("accent", "#89b4fa", 1.0, 1.0, "89b4fa");
        assert_eq!(e["hex"], "#89b4fa");
        assert!(e["displayHex"].as_str().unwrap().starts_with('#'));
    }

    #[test]
    fn empty_when_no_omarchy_state() {
        // In CI there is no ~/.local/state/omarchy; must degrade to empty, no panic.
        let cfg = config::default_config();
        let p = get_palette(&cfg);
        assert!(p["theme"].is_array());
        assert!(p["background"].is_array());
    }
}
