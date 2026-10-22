//! Built-in backend: OpenRGB (`openrgb` CLI) for motherboards, RAM, ARGB
//! headers and other OpenRGB-supported devices.
//!
//! **Safety invariant:** this backend *never* runs `openrgb -c` without `-d`.
//! If the non-GPU target set cannot be resolved, the write is skipped instead
//! of hitting every device (which would include the GPU driven over I2C).
//!
//! **Server invariant:** the OpenRGB CLI only *applies* direct color commands
//! when it can reach a running OpenRGB SDK server (client mode). In standalone
//! mode it detects the devices, writes **nothing**, and still exits `0` — a
//! silent no-op. Every write is therefore gated by a reachability probe of
//! the SDK server, and the CLI output is captured instead of discarded so a
//! failed write can never be reported as a success.

use std::io::Read;
use std::net::{TcpStream, ToSocketAddrs};
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

/// Address of the OpenRGB SDK server (`openrgb --server`).
const OPENRGB_SERVER_ADDR: &str = "127.0.0.1:6742";

/// Is the OpenRGB SDK server reachable?
///
/// The `openrgb` CLI only applies direct color commands in **client mode**,
/// i.e. when it can talk to a running SDK server. In standalone mode it
/// detects the devices, writes nothing, and still exits `0`.
fn server_reachable(addr: &str) -> bool {
    let Ok(mut addrs) = addr.to_socket_addrs() else {
        return false;
    };
    let Some(sock) = addrs.next() else {
        return false;
    };
    TcpStream::connect_timeout(&sock, Duration::from_millis(900)).is_ok()
}

/// List OpenRGB devices via `openrgb --list-devices`.
#[must_use]
pub fn detect_openrgb_devices() -> Vec<OpenRgbDevice> {
    if !is_openrgb_available() {
        return Vec::new();
    }
    let Ok(output) = Command::new("openrgb").arg("--list-devices").output() else {
        return Vec::new();
    };
    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        eprintln!("Warning: openrgb --list-devices failed: {}", stderr.trim());
        return Vec::new();
    }
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

/// Captured result of an `openrgb` CLI invocation.
#[derive(Debug, Clone, Default)]
struct CliOutput {
    success: bool,
    stdout: String,
    stderr: String,
    timed_out: bool,
}

impl CliOutput {
    /// Best-effort human-readable diagnostic from the CLI.
    fn diagnostic(&self) -> String {
        let stderr = self.stderr.trim();
        let stdout = self.stdout.trim();
        if !stderr.is_empty() {
            stderr.to_string()
        } else if !stdout.is_empty() {
            stdout.to_string()
        } else {
            "no output".to_string()
        }
    }
}

/// Run the `openrgb` CLI, capturing stdout/stderr (never discarding them).
///
/// The previous implementation piped both streams to `/dev/null` and trusted
/// the exit code alone. That is exactly what hid the failure this backend
/// must surface: OpenRGB exits `0` even when it writes nothing.
fn run_openrgb(args: &[&str], timeout: Duration) -> CliOutput {
    let mut child = match Command::new("openrgb")
        .args(args)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
    {
        Ok(child) => child,
        Err(e) => {
            return CliOutput {
                success: false,
                stdout: String::new(),
                stderr: format!("failed to spawn openrgb: {e}"),
                timed_out: false,
            };
        }
    };

    // Drain both pipes concurrently: a full pipe buffer would deadlock the child.
    let mut stdout_pipe = child.stdout.take();
    let mut stderr_pipe = child.stderr.take();
    let out_handle = std::thread::spawn(move || {
        let mut buf = Vec::new();
        if let Some(p) = stdout_pipe.as_mut() {
            let _ = p.read_to_end(&mut buf);
        }
        String::from_utf8_lossy(&buf).into_owned()
    });
    let err_handle = std::thread::spawn(move || {
        let mut buf = Vec::new();
        if let Some(p) = stderr_pipe.as_mut() {
            let _ = p.read_to_end(&mut buf);
        }
        String::from_utf8_lossy(&buf).into_owned()
    });

    let deadline = Instant::now() + timeout;
    let mut timed_out = false;
    let status = loop {
        match child.try_wait() {
            Ok(Some(status)) => break Some(status),
            Ok(None) if Instant::now() >= deadline => {
                let _ = child.kill();
                let _ = child.wait();
                timed_out = true;
                break None;
            }
            Ok(None) => std::thread::sleep(Duration::from_millis(50)),
            Err(_) => break None,
        }
    };

    let stdout = out_handle.join().unwrap_or_default();
    let stderr = err_handle.join().unwrap_or_default();
    let success = !timed_out && status.is_some_and(|s| s.success());
    CliOutput { success, stdout, stderr, timed_out }
}

/// Write a color, restricted to non-GPU devices. Never calls a filterless write.
pub fn set_openrgb_color(hex_str: &str, device_ids: &DeviceIds) -> Result<ApplyOutcome> {
    set_openrgb_color_on(hex_str, device_ids, OPENRGB_SERVER_ADDR)
}

/// Same as [`set_openrgb_color`], with an explicit SDK server address.
fn set_openrgb_color_on(
    hex_str: &str,
    device_ids: &DeviceIds,
    server_addr: &str,
) -> Result<ApplyOutcome> {
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

    // The CLI only writes in client mode. Without the SDK server it exits 0
    // without touching the hardware — report it instead of faking success.
    if !server_reachable(server_addr) {
        return Ok(ApplyOutcome::skipped(format!(
            "OpenRGB SDK server is not running at {server_addr}; the openrgb CLI writes \
             nothing without it. Start it with: sudo systemctl enable --now openrgb.service"
        )));
    }

    let mut failures: Vec<String> = Vec::new();
    for dev_id in &targets {
        let out = run_openrgb(&["-d", &dev_id.to_string(), "-c", clean], Duration::from_secs(25));
        if !out.success {
            let reason = if out.timed_out { "timed out".to_string() } else { out.diagnostic() };
            failures.push(format!("device {dev_id}: {reason}"));
        }
    }
    Ok(if failures.is_empty() {
        ApplyOutcome::ok(targets.len())
    } else {
        ApplyOutcome {
            applied: false,
            devices: targets.len(),
            message: Some(format!("OpenRGB write failed — {}", failures.join("; "))),
        }
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

    #[test]
    fn unreachable_server_is_reported_instead_of_faking_success() {
        // Port 1 is never a live OpenRGB SDK server.
        assert!(!server_reachable("127.0.0.1:1"));
        let out = set_openrgb_color_on("ff0000", &DeviceIds::List(vec![1]), "127.0.0.1:1").unwrap();
        assert!(!out.applied, "a write without the SDK server must never report success");
        assert_eq!(out.devices, 0);
        let msg = out.message.unwrap_or_default();
        if is_openrgb_available() {
            assert!(msg.contains("SDK server"), "unhelpful message: {msg}");
        }
    }

    /// Hardware regression test: apply a color and **read it back** from the
    /// device instead of trusting the CLI exit code.
    ///
    /// This is the exact failure the plugin shipped with: `openrgb -c` exits 0
    /// while writing nothing, so every exit-code-based assertion passed while
    /// the LEDs stayed dark.
    #[test]
    #[ignore = "requires the OpenRGB SDK server and real hardware"]
    fn applied_color_is_read_back_from_hardware() {
        assert!(is_openrgb_available(), "openrgb not installed");
        assert!(server_reachable(OPENRGB_SERVER_ADDR), "OpenRGB SDK server not running");

        let targets = resolve_targets(&DeviceIds::All).expect("resolvable non-GPU targets");
        let id = *targets.first().expect("at least one target");
        let device =
            detect_openrgb_devices().into_iter().find(|d| d.id == id).expect("detected device");

        let hex = "00ffdd";
        let applied = set_openrgb_color(hex, &DeviceIds::List(vec![id])).expect("apply");
        assert!(applied.applied, "apply failed: {:?}", applied.message);

        // Read the *real* LED state back (OpenRGB stores colors as 0x00BBGGRR).
        let profile = "omaaura_selftest";
        let saved = run_openrgb(&["--save-profile", profile], Duration::from_secs(30));
        assert!(saved.success, "save-profile failed: {}", saved.diagnostic());

        let mut candidates = vec![
            config::home().join(".config/OpenRGB/profiles").join(format!("{profile}.json")),
            PathBuf::from("/etc/openrgb/profiles").join(format!("{profile}.json")),
        ];
        let path = candidates
            .iter()
            .find(|p| p.exists())
            .unwrap_or_else(|| panic!("saved profile not found in {candidates:?}"))
            .clone();
        candidates.clear();

        let text = std::fs::read_to_string(&path).expect("profile readable");
        let data: Json = serde_json::from_str(&text).expect("profile is valid JSON");
        let controller = data
            .get("controllers")
            .and_then(Json::as_array)
            .expect("controllers")
            .iter()
            .find(|c| c.get("name").and_then(Json::as_str) == Some(device.name.as_str()))
            .expect("controller in profile");
        let colors = controller.get("colors").and_then(Json::as_array).expect("colors");

        // #00ffdd -> stored as 0xddff00 (0x00BBGGRR).
        let expected = 0x00dd_u32 << 16 | 0x00ff_u32 << 8;
        assert!(
            colors.iter().any(|c| c.as_u64() == Some(u64::from(expected))),
            "color {hex} was not read back from the hardware; got {colors:?}"
        );
    }
}
