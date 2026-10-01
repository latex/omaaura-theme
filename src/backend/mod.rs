//! Backend system: the pluggable hardware abstraction of OmaAura.
//!
//! The core (calibration, theme, CLI, widget, daemon) knows nothing about
//! hardware. It only orchestrates [`Backend`] implementations:
//!
//! * **built-in** — compiled into the crate (`builtin::openrgb`, `builtin::ene_i2c`);
//! * **external** — any executable that speaks the JSON protocol in
//!   [`external`], allowing third-party developers to add hardware in *any*
//!   language without forking this crate.
//!
//! See `docs/BACKENDS.md` for the contributor guide.

pub mod builtin;
pub mod external;
pub mod registry;

use anyhow::Result;

/// A calibrated LED color, as lowercase `rrggbb` (no `#`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Color {
    pub hex: String,
}

impl Color {
    #[must_use]
    pub fn new(hex: &str) -> Self {
        Self { hex: hex.trim().trim_start_matches('#').to_ascii_lowercase() }
    }

    #[must_use]
    pub fn hash(&self) -> String {
        format!("#{}", self.hex)
    }
}

/// What a device physically is. Used for display and (future) routing.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "lowercase")]
pub enum DeviceKind {
    Gpu,
    Motherboard,
    Ram,
    Mic,
    Deck,
    Keyboard,
    Mouse,
    Headset,
    Light,
    Generic,
}

impl DeviceKind {
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Gpu => "gpu",
            Self::Motherboard => "motherboard",
            Self::Ram => "ram",
            Self::Mic => "mic",
            Self::Deck => "deck",
            Self::Keyboard => "keyboard",
            Self::Mouse => "mouse",
            Self::Headset => "headset",
            Self::Light => "light",
            Self::Generic => "generic",
        }
    }

    #[must_use]
    pub fn parse(s: &str) -> Self {
        match s.trim().to_ascii_lowercase().as_str() {
            "gpu" => Self::Gpu,
            "motherboard" | "mainboard" => Self::Motherboard,
            "ram" | "memory" => Self::Ram,
            "mic" | "microphone" => Self::Mic,
            "deck" | "keypad" => Self::Deck,
            "keyboard" => Self::Keyboard,
            "mouse" => Self::Mouse,
            "headset" => Self::Headset,
            "light" | "strip" => Self::Light,
            _ => Self::Generic,
        }
    }
}

/// A device discovered by a backend. Only *controllable* devices are reported.
#[derive(Debug, Clone, serde::Serialize)]
pub struct DetectedDevice {
    /// Backend-local identifier (device path, OpenRGB index, USB id, …).
    pub id: String,
    /// Human-readable name.
    pub name: String,
    pub kinds: Vec<DeviceKind>,
    /// Owning backend id.
    pub backend: String,
    /// Whether the owning backend is enabled in `backends.order`.
    pub enabled: bool,
}

/// Result of an `apply`/`off` call.
#[derive(Debug, Clone, Default, serde::Serialize)]
pub struct ApplyOutcome {
    pub applied: bool,
    /// Number of physical devices written.
    pub devices: usize,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub message: Option<String>,
}

impl ApplyOutcome {
    #[must_use]
    pub fn ok(devices: usize) -> Self {
        Self { applied: true, devices, message: None }
    }

    #[must_use]
    pub fn skipped(message: impl Into<String>) -> Self {
        Self { applied: false, devices: 0, message: Some(message.into()) }
    }
}

/// The contract every hardware backend implements.
///
/// Implementors must be cheap to `detect()` and side-effect free there; all
/// hardware writes happen in [`Backend::apply`] / [`Backend::off`].
pub trait Backend: Send + Sync {
    /// Stable, unique id (e.g. `"openrgb"`, `"ene_i2c"`, `"fifine"`).
    fn id(&self) -> String;

    /// Human-readable name.
    fn name(&self) -> String;

    /// Device kinds this backend is able to drive.
    fn kinds(&self) -> Vec<DeviceKind>;

    /// Optional backend version string (external manifests provide one).
    fn version(&self) -> Option<String> {
        None
    }

    /// Report the controllable devices currently present.
    fn detect(&self, enabled: bool) -> Vec<DetectedDevice>;

    /// Apply a color to every controllable device of this backend.
    ///
    /// `params` is the backend's own table from `config.toml` (e.g. `[openrgb]`).
    fn apply(&self, color: &Color, params: &toml::Value) -> Result<ApplyOutcome>;

    /// Turn every controllable device of this backend off.
    fn off(&self, params: &toml::Value) -> Result<ApplyOutcome>;
}
