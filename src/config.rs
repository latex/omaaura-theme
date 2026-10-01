//! Configuration management for OmaAura (`~/.config/omaaura/config.toml`).
//!
//! Layout (v2):
//! ```toml
//! [backends]
//! order = ["openrgb", "ene_i2c"]   # enabled + application order
//!
//! [ene_i2c]
//! bus = "/dev/i2c-1"
//! addr = 0x67
//!
//! [openrgb]
//! devices = "all"
//!
//! [theme]
//! calibrate_led = true
//! ```

use std::path::PathBuf;

use toml::Value;
use toml::map::Map as TomlMap;

#[must_use]
pub fn home() -> PathBuf {
    std::env::var_os("HOME").map_or_else(|| PathBuf::from("/"), PathBuf::from)
}

#[must_use]
pub fn config_dir() -> PathBuf {
    let base = std::env::var_os("XDG_CONFIG_HOME").map_or_else(|| home().join(".config"), PathBuf::from);
    base.join("omaaura")
}

#[must_use]
pub fn config_path() -> PathBuf {
    config_dir().join("config.toml")
}

#[must_use]
pub fn state_dir() -> PathBuf {
    let base = std::env::var_os("XDG_STATE_HOME").map_or_else(|| home().join(".local/state"), PathBuf::from);
    base.join("omaaura-theme")
}

fn get<'a>(table: &'a Value, section: &str, key: &str) -> Option<&'a Value> {
    table.get(section).and_then(|s| s.get(key))
}

#[must_use]
pub fn get_str(table: &Value, section: &str, key: &str, default: &str) -> String {
    get(table, section, key)
        .and_then(Value::as_str)
        .map_or_else(|| default.to_string(), str::to_string)
}

#[must_use]
pub fn get_bool(table: &Value, section: &str, key: &str, default: bool) -> bool {
    get(table, section, key).and_then(Value::as_bool).unwrap_or(default)
}

#[must_use]
pub fn get_f64(table: &Value, section: &str, key: &str, default: f64) -> f64 {
    get(table, section, key).and_then(Value::as_float).unwrap_or(default)
}

/// Enabled backends, in application order (`backends.order`).
#[must_use]
pub fn backend_order(cfg: &Value) -> Vec<String> {
    cfg.get("backends")
        .and_then(|b| b.get("order"))
        .and_then(Value::as_array)
        .map(|a| a.iter().filter_map(Value::as_str).map(str::to_string).collect())
        .unwrap_or_default()
}

/// A backend's own parameter table (e.g. `[openrgb]`). Empty when absent.
#[must_use]
pub fn backend_params(cfg: &Value, id: &str) -> Value {
    cfg.get(id).cloned().unwrap_or_else(|| Value::Table(TomlMap::new()))
}

/// Default configuration (mirrors the historical Python defaults, v2 layout).
#[must_use]
pub fn default_config() -> Value {
    let mut backends = TomlMap::new();
    backends.insert(
        "order".into(),
        Value::Array(vec![Value::String("openrgb".into()), Value::String("ene_i2c".into())]),
    );

    let mut ene = TomlMap::new();
    ene.insert("bus".into(), Value::String("/dev/i2c-1".into()));
    ene.insert("addr".into(), Value::Integer(0x67));

    let mut openrgb = TomlMap::new();
    openrgb.insert("devices".into(), Value::String("all".into()));

    let mut theme = TomlMap::new();
    theme.insert("sync_mode".into(), Value::String("accent".into()));
    theme.insert("calibrate_led".into(), Value::Boolean(true));
    theme.insert("saturation_floor".into(), Value::Float(1.0));
    theme.insert("value_target".into(), Value::Float(1.0));
    theme.insert("fallback_color".into(), Value::String("89b4fa".into()));

    let mut service = TomlMap::new();
    service.insert("enabled".into(), Value::Boolean(true));
    service.insert("poll_interval_sec".into(), Value::Float(2.0));

    let mut root = TomlMap::new();
    root.insert("backends".into(), Value::Table(backends));
    root.insert("ene_i2c".into(), Value::Table(ene));
    root.insert("openrgb".into(), Value::Table(openrgb));
    root.insert("theme".into(), Value::Table(theme));
    root.insert("service".into(), Value::Table(service));
    Value::Table(root)
}

/// Load configuration, merged over defaults. Never fails.
#[must_use]
pub fn load_config() -> Value {
    let defaults = default_config();
    let Ok(text) = std::fs::read_to_string(config_path()) else {
        return defaults;
    };
    let Ok(data) = text.parse::<Value>() else {
        return defaults;
    };
    merge(defaults, &data)
}

fn merge(base: Value, overlay: &Value) -> Value {
    match (base, overlay) {
        (Value::Table(mut base_tbl), Value::Table(overlay_tbl)) => {
            for (section, values) in overlay_tbl {
                let merged = match base_tbl.get(section) {
                    Some(existing) => merge(existing.clone(), values),
                    None => values.clone(),
                };
                base_tbl.insert(section.clone(), merged);
            }
            Value::Table(base_tbl)
        }
        (_, other) => other.clone(),
    }
}

fn format_value(val: &Value) -> String {
    match val {
        Value::Boolean(b) => if *b { "true" } else { "false" }.to_string(),
        Value::Integer(i) if *i == 0x67 || *i == 0x68 => format!("0x{i:02x}"),
        Value::Integer(i) => i.to_string(),
        Value::Float(f) => {
            if f.fract().abs() < f64::EPSILON {
                format!("{f:.1}")
            } else {
                f.to_string()
            }
        }
        Value::String(s) => format!("\"{}\"", s.replace('\\', "\\\\").replace('"', "\\\"")),
        Value::Array(items) => {
            let inner: Vec<String> = items.iter().map(format_value).collect();
            format!("[{}]", inner.join(", "))
        }
        other => format!("\"{other}\""),
    }
}

/// Persist configuration in the same layout the wizard produces.
pub fn save_config(config: &Value) -> std::io::Result<PathBuf> {
    let dir = config_dir();
    std::fs::create_dir_all(&dir)?;
    let path = config_path();

    let mut lines = vec![
        "# OmaAura Configuration File".to_string(),
        "# Automatically generated by 'omaaura setup'".to_string(),
        String::new(),
    ];

    if let Value::Table(sections) = config {
        for (section_name, section_value) in sections {
            if let Value::Table(section) = section_value {
                lines.push(format!("[{section_name}]"));
                for (key, val) in section {
                    lines.push(format!("{key} = {}", format_value(val)));
                }
                lines.push(String::new());
            }
        }
    }

    let content = format!("{}\n", lines.join("\n").trim());
    std::fs::write(&path, content)?;
    Ok(path)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_order_is_openrgb_then_ene() {
        let cfg = default_config();
        assert_eq!(backend_order(&cfg), vec!["openrgb", "ene_i2c"]);
    }

    #[test]
    fn params_are_readable() {
        let cfg = default_config();
        let ene = backend_params(&cfg, "ene_i2c");
        assert_eq!(ene.get("bus").and_then(Value::as_str), Some("/dev/i2c-1"));
        assert_eq!(ene.get("addr").and_then(Value::as_integer), Some(0x67));
        let missing = backend_params(&cfg, "does_not_exist");
        assert!(missing.as_table().is_some_and(toml::map::Map::is_empty));
    }

    #[test]
    fn hex_addr_formatting() {
        assert_eq!(format_value(&Value::Integer(0x67)), "0x67");
        assert_eq!(format_value(&Value::Float(2.0)), "2.0");
    }

    #[test]
    fn merge_is_partial() {
        let base = default_config();
        let overlay: Value = "[backends]\norder = [\"ene_i2c\"]\n".parse().unwrap();
        let merged = merge(base, &overlay);
        assert_eq!(backend_order(&merged), vec!["ene_i2c"]);
        assert!(get_bool(&merged, "theme", "calibrate_led", false));
    }
}
