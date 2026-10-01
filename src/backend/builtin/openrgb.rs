//! Built-in backend: OpenRGB (`openrgb` CLI) for motherboards, RAM, ARGB
//! headers and other OpenRGB-supported devices.
//!
//! **Safety invariant:** this backend *never* runs `openrgb -c` without `-d`.
//! If the non-GPU target set cannot be resolved, the write is skipped instead
//! of hitting every device (which would include the GPU driven over I2C).

use std::path::PathBuf;
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

use anyhow::{Result, bail};
use serde_json::{Value as Json, json};

use crate::backend::{ApplyOutcome, Backend, Color, DetectedDevice, DeviceKind};
use crate::config;

/// Locate an executable in `PATH`.
fn which(cmd: &str) -> Option<PathBuf> {
    let path = std::env::var_os("PATH")?;
    std::env::split_paths(&path).map(|dir| dir.join(cmd)).find(|p| p.is_file())
}

#[must_use]
pub fn is_openrgb_available() -> bool {
    which("openrgb").is_some()
}

fn is_gpu_device(name: &str) -> bool {
    regex::Regex::new(r"(?i)geforce|radeon|\brtx\b|\bgtx\b|\brx\s?\d|nvidia|quadro|arc\s+a\d|intel arc")
        .is_ok_and(|re| re.is_match(name))
}

/// Detected OpenRGB device.
#[derive(Debug, Clone)]
pub struct OpenRgbDevice {
    pub id: i64,
    pub name: String,
}

/// List OpenRGB devices via `openrgb --list-devices`.
#[must_use]
pub fn detect_openrgb_devices() -> Vec<OpenRgbDevice> {
    if !is_openrgb_available() {
        return Vec::new();
    }
    let Ok(output) = Command::new("openrgb").arg("--list-devices").stderr(Stdio::null()).output() else {
        return Vec::new();
    };
    let text = String::from_utf8_lossy(&output.stdout);
    let Ok(re) = regex::Regex::new(r"^(\d+):\s*(.+)$") else {
        return Vec::new();
    };
    text.lines()
        .filter_map(|line| {
            let caps = re.captures(line.trim())?;
            Some(OpenRgbDevice { id: caps[1].parse().ok()?, name: caps[2].trim().to_string() })
        })
        .collect()
}

const TARGETS_TTL_SEC: f64 = 1800.0;

fn targets_cache_file() -> PathBuf {
    config::state_dir().join("openrgb_targets.json")
}

fn read_targets_cache() -> Option<Vec<i64>> {
    let text = std::fs::read_to_string(targets_cache_file()).ok()?;
    let data: Json = serde_json::from_str(&text).ok()?;
    let now = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).ok()?.as_secs_f64();
    if now - data.get("ts")?.as_f64()? > TARGETS_TTL_SEC {
        return None;
    }
    let ids: Vec<i64> = data.get("ids")?.as_array()?.iter().filter_map(Json::as_i64).collect();
    (!ids.is_empty()).then_some(ids)
}

fn write_targets_cache(ids: &[i64]) {
    let Ok(now) = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH) else {
        return;
    };
    let path = targets_cache_file();
    if let Some(parent) = path.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    let _ = std::fs::write(path, json!({ "ts": now.as_secs_f64(), "ids": ids }).to_string());
}

/// Which OpenRGB devices to target, derived from the backend's config table.
#[derive(Debug, Clone)]
pub enum DeviceIds {
    /// `devices = "all"` (or absent) -> every *non-GPU* device.
    All,
    /// `devices = [0, 1]` -> explicit list.
    List(Vec<i64>),
}

impl DeviceIds {
    #[must_use]
    pub fn from_params(params: &toml::Value) -> Self {
        match params.get("devices") {
            Some(toml::Value::Array(items)) => {
                Self::List(items.iter().filter_map(toml::Value::as_integer).collect())
            }
            _ => Self::All,
        }
    }
}

/// Resolve target device IDs. `All` maps to "every non-GPU device"; `None` when
/// the set cannot be determined (caller must **not** fall back to a filterless write).
#[must_use]
pub fn resolve_targets(device_ids: &DeviceIds) -> Option<Vec<i64>> {
    match device_ids {
        DeviceIds::List(ids) => Some(ids.clone()),
        DeviceIds::All => {
            if let Some(cached) = read_targets_cache() {
                return Some(cached);
            }
            let devices = detect_openrgb_devices();
            let non_gpu: Vec<i64> = devices.iter().filter(|d| !is_gpu_device(&d.name)).map(|d| d.id).collect();
            if !devices.is_empty() && !non_gpu.is_empty() {
                write_targets_cache(&non_gpu);
                return Some(non_gpu);
            }
            None
        }
    }
}

fn run_openrgb(args: &[&str], timeout: Duration) -> bool {
    let mut child = match Command::new("openrgb").args(args).stdout(Stdio::null()).stderr(Stdio::null()).spawn() {
        Ok(child) => child,
        Err(_) => return false,
    };
    let deadline = Instant::now() + timeout;
    loop {
        match child.try_wait() {
            Ok(Some(status)) => {
                if !status.success() {
                    eprintln!("Warning: openrgb {} exited with {:?}", args.join(" "), status.code());
                    return false;
                }
                return true;
            }
            Ok(None) if Instant::now() >= deadline => {
                let _ = child.kill();
                let _ = child.wait();
                eprintln!("Warning: openrgb {} timed out", args.join(" "));
                return false;
            }
            Ok(None) => std::thread::sleep(Duration::from_millis(50)),
            Err(_) => return false,
        }
    }
}

/// Write a color, restricted to non-GPU devices. Never calls a filterless write.
pub fn set_openrgb_color(hex_str: &str, device_ids: &DeviceIds) -> Result<ApplyOutcome> {
    if !is_openrgb_available() {
        return Ok(ApplyOutcome::skipped("OpenRGB not installed"));
    }
    let clean = hex_str.trim().trim_start_matches('#');
    if clean.len() != 6 {
        bail!("Invalid hex color string: '{hex_str}'");
    }

    let Some(targets) = resolve_targets(device_ids) else {
        return Ok(ApplyOutcome::skipped(
            "could not resolve non-GPU OpenRGB targets; refusing to touch all devices (would hit the GPU)",
        ));
    };
    if targets.is_empty() {
        return Ok(ApplyOutcome::skipped("no OpenRGB targets"));
    }

    let mut applied = true;
    for dev_id in &targets {
        applied &= run_openrgb(&["-d", &dev_id.to_string(), "-c", clean], Duration::from_secs(25));
    }
    Ok(if applied {
        ApplyOutcome::ok(targets.len())
    } else {
        ApplyOutcome { applied: false, devices: targets.len(), message: Some("one or more OpenRGB writes failed".into()) }
    })
}

/// OpenRGB backend.
#[derive(Default)]
pub struct OpenRgbBackend;

impl Backend for OpenRgbBackend {
    fn id(&self) -> String {
        "openrgb".to_string()
    }

    fn name(&self) -> String {
        "OpenRGB".to_string()
    }

    fn kinds(&self) -> Vec<DeviceKind> {
        vec![
            DeviceKind::Motherboard,
            DeviceKind::Ram,
            DeviceKind::Keyboard,
            DeviceKind::Mouse,
            DeviceKind::Light,
            DeviceKind::Generic,
        ]
    }

    fn detect(&self, enabled: bool) -> Vec<DetectedDevice> {
        detect_openrgb_devices()
            .into_iter()
            .filter(|d| !is_gpu_device(&d.name))
            .map(|d| DetectedDevice {
                id: d.id.to_string(),
                name: d.name,
                kinds: vec![DeviceKind::Motherboard],
                backend: self.id(),
                enabled,
            })
            .collect()
    }

    fn apply(&self, color: &Color, params: &toml::Value) -> Result<ApplyOutcome> {
        set_openrgb_color(&color.hex, &DeviceIds::from_params(params))
    }

    fn off(&self, params: &toml::Value) -> Result<ApplyOutcome> {
        set_openrgb_color("000000", &DeviceIds::from_params(params))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn gpu_names_are_recognised() {
        assert!(is_gpu_device("NVIDIA GeForce RTX 5060 Ti"));
        assert!(is_gpu_device("ASUS Radeon RX 7900"));
        assert!(!is_gpu_device("ASUS ROG STRIX B550-F GAMING"));
    }

    #[test]
    fn explicit_list_is_honoured() {
        assert_eq!(resolve_targets(&DeviceIds::List(vec![0, 2])), Some(vec![0, 2]));
    }

    #[test]
    fn filterless_write_is_never_attempted() {
        if is_openrgb_available() {
            return;
        }
        let out = set_openrgb_color("ff0000", &DeviceIds::All).unwrap();
        assert!(!out.applied);
    }
}
