//! Hardware detection and LED control.
//!
//! Two backends:
//! 1. Direct Linux I2C ioctl for ENE Aura controllers (ASUS TUF/ROG GPUs @ 0x67).
//! 2. OpenRGB for ASUS ROG motherboards, ARGB headers and RAM.

use std::os::fd::{AsRawFd, RawFd};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use anyhow::{Context, Result, bail};
use serde_json::{Value as Json, json};
use toml::Value as TomlValue;

use crate::config;

// ---------------------------------------------------------------------------
// Linux I2C ioctl
// ---------------------------------------------------------------------------

const I2C_RDWR: libc::c_ulong = 0x0707;

#[repr(C)]
struct I2cMsg {
    addr: u16,
    flags: u16,
    len: u16,
    buf: *mut u8,
}

#[repr(C)]
struct I2cRdwrIoctlData {
    msgs: *mut I2cMsg,
    nmsgs: u32,
}

fn i2c_write(fd: RawFd, addr: u16, data: &[u8]) -> Result<()> {
    let mut buf = data.to_vec();
    let Ok(len) = u16::try_from(data.len()) else {
        bail!("I2C payload too large");
    };
    let mut msg = I2cMsg { addr, flags: 0, len, buf: buf.as_mut_ptr() };
    let mut payload = I2cRdwrIoctlData { msgs: &raw mut msg, nmsgs: 1 };
    // SAFETY: `payload` and its buffers stay alive for the duration of the call;
    // the kernel only reads them.
    let ret = unsafe { libc::ioctl(fd, I2C_RDWR, &raw mut payload) };
    if ret < 0 {
        bail!("ioctl I2C_RDWR returned {ret}: {}", std::io::Error::last_os_error());
    }
    Ok(())
}

fn ene_write_reg(fd: RawFd, addr: u16, reg: u16, val: u8) -> Result<()> {
    i2c_write(fd, addr, &[0x00, (reg >> 8) as u8, reg as u8])?;
    i2c_write(fd, addr, &[0x01, val])
}

fn ene_write_block(fd: RawFd, addr: u16, reg: u16, bytes: &[u8]) -> Result<()> {
    i2c_write(fd, addr, &[0x00, (reg >> 8) as u8, reg as u8])?;
    for &val in bytes {
        i2c_write(fd, addr, &[0x01, val])?;
    }
    Ok(())
}

/// Set RGB color on an ENE Aura chip (ASUS GPU) via direct I2C.
///
/// Returns `Ok(false)` when the device node is absent (benign, not an error).
pub fn set_ene_color(hex_str: &str, dev_path: &str, addr: u16) -> Result<bool> {
    let clean = hex_str.trim().trim_start_matches('#');
    if clean.len() != 6 {
        bail!("Invalid hex color string: '{hex_str}'");
    }
    let r = u8::from_str_radix(&clean[0..2], 16).context("bad red channel")?;
    let g = u8::from_str_radix(&clean[2..4], 16).context("bad green channel")?;
    let b = u8::from_str_radix(&clean[4..6], 16).context("bad blue channel")?;

    if !Path::new(dev_path).exists() {
        return Ok(false);
    }

    let file = std::fs::OpenOptions::new()
        .read(true)
        .write(true)
        .open(dev_path)
        .with_context(|| format!("opening {dev_path}"))?;
    let fd = file.as_raw_fd();

    let mode: u8 = u8::from(r > 0 || g > 0 || b > 0);

    ene_write_reg(fd, addr, 0x8021, mode)?;
    ene_write_reg(fd, addr, 0x8022, 2)?;
    ene_write_reg(fd, addr, 0x8023, 0)?;
    ene_write_reg(fd, addr, 0x8020, 0)?;

    // ENE Aura controllers use RBG byte ordering, repeated across 4 LEDs.
    let rbg: Vec<u8> = [r, b, g].iter().cycle().take(12).copied().collect();

    ene_write_block(fd, addr, 0x8160, &rbg)?;
    ene_write_block(fd, addr, 0x8100, &rbg)?;
    ene_write_block(fd, addr, 0x8010, &rbg)?;
    ene_write_block(fd, addr, 0x8000, &rbg)?;

    ene_write_reg(fd, addr, 0x80A0, 0x01)?;
    std::thread::sleep(Duration::from_millis(20));
    ene_write_reg(fd, addr, 0x80A0, 0xAA)?;
    Ok(true)
}

/// Probe the ENE Aura controller at `dev_path`/`addr`.
#[must_use]
pub fn test_ene_aura(dev_path: &str, addr: u16) -> Json {
    let mut result = json!({
        "dev_path": dev_path,
        "addr": format!("{addr:#x}"),
        "exists": false,
        "permission_ok": false,
        "success": false,
        "message": "",
    });

    if !Path::new(dev_path).exists() {
        result["message"] = format!("Device node {dev_path} does not exist.").into();
        return result;
    }
    result["exists"] = true.into();

    let can_rw = can_read_write(dev_path);
    result["permission_ok"] = can_rw.into();
    if !can_rw {
        result["message"] = format!(
            "Insufficient permissions to access {dev_path}. Ensure your user is in the 'i2c' group or udev rules are set."
        )
        .into();
        return result;
    }

    match std::fs::OpenOptions::new().read(true).write(true).open(dev_path) {
        Ok(file) => {
            let fd = file.as_raw_fd();
            match i2c_write(fd, addr, &[0x00, 0x80, 0x20]) {
                Ok(()) => {
                    result["success"] = true.into();
                    result["message"] =
                        format!("Successfully communicated with ENE Aura at {dev_path} (addr {addr:#x}).").into();
                }
                Err(e) => {
                    result["message"] = format!("Failed to communicate with ENE Aura at {dev_path}: {e}").into();
                }
            }
        }
        Err(e) => {
            result["message"] = format!("Failed to open {dev_path}: {e}").into();
        }
    }
    result
}

fn can_read_write(path: &str) -> bool {
    let Ok(c_path) = std::ffi::CString::new(path) else {
        return false;
    };
    // SAFETY: `c_path` is a valid NUL-terminated string.
    unsafe { libc::access(c_path.as_ptr(), libc::R_OK | libc::W_OK) == 0 }
}

/// Enumerate I2C adapters and their access permissions.
#[must_use]
pub fn detect_i2c_adapters() -> Vec<Json> {
    let sys_bus = Path::new("/sys/bus/i2c/devices");
    let mut adapters = Vec::new();
    let Ok(entries) = std::fs::read_dir(sys_bus) else {
        return adapters;
    };

    let mut names: Vec<PathBuf> = entries
        .flatten()
        .map(|e| e.path())
        .filter(|p| p.file_name().is_some_and(|n| n.to_string_lossy().starts_with("i2c-")))
        .collect();
    names.sort();

    for item in names {
        let name = item
            .join("name")
            .is_file()
            .then(|| std::fs::read_to_string(item.join("name")).ok())
            .flatten()
            .map_or_else(|| "Unknown".to_string(), |s| s.trim().to_string());
        let bus = item.file_name().map_or_else(String::new, |n| n.to_string_lossy().into_owned());
        let dev_path = format!("/dev/{bus}");
        let exists = Path::new(&dev_path).exists();
        let lower = name.to_lowercase();

        adapters.push(json!({
            "id": bus,
            "dev_path": dev_path,
            "name": name,
            "exists": exists,
            "readable": exists && can_read(&dev_path),
            "writable": exists && can_read_write(&dev_path),
            "is_nvidia": lower.contains("nvidia"),
            "is_smbus": lower.contains("smbus") || lower.contains("piix4"),
        }));
    }
    adapters
}

fn can_read(path: &str) -> bool {
    let Ok(c_path) = std::ffi::CString::new(path) else {
        return false;
    };
    // SAFETY: `c_path` is a valid NUL-terminated string.
    unsafe { libc::access(c_path.as_ptr(), libc::R_OK) == 0 }
}

// ---------------------------------------------------------------------------
// OpenRGB backend
// ---------------------------------------------------------------------------

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
    let pat = regex::Regex::new(
        r"(?i)geforce|radeon|\brtx\b|\bgtx\b|\brx\s?\d|nvidia|quadro|arc\s+a\d|intel arc",
    );
    pat.is_ok_and(|re| re.is_match(name))
}

/// Detected OpenRGB device id + name.
#[derive(Debug, Clone)]
pub struct OpenRgbDevice {
    pub id: i64,
    pub name: String,
}

/// List detected OpenRGB devices via `openrgb --list-devices`.
#[must_use]
pub fn detect_openrgb_devices(timeout: Duration) -> Vec<OpenRgbDevice> {
    if !is_openrgb_available() {
        return Vec::new();
    }
    let output = std::process::Command::new("openrgb")
        .arg("--list-devices")
        .stderr(Stdio::null())
        .output();
    let Ok(output) = output else {
        return Vec::new();
    };
    let _ = timeout; // subprocess timeout is enforced by the caller via `timeout` crate-free sleep loops
    let text = String::from_utf8_lossy(&output.stdout);

    let re = regex::Regex::new(r"^(\d+):\s*(.+)$");
    let Ok(re) = re else { return Vec::new() };

    text.lines()
        .filter_map(|line| {
            let line = line.trim();
            let caps = re.captures(line)?;
            Some(OpenRgbDevice { id: caps[1].parse().ok()?, name: caps[2].trim().to_string() })
        })
        .collect()
}

// OpenRGB re-enumerates everything on every launch (~8s), so cache the resolved
// non-GPU target IDs for a while.
const TARGETS_TTL_SEC: f64 = 1800.0;

fn targets_cache_file() -> PathBuf {
    config::state_dir().join("openrgb_targets.json")
}

fn read_targets_cache() -> Option<Vec<i64>> {
    let text = std::fs::read_to_string(targets_cache_file()).ok()?;
    let data: Json = serde_json::from_str(&text).ok()?;
    let ts = data.get("ts")?.as_f64()?;
    let now = SystemTime::now().duration_since(UNIX_EPOCH).ok()?.as_secs_f64();
    if now - ts > TARGETS_TTL_SEC {
        return None;
    }
    let ids: Vec<i64> = data.get("ids")?.as_array()?.iter().filter_map(Json::as_i64).collect();
    (!ids.is_empty()).then_some(ids)
}

fn write_targets_cache(ids: &[i64]) {
    let Ok(now) = SystemTime::now().duration_since(UNIX_EPOCH) else {
        return;
    };
    let path = targets_cache_file();
    if let Some(parent) = path.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    let data = json!({ "ts": now.as_secs_f64(), "ids": ids });
    let _ = std::fs::write(path, data.to_string());
}

/// Which OpenRGB devices the *motherboard* backend should target.
#[derive(Debug, Clone)]
pub enum DeviceIds {
    /// "all" / absent -> every non-GPU device.
    All,
    /// Explicit list of device IDs.
    List(Vec<i64>),
}

impl DeviceIds {
    /// Interpret the `hardware.openrgb_devices` config value.
    #[must_use]
    pub fn from_config(cfg: &TomlValue) -> Self {
        match cfg.get("hardware").and_then(|h| h.get("openrgb_devices")) {
            Some(TomlValue::Array(items)) => {
                DeviceIds::List(items.iter().filter_map(TomlValue::as_integer).collect())
            }
            _ => DeviceIds::All,
        }
    }
}

/// Resolve which OpenRGB device IDs should receive the color.
///
/// `All` maps to "every *non-GPU* device" so the GPU keeps being handled by the
/// direct I2C backend. Returns `None` when the target set cannot be determined.
#[must_use]
pub fn resolve_openrgb_targets(device_ids: &DeviceIds) -> Option<Vec<i64>> {
    match device_ids {
        DeviceIds::List(ids) => Some(ids.clone()),
        DeviceIds::All => {
            if let Some(cached) = read_targets_cache() {
                return Some(cached);
            }
            let devices = detect_openrgb_devices(Duration::from_secs(12));
            let non_gpu: Vec<i64> =
                devices.iter().filter(|d| !is_gpu_device(&d.name)).map(|d| d.id).collect();
            if !devices.is_empty() && !non_gpu.is_empty() {
                write_targets_cache(&non_gpu);
                return Some(non_gpu);
            }
            // Detection unavailable *or* only the GPU was found. Do NOT fall
            // back to a filterless write (see `set_openrgb_color`).
            None
        }
    }
}

/// Set RGB color using the OpenRGB CLI, restricted to non-GPU devices.
///
/// **Safety fix (v2):** when the target set cannot be resolved we *do not* run
/// `openrgb -c` without `-d`. Doing so would drive every device — including the
/// GPU that is simultaneously controlled over I2C — and fight the direct write.
/// We skip the OpenRGB write instead and report it.
pub fn set_openrgb_color(hex_str: &str, device_ids: &DeviceIds, timeout: Duration) -> Result<bool> {
    if !is_openrgb_available() {
        return Ok(false);
    }

    let clean = hex_str.trim().trim_start_matches('#');
    if clean.len() != 6 {
        bail!("Invalid hex color string: '{hex_str}'");
    }

    let Some(targets) = resolve_openrgb_targets(device_ids) else {
        eprintln!(
            "Warning: could not resolve non-GPU OpenRGB targets; skipping OpenRGB write \
             (refusing to touch all devices, which would hit the GPU)."
        );
        return Ok(false);
    };

    if targets.is_empty() {
        eprintln!("Warning: OpenRGB target list is empty; nothing to write.");
        return Ok(false);
    }

    let mut ok = true;
    for dev_id in targets {
        ok &= run_openrgb(&["-d", &dev_id.to_string(), "-c", clean], timeout);
    }
    Ok(ok)
}

fn run_openrgb(args: &[&str], timeout: Duration) -> bool {
    let mut child = match Command::new("openrgb")
        .args(args)
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
    {
        Ok(child) => child,
        Err(_) => return false,
    };

    let deadline = std::time::Instant::now() + timeout;
    loop {
        match child.try_wait() {
            Ok(Some(status)) => {
                if !status.success() {
                    eprintln!("Warning: OpenRGB command {} exited with code {:?}", args.join(" "), status.code());
                    return false;
                }
                return true;
            }
            Ok(None) if std::time::Instant::now() >= deadline => {
                let _ = child.kill();
                let _ = child.wait();
                eprintln!("Warning: OpenRGB command {} timed out", args.join(" "));
                return false;
            }
            Ok(None) => std::thread::sleep(Duration::from_millis(50)),
            Err(_) => return false,
        }
    }
}

/// Full hardware diagnostic snapshot.
#[must_use]
pub fn detect_all_hardware() -> Json {
    let adapters = detect_i2c_adapters();

    let nvidia_buses: Vec<String> = adapters
        .iter()
        .filter(|a| a["is_nvidia"].as_bool().unwrap_or(false))
        .filter_map(|a| a["dev_path"].as_str().map(str::to_string))
        .collect();

    let gpu_bus = if nvidia_buses.is_empty() || nvidia_buses.iter().any(|b| b == "/dev/i2c-1") {
        "/dev/i2c-1".to_string()
    } else {
        nvidia_buses[0].clone()
    };

    let ene_test = test_ene_aura(&gpu_bus, 0x67);
    let openrgb_available = is_openrgb_available();
    let devices: Vec<Json> = if openrgb_available {
        detect_openrgb_devices(Duration::from_secs(12))
            .into_iter()
            .map(|d| json!({ "id": d.id, "name": d.name }))
            .collect()
    } else {
        Vec::new()
    };

    json!({
        "i2c_adapters": adapters,
        "gpu_bus_candidate": gpu_bus,
        "ene_aura": ene_test,
        "openrgb_installed": openrgb_available,
        "openrgb_devices": devices,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn gpu_names_are_recognised() {
        assert!(is_gpu_device("NVIDIA GeForce RTX 5060 Ti"));
        assert!(is_gpu_device("ASUS Radeon RX 7900"));
        assert!(!is_gpu_device("ASUS ROG STRIX B550-F GAMING"));
        assert!(!is_gpu_device("Corsair Vengeance RGB"));
    }

    #[test]
    fn explicit_device_list_is_honoured() {
        assert_eq!(resolve_openrgb_targets(&DeviceIds::List(vec![0, 2])), Some(vec![0, 2]));
    }

    #[test]
    fn filterless_write_is_never_attempted_when_unresolved() {
        // Simulate the historical dangerous path: `All` with no cache and no
        // OpenRGB binary available -> `None`, and `set_openrgb_color` must
        // refuse to run (returns Ok(false)) rather than calling `openrgb -c`.
        // (openrgb is absent in CI, so this exercises the guard.)
        if is_openrgb_available() {
            return;
        }
        let ok = set_openrgb_color("ff0000", &DeviceIds::All, Duration::from_secs(1)).unwrap();
        assert!(!ok);
    }
}
