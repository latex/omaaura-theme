//! External backend runner: lets **any developer** add hardware in **any
//! language** without forking this crate.
//!
//! An external backend is a manifest (`backend.toml`) plus an executable that
//! speaks a tiny JSON protocol over stdout:
//!
//! ```text
//! <exec> detect          -> {"devices":[{"id":"…","name":"…","kinds":["mic"]}]}
//! <exec> apply <rrggbb>  -> {"applied":true}
//! <exec> off             -> {"applied":true}
//! <exec> info            -> {"id":"…","name":"…","version":"…"}   (optional)
//! ```
//!
//! Manifests are discovered under `~/.config/omaaura/backends/` either as a
//! `backends/<id>/backend.toml` directory or a flat `backends/<id>.toml` file.
//! The backend's config table is passed to the process via the
//! `OMAAURA_PARAMS` environment variable (JSON).

use std::io::Read as _;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

use anyhow::{Context, Result, bail};
use serde::Deserialize;
use serde_json::Value as Json;

use crate::backend::{ApplyOutcome, Backend, Color, DetectedDevice, DeviceKind};
use crate::config;

const CALL_TIMEOUT: Duration = Duration::from_secs(30);

#[derive(Debug, Clone, Deserialize)]
pub struct Manifest {
    pub id: String,
    pub name: String,
    /// Executable name (resolved via `PATH`) or path relative to the manifest.
    pub exec: String,
    #[serde(default)]
    pub kinds: Vec<String>,
    #[serde(default)]
    pub version: Option<String>,
}

/// An external backend described by a manifest on disk.
pub struct ExternalBackend {
    manifest: Manifest,
    base_dir: PathBuf,
}

impl ExternalBackend {
    fn from_manifest_path(path: &Path) -> Result<Self> {
        let text = std::fs::read_to_string(path).with_context(|| format!("reading {}", path.display()))?;
        let manifest: Manifest =
            toml::from_str(&text).with_context(|| format!("parsing {}", path.display()))?;
        if manifest.id.trim().is_empty() || manifest.exec.trim().is_empty() {
            bail!("manifest {} must define non-empty 'id' and 'exec'", path.display());
        }
        let base_dir = path.parent().map_or_else(|| PathBuf::from("."), Path::to_path_buf);
        Ok(Self { manifest, base_dir })
    }

    fn exec_path(&self) -> PathBuf {
        let raw = Path::new(&self.manifest.exec);
        if raw.is_absolute() {
            return raw.to_path_buf();
        }
        let local = self.base_dir.join(raw);
        if local.is_file() {
            local
        } else {
            // Resolved via PATH at spawn time.
            raw.to_path_buf()
        }
    }

    fn run(&self, args: &[&str], params: &toml::Value) -> Result<Json> {
        let mut child = Command::new(self.exec_path())
            .args(args)
            .env("OMAAURA_BACKEND_ID", &self.manifest.id)
            .env("OMAAURA_PARAMS", params.to_string())
            .env("OMAAURA_COLOR", args.get(1).copied().unwrap_or(""))
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .with_context(|| format!("spawning external backend '{}'", self.manifest.id))?;

        let mut stdout = child.stdout.take().expect("piped stdout");
        let deadline = Instant::now() + CALL_TIMEOUT;
        loop {
            match child.try_wait() {
                Ok(Some(status)) => {
                    if !status.success() {
                        bail!("external backend '{}' exited with {:?}", self.manifest.id, status.code());
                    }
                    break;
                }
                Ok(None) if Instant::now() >= deadline => {
                    let _ = child.kill();
                    let _ = child.wait();
                    bail!("external backend '{}' timed out", self.manifest.id);
                }
                Ok(None) => std::thread::sleep(Duration::from_millis(20)),
                Err(e) => bail!("waiting on external backend '{}': {e}", self.manifest.id),
            }
        }

        let mut buf = String::new();
        stdout.read_to_string(&mut buf).context("reading external backend stdout")?;
        let trimmed = buf.trim();
        if trimmed.is_empty() {
            return Ok(Json::Null);
        }
        serde_json::from_str(trimmed).with_context(|| format!("parsing JSON from '{}'", self.manifest.id))
    }
}

impl Backend for ExternalBackend {
    fn id(&self) -> String {
        self.manifest.id.clone()
    }

    fn name(&self) -> String {
        self.manifest.name.clone()
    }

    fn kinds(&self) -> Vec<DeviceKind> {
        self.manifest.kinds.iter().map(|k| DeviceKind::parse(k)).collect()
    }

    fn version(&self) -> Option<String> {
        self.manifest.version.clone()
    }

    fn detect(&self, enabled: bool) -> Vec<DetectedDevice> {
        let params = toml::Value::Table(toml::map::Map::new());
        let Ok(json) = self.run(&["detect"], &params) else {
            return Vec::new();
        };
        let Some(devices) = json.get("devices").and_then(Json::as_array) else {
            return Vec::new();
        };
        devices
            .iter()
            .filter_map(|d| {
                let id = d.get("id")?.as_str()?.to_string();
                let name = d.get("name").and_then(Json::as_str).unwrap_or(&id).to_string();
                let kinds = d
                    .get("kinds")
                    .and_then(Json::as_array)
                    .map(|a| a.iter().filter_map(Json::as_str).map(DeviceKind::parse).collect())
                    .unwrap_or_else(|| self.kinds());
                Some(DetectedDevice { id, name, kinds, backend: self.manifest.id.clone(), enabled })
            })
            .collect()
    }

    fn apply(&self, color: &Color, params: &toml::Value) -> Result<ApplyOutcome> {
        let json = self.run(&["apply", &color.hex], params)?;
        Ok(outcome_from(&json))
    }

    fn off(&self, params: &toml::Value) -> Result<ApplyOutcome> {
        let json = self.run(&["off"], params)?;
        Ok(outcome_from(&json))
    }
}

fn outcome_from(json: &Json) -> ApplyOutcome {
    ApplyOutcome {
        applied: json.get("applied").and_then(Json::as_bool).unwrap_or(false),
        devices: json.get("devices").and_then(Json::as_u64).and_then(|n| usize::try_from(n).ok()).unwrap_or(0),
        message: json.get("message").and_then(Json::as_str).map(str::to_string),
    }
}

/// Discover external backend manifests under `~/.config/omaaura/backends/`.
#[must_use]
pub fn discover() -> Vec<ExternalBackend> {
    let dir = config::config_dir().join("backends");
    let Ok(entries) = std::fs::read_dir(&dir) else {
        return Vec::new();
    };

    let mut manifests: Vec<PathBuf> = Vec::new();
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            let manifest = path.join("backend.toml");
            if manifest.is_file() {
                manifests.push(manifest);
            }
        } else if path.extension().is_some_and(|e| e == "toml") {
            manifests.push(path);
        }
    }
    manifests.sort();

    manifests
        .iter()
        .filter_map(|p| match ExternalBackend::from_manifest_path(p) {
            Ok(b) => Some(b),
            Err(e) => {
                eprintln!("Warning: skipping external backend {}: {e}", p.display());
                None
            }
        })
        .collect()
}
