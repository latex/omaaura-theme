//! Backend registry: discovers built-in and external backends and resolves the
//! configured application order.

use toml::Value as TomlValue;

use crate::backend::{Backend, DetectedDevice, builtin, external};
use crate::config;

pub struct Registry {
    backends: Vec<Box<dyn Backend>>,
}

impl Registry {
    /// Load built-in backends plus any external manifests on disk.
    #[must_use]
    pub fn load() -> Self {
        let mut backends: Vec<Box<dyn Backend>> = builtin::builtin_backends();
        for ext in external::discover() {
            backends.push(Box::new(ext));
        }
        Self { backends }
    }

    #[must_use]
    pub fn get(&self, id: &str) -> Option<&dyn Backend> {
        self.backends.iter().find(|b| b.id() == id).map(Box::as_ref)
    }

    pub fn iter(&self) -> impl Iterator<Item = &dyn Backend> {
        self.backends.iter().map(Box::as_ref)
    }

    /// Backends in configured order. Falls back to the full registry when the
    /// config declares no order.
    #[must_use]
    pub fn ordered<'a>(&'a self, cfg: &TomlValue) -> Vec<&'a dyn Backend> {
        let order = config::backend_order(cfg);
        if order.is_empty() {
            return self.iter().collect();
        }
        order.iter().filter_map(|id| self.get(id)).collect()
    }

    /// Detect controllable devices across every registered backend. Only
    /// controllable devices are returned (per design).
    #[must_use]
    pub fn detect_all(&self, cfg: &TomlValue) -> Vec<DetectedDevice> {
        let order = config::backend_order(cfg);
        let mut out = Vec::new();
        for backend in &self.backends {
            let enabled = order.is_empty() || order.contains(&backend.id());
            out.extend(backend.detect(enabled));
        }
        out
    }
}
