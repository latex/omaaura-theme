//! OmaAura — ASUS Aura hardware LED & Omarchy theme synchronization.
//!
//! The core is hardware-agnostic: it calibrates the theme color and hands it to
//! the configured [`backend`]s (built-in and external). See `docs/BACKENDS.md`.

mod backend;
mod color;
mod config;
mod palette;
mod setup;

use std::os::fd::AsRawFd;
use std::path::PathBuf;
use std::time::Duration;

use anyhow::{Context, Result, bail};
use clap::{Parser, Subcommand};
use toml::Value as TomlValue;

use crate::backend::Color;
use crate::backend::builtin::{ene_i2c, openrgb};
use crate::backend::registry::Registry;

#[derive(Parser)]
#[command(
    name = "omaaura",
    version,
    about = "OmaAura: ASUS Aura hardware LED & Omarchy theme synchronization."
)]
struct Cli {
    #[command(subcommand)]
    command: Option<Cmd>,
}

#[derive(Subcommand)]
enum Cmd {
    /// Sincroniza os LEDs com o tema Omarchy atual (padrão)
    Sync,
    /// Define uma cor hexadecimal específica (ex.: ff0055, #89b4fa)
    Set {
        /// Código hexadecimal da cor
        hex: String,
    },
    /// Apaga todos os LEDs
    Off,
    /// Alterna os LEDs entre ligado e desligado
    Toggle,
    /// Mostra o status atual de hardware e tema
    Status,
    /// Lista os dispositivos de LED controláveis (todos os backends)
    Devices {
        /// Saída em JSON
        #[arg(long)]
        json: bool,
    },
    /// Lista os backends de hardware registrados (internos + externos)
    Backends,
    /// Executa o assistente interativo de configuração
    Setup {
        /// Executa sem prompts, usando os melhores padrões detectados
        #[arg(short = 'y', long)]
        yes: bool,
    },
    /// Inspeciona I2C/OpenRGB e o inventário de backends
    #[command(name = "test-hardware")]
    TestHardware,
    /// Emite o JSON da paleta (tema + wallpaper) consumido pelo widget
    Palette,
    /// Roda como daemon de sincronização em segundo plano
    Daemon,
}

fn state_file() -> PathBuf {
    config::state_dir().join("state")
}

/// Apply LED calibration — the single source of truth for hardware colors.
fn calibrate_color(hex_code: &str, cfg: &TomlValue) -> String {
    let clean = hex_code.trim().trim_start_matches('#');
    if !config::get_bool(cfg, "theme", "calibrate_led", true) {
        return clean.to_string();
    }
    color::calibrate_hex(
        clean,
        config::get_f64(cfg, "theme", "saturation_floor", 0.70),
        config::get_f64(cfg, "theme", "value_target", 1.0),
        &config::get_str(cfg, "theme", "fallback_color", color::DEFAULT_FALLBACK),
    )
}

/// Read the *raw* active Omarchy theme accent color (no LED calibration).
fn get_theme_color(cfg: &TomlValue) -> String {
    let fallback = config::get_str(cfg, "theme", "fallback_color", color::DEFAULT_FALLBACK);
    let base = config::home().join(".local/state/omarchy/current/theme");

    if let Ok(raw) = std::fs::read_to_string(base.join("keyboard.rgb")) {
        let raw = raw.trim().trim_start_matches('#').to_string();
        if raw.len() == 6 && raw.chars().all(|c| c.is_ascii_hexdigit()) {
            return raw;
        }
    }

    if let Ok(text) = std::fs::read_to_string(base.join("colors.toml"))
        && let Ok(data) = text.parse::<TomlValue>()
            && let Some(accent) = data.get("accent").and_then(TomlValue::as_str)
                && accent.starts_with('#') && accent.len() == 7 {
                    return accent.trim_start_matches('#').to_string();
                }
    fallback
}

/// External file lock guarding concurrent hardware writes.
struct Lock {
    _file: std::fs::File,
}

impl Lock {
    fn acquire() -> Result<Self> {
        let dir = std::env::var_os("XDG_RUNTIME_DIR").map_or_else(|| PathBuf::from("/tmp"), PathBuf::from);
        let path = dir.join("omaaura-theme.lock");
        let file = std::fs::OpenOptions::new()
            .create(true)
            .write(true)
            .truncate(false)
            .open(&path)
            .with_context(|| format!("opening lock {}", path.display()))?;
        // SAFETY: `file` owns a valid fd for the duration of the lock.
        if unsafe { libc::flock(file.as_raw_fd(), libc::LOCK_EX) } != 0 {
            bail!("flock failed: {}", std::io::Error::last_os_error());
        }
        Ok(Self { _file: file })
    }
}

/// Apply a hex color across every configured backend, in order.
fn apply_color(hex_str: &str, cfg: &TomlValue, registry: &Registry) -> bool {
    let applied = calibrate_color(hex_str, cfg);
    let color = Color::new(&applied);
    if let Some(parent) = state_file().parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    let Ok(_lock) = Lock::acquire() else {
        eprintln!("Warning: could not acquire hardware lock");
        return false;
    };

    let mut ok = true;
    for backend in registry.ordered(cfg) {
        let params = config::backend_params(cfg, &backend.id());
        match backend.apply(&color, &params) {
            Ok(outcome) => {
                if !outcome.applied {
                    ok = false;
                    if let Some(msg) = outcome.message {
                        eprintln!("Warning: [{}] {msg}", backend.id());
                    }
                }
            }
            Err(e) => {
                eprintln!("Warning: [{}] {e}", backend.id());
                ok = false;
            }
        }
    }

    let _ = std::fs::write(state_file(), format!("on:{}\n", color.hash()));
    ok
}

/// Turn off every configured backend.
fn turn_off(cfg: &TomlValue, registry: &Registry) -> bool {
    if let Some(parent) = state_file().parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    let Ok(_lock) = Lock::acquire() else {
        return false;
    };
    for backend in registry.ordered(cfg) {
        let params = config::backend_params(cfg, &backend.id());
        if let Err(e) = backend.off(&params) {
            eprintln!("Warning: [{}] {e}", backend.id());
        }
    }
    let _ = std::fs::write(state_file(), "off\n");
    true
}

fn current_state() -> String {
    std::fs::read_to_string(state_file()).map_or_else(|_| "unknown".to_string(), |s| s.trim().to_string())
}

fn toggle(cfg: &TomlValue, registry: &Registry) {
    if current_state().starts_with("off") {
        let theme = get_theme_color(cfg);
        apply_color(&theme, cfg, registry);
    } else {
        turn_off(cfg, registry);
    }
}

fn cmd_devices(cfg: &TomlValue, registry: &Registry, json: bool) -> Result<()> {
    let devices = registry.detect_all(cfg);
    if json {
        println!("{}", serde_json::to_string_pretty(&devices)?);
        return Ok(());
    }
    if devices.is_empty() {
        println!("Nenhum dispositivo de LED controlável encontrado.");
        return Ok(());
    }
    println!("{:<14} {:<9} {:<26} DISPOSITIVO", "BACKEND", "ATIVO", "TIPOS");
    for d in &devices {
        let kinds: Vec<&str> = d.kinds.iter().map(|k| k.as_str()).collect();
        println!(
            "{:<14} {:<9} {:<26} {} [{}]",
            d.backend,
            if d.enabled { "sim" } else { "não" },
            kinds.join(","),
            d.name,
            d.id
        );
    }
    Ok(())
}

fn cmd_backends(cfg: &TomlValue, registry: &Registry) {
    let order = config::backend_order(cfg);
    println!("{:<14} {:<10} {:<28} {:<7} TIPOS", "ID", "VERSÃO", "NOME", "ATIVO");
    for backend in registry.iter() {
        let id = backend.id();
        let kinds: Vec<&str> = backend.kinds().iter().map(|k| k.as_str()).collect();
        println!(
            "{:<14} {:<10} {:<28} {:<7} {}",
            id,
            backend.version().unwrap_or_else(|| "-".to_string()),
            backend.name(),
            if order.contains(&id) { "sim" } else { "não" },
            kinds.join(",")
        );
    }
}

fn main() -> Result<()> {
    let cli = Cli::parse();

    match cli.command.unwrap_or(Cmd::Sync) {
        Cmd::Sync => {
            let cfg = config::load_config();
            let registry = Registry::load();
            let theme = get_theme_color(&cfg);
            let applied = calibrate_color(&theme, &cfg);
            apply_color(&theme, &cfg, &registry);
            println!("OmaAura: Applied theme color #{applied} (from #{theme})");
        }
        Cmd::Set { hex } => {
            let cfg = config::load_config();
            let registry = Registry::load();
            let clean = hex.trim().trim_start_matches('#');
            if clean.len() != 6 {
                bail!("Invalid hex color '{hex}'. Use format RRGGBB.");
            }
            let applied = calibrate_color(clean, &cfg);
            apply_color(clean, &cfg, &registry);
            println!("OmaAura: Applied color #{applied} (from #{clean})");
        }
        Cmd::Off => {
            let cfg = config::load_config();
            let registry = Registry::load();
            turn_off(&cfg, &registry);
            println!("OmaAura: LEDs turned off.");
        }
        Cmd::Toggle => {
            let cfg = config::load_config();
            let registry = Registry::load();
            toggle(&cfg, &registry);
        }
        Cmd::Status => {
            let cfg = config::load_config();
            let registry = Registry::load();
            let theme = calibrate_color(&get_theme_color(&cfg), &cfg);
            println!("Tema atual (normalizado): #{theme}");
            println!("Estado LEDs: {}", current_state());
            println!("Backends ativos: {}", config::backend_order(&cfg).join(", "));
            println!("Dispositivos controláveis: {}", registry.detect_all(&cfg).len());
        }
        Cmd::Devices { json } => {
            let cfg = config::load_config();
            let registry = Registry::load();
            cmd_devices(&cfg, &registry, json)?;
        }
        Cmd::Backends => {
            let cfg = config::load_config();
            let registry = Registry::load();
            cmd_backends(&cfg, &registry);
        }
        Cmd::Setup { yes } => {
            setup::run_setup(!yes)?;
        }
        Cmd::TestHardware => {
            let cfg = config::load_config();
            let registry = Registry::load();
            let report = serde_json::json!({
                "gpu_bus_candidate": ene_i2c::candidate_gpu_bus(),
                "i2c_adapters": ene_i2c::detect_i2c_adapters(),
                "ene_aura": ene_i2c::test_ene_aura(&ene_i2c::candidate_gpu_bus(), 0x67),
                "openrgb_installed": openrgb::is_openrgb_available(),
                "openrgb_devices": openrgb::detect_openrgb_devices()
                    .into_iter().map(|d| serde_json::json!({"id": d.id, "name": d.name})).collect::<Vec<_>>(),
                "backends_order": config::backend_order(&cfg),
                "devices": registry.detect_all(&cfg),
            });
            println!("{}", serde_json::to_string_pretty(&report)?);
        }
        Cmd::Palette => {
            let cfg = config::load_config();
            println!("{}", palette::get_palette(&cfg));
        }
        Cmd::Daemon => run_daemon()?,
    }

    Ok(())
}

fn run_daemon() -> Result<()> {
    let cfg = config::load_config();
    let registry = Registry::load();
    let interval = config::get_f64(&cfg, "service", "poll_interval_sec", 2.0).max(0.2);
    println!("OmaAura daemon started (poll interval: {interval}s)...");

    let mut last_color = get_theme_color(&cfg);
    apply_color(&last_color, &cfg, &registry);

    loop {
        std::thread::sleep(Duration::from_secs_f64(interval));
        let current = get_theme_color(&cfg);
        if current != last_color {
            apply_color(&current, &cfg, &registry);
            last_color = current;
            println!("[OmaAura] Theme color update: #{last_color}");
        }
    }
}
