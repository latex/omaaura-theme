//! Built-in backend: ENE Aura GPU control via direct Linux I2C.
//!
//! Drives ASUS TUF/ROG GPUs (ENE Aura controller at `0x67`) by writing raw
//! `I2C_RDWR` ioctls to `/dev/i2c-*`. This is the only path that keeps the GPU
//! out of OpenRGB, so the two never fight over the same controller.

use std::os::fd::{AsRawFd, RawFd};
use std::path::Path;
use std::time::Duration;

use anyhow::{Context, Result, bail};
use serde_json::{Value as Json, json};

use crate::backend::{ApplyOutcome, Backend, Color, DetectedDevice, DeviceKind};

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
    // SAFETY: `payload` and its buffers outlive the call; the kernel only reads them.
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

/// Set RGB on an ENE Aura chip via direct I2C. `Ok(false)` when the device node
/// is absent (benign).
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

    ene_write_reg(fd, addr, 0x8021, u8::from(r > 0 || g > 0 || b > 0))?;
    ene_write_reg(fd, addr, 0x8022, 2)?;
    ene_write_reg(fd, addr, 0x8023, 0)?;
    ene_write_reg(fd, addr, 0x8020, 0)?;

    // ENE Aura uses RBG ordering, repeated across 4 LEDs.
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

/// Probe the ENE Aura controller. Returns a JSON report (used by diagnostics).
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

    if !can_read_write(dev_path) {
        result["message"] = format!(
            "Insufficient permissions to access {dev_path}. Ensure your user is in the 'i2c' group."
        )
        .into();
        return result;
    }

    match std::fs::OpenOptions::new().read(true).write(true).open(dev_path) {
        Ok(file) => match i2c_write(file.as_raw_fd(), addr, &[0x00, 0x80, 0x20]) {
            Ok(()) => {
                result["success"] = true.into();
                result["message"] =
                    format!("Successfully communicated with ENE Aura at {dev_path} (addr {addr:#x}).").into();
            }
            Err(e) => result["message"] = format!("Failed to communicate: {e}").into(),
        },
        Err(e) => result["message"] = format!("Failed to open {dev_path}: {e}").into(),
    }
    result
}

/// Enumerate I2C adapters (id, name, permissions, flags).
#[must_use]
pub fn detect_i2c_adapters() -> Vec<Json> {
    let Ok(entries) = std::fs::read_dir("/sys/bus/i2c/devices") else {
        return Vec::new();
    };
    let mut dirs: Vec<std::path::PathBuf> = entries
        .flatten()
        .map(|e| e.path())
        .filter(|p| p.file_name().is_some_and(|n| n.to_string_lossy().starts_with("i2c-")))
        .collect();
    dirs.sort();

    dirs.into_iter()
        .map(|item| {
            let bus = item.file_name().map_or_else(String::new, |n| n.to_string_lossy().into_owned());
            let name = std::fs::read_to_string(item.join("name"))
                .map_or_else(|_| "Unknown".to_string(), |s| s.trim().to_string());
            let dev_path = format!("/dev/{bus}");
            let exists = Path::new(&dev_path).exists();
            let lower = name.to_lowercase();
            json!({
                "id": bus,
                "dev_path": dev_path,
                "name": name,
                "exists": exists,
                "readable": exists && can_read(&dev_path),
                "writable": exists && can_read_write(&dev_path),
                "is_nvidia": lower.contains("nvidia"),
                "is_smbus": lower.contains("smbus") || lower.contains("piix4"),
            })
        })
        .collect()
}

fn can_read(path: &str) -> bool {
    let Ok(c) = std::ffi::CString::new(path) else { return false };
    // SAFETY: `c` is a valid NUL-terminated string.
    unsafe { libc::access(c.as_ptr(), libc::R_OK) == 0 }
}

fn can_read_write(path: &str) -> bool {
    let Ok(c) = std::ffi::CString::new(path) else { return false };
    // SAFETY: `c` is a valid NUL-terminated string.
    unsafe { libc::access(c.as_ptr(), libc::R_OK | libc::W_OK) == 0 }
}

/// Pick the most likely GPU I2C bus (NVIDIA adapter, preferring `/dev/i2c-1`).
#[must_use]
pub fn candidate_gpu_bus() -> String {
    let nvidia: Vec<String> = detect_i2c_adapters()
        .iter()
        .filter(|a| a["is_nvidia"].as_bool().unwrap_or(false))
        .filter_map(|a| a["dev_path"].as_str().map(str::to_string))
        .collect();
    if nvidia.is_empty() || nvidia.iter().any(|b| b == "/dev/i2c-1") {
        "/dev/i2c-1".to_string()
    } else {
        nvidia[0].clone()
    }
}

fn param_str(params: &toml::Value, key: &str, default: &str) -> String {
    params.get(key).and_then(toml::Value::as_str).map_or_else(|| default.to_string(), str::to_string)
}

fn param_addr(params: &toml::Value) -> u16 {
    params
        .get("addr")
        .and_then(toml::Value::as_integer)
        .and_then(|v| u16::try_from(v).ok())
        .unwrap_or(0x67)
}

/// GPU backend (ENE Aura over I2C).
#[derive(Default)]
pub struct EneI2cBackend;

impl Backend for EneI2cBackend {
    fn id(&self) -> String {
        "ene_i2c".to_string()
    }

    fn name(&self) -> String {
        "ENE Aura (I2C direto)".to_string()
    }

    fn kinds(&self) -> Vec<DeviceKind> {
        vec![DeviceKind::Gpu]
    }

    fn detect(&self, enabled: bool) -> Vec<DetectedDevice> {
        let bus = candidate_gpu_bus();
        let probe = test_ene_aura(&bus, 0x67);
        if !probe["success"].as_bool().unwrap_or(false) {
            return Vec::new();
        }
        vec![DetectedDevice {
            id: format!("{bus}@0x67"),
            name: "ENE Aura (GPU ASUS)".to_string(),
            kinds: vec![DeviceKind::Gpu],
            backend: self.id(),
            enabled,
        }]
    }

    fn apply(&self, color: &Color, params: &toml::Value) -> Result<ApplyOutcome> {
        let bus = param_str(params, "bus", &candidate_gpu_bus());
        let addr = param_addr(params);
        if set_ene_color(&color.hex, &bus, addr)? {
            Ok(ApplyOutcome::ok(1))
        } else {
            Ok(ApplyOutcome::skipped(format!("GPU ENE skipped ({bus} not available)")))
        }
    }

    fn off(&self, params: &toml::Value) -> Result<ApplyOutcome> {
        let bus = param_str(params, "bus", &candidate_gpu_bus());
        let addr = param_addr(params);
        let _ = set_ene_color("000000", &bus, addr);
        Ok(ApplyOutcome::ok(1))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn addr_defaults_to_0x67() {
        assert_eq!(param_addr(&toml::Value::Table(toml::map::Map::new())), 0x67);
    }
}
