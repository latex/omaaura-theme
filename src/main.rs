//! OmaAura — ASUS Aura hardware LED & Omarchy theme synchronization.
//!
//! Unified CLI entry point. Replaces the former Python implementation with a
//! single static Rust binary (v2.0.0).

mod color;
mod config;
mod hardware;
mod palette;
mod setup;

use std::os::fd::AsRawFd;
use std::path::PathBuf;
use std::time::Duration;

use anyhow::{Context, Result, bail};
use clap::{Parser, Subcommand};
use toml::Value as TomlValue;

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
    /// Executa o assistente interativo de configuração
    Setup {
        /// Executa sem prompts, usando os melhores padrões detectados
        #[arg(short = 'y', long)]
        yes: bool,
    },
    /// Inspeciona e testa o hardware I2C e o OpenRGB
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
        let rc = unsafe { libc::flock(file.as_raw_fd(), libc::LOCK_EX) };
        if rc != 0 {
            bail!("flock failed: {}", std::io::Error::last_os_error());
        }
        Ok(Self { _file: file })
    }
}

fn hardware_u16(cfg: &TomlValue, key: &str, default: i64) -> u16 {
    u16::try_from(cfg.get("hardware").and_then(|h| h.get(key)).and_then(TomlValue::as_integer).unwrap_or(default))
        .unwrap_or(0x67)
}

/// Apply a hex color across all configured hardware backends.
fn apply_color(hex_str: &str, cfg: &TomlValue) -> bool {
    let clean = hex_str.trim().trim_start_matches('#');
    let applied = calibrate_color(clean, cfg);
    if let Some(parent) = state_file().parent() {
        let _ = std::fs::create_dir_all(parent);
    }

    let Ok(_lock) = Lock::acquire() else {
        eprintln!("Warning: could not acquire hardware lock");
        return false;
    };

    let mut ok = true;

    // 1. Motherboard / ARGB / RAM first, so the GPU (written last) always wins.
    if config::get_str(cfg, "hardware", "motherboard_backend", "none") == "openrgb" {
        let ids = hardware::DeviceIds::from_config(cfg);
        match hardware::set_openrgb_color(&applied, &ids, Duration::from_secs(25)) {
            Ok(true) => {}
            Ok(false) => {
                eprintln!("Warning: OpenRGB color write failed");
                ok = false;
            }
            Err(e) => {
                eprintln!("Warning: Failed to set OpenRGB color: {e}");
                ok = false;
            }
        }
    }

    // 2. GPU via ENE Aura direct I2C.
    if config::get_str(cfg, "hardware", "gpu_backend", "none") == "ene_i2c" {
        let dev = config::get_str(cfg, "hardware", "gpu_i2c_bus", "/dev/i2c-1");
        let addr = hardware_u16(cfg, "gpu_i2c_addr", 0x67);
        match hardware::set_ene_color(&applied, &dev, addr) {
            Ok(true) => {}
            Ok(false) => {
                eprintln!("Warning: GPU ENE write skipped ({dev} not available)");
                ok = false;
            }
            Err(e) => {
                eprintln!("Warning: Failed to set GPU ENE color: {e}");
                ok = false;
            }
        }
    }

    let _ = std::fs::write(state_file(), format!("on:#{applied}\n"));
    ok
}

/// Turn off all hardware LEDs.
fn turn_off(cfg: &TomlValue) -> bool {
    if let Some(parent) = state_file().parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    let Ok(_lock) = Lock::acquire() else {
        return false;
    };

    if config::get_str(cfg, "hardware", "gpu_backend", "none") == "ene_i2c" {
        let dev = config::get_str(cfg, "hardware", "gpu_i2c_bus", "/dev/i2c-1");
        let addr = hardware_u16(cfg, "gpu_i2c_addr", 0x67);
        let _ = hardware::set_ene_color("000000", &dev, addr);
    }
    if config::get_str(cfg, "hardware", "motherboard_backend", "none") == "openrgb" {
        let ids = hardware::DeviceIds::from_config(cfg);
        let _ = hardware::set_openrgb_color("000000", &ids, Duration::from_secs(25));
    }

    let _ = std::fs::write(state_file(), "off\n");
    true
}

fn current_state() -> String {
    std::fs::read_to_string(state_file()).map_or_else(|_| "unknown".to_string(), |s| s.trim().to_string())
}

fn toggle(cfg: &TomlValue) {
    if current_state().starts_with("off") {
        let theme = get_theme_color(cfg);
        apply_color(&theme, cfg);
    } else {
        turn_off(cfg);
    }
}

fn main() -> Result<()> {
    let cli = Cli::parse();

    match cli.command.unwrap_or(Cmd::Sync) {
        Cmd::Sync => {
            let cfg = config::load_config();
            let theme = get_theme_color(&cfg);
            let applied = calibrate_color(&theme, &cfg);
            apply_color(&theme, &cfg);
            println!("OmaAura: Applied theme color #{applied} (from #{theme})");
        }
        Cmd::Set { hex } => {
            let cfg = config::load_config();
            let clean = hex.trim().trim_start_matches('#');
            if clean.len() != 6 {
                bail!("Invalid hex color '{hex}'. Use format RRGGBB.");
            }
            let applied = calibrate_color(clean, &cfg);
            apply_color(clean, &cfg);
            println!("OmaAura: Applied color #{applied} (from #{clean})");
        }
        Cmd::Off => {
            let cfg = config::load_config();
            turn_off(&cfg);
            println!("OmaAura: LEDs turned off.");
        }
        Cmd::Toggle => {
            let cfg = config::load_config();
            toggle(&cfg);
        }
        Cmd::Status => {
            let cfg = config::load_config();
            let theme = calibrate_color(&get_theme_color(&cfg), &cfg);
            println!("Tema atual (normalizado): #{theme}");
            println!("Estado LEDs: {}", current_state());
            println!("GPU Backend: {}", config::get_str(&cfg, "hardware", "gpu_backend", "?"));
            println!("Motherboard Backend: {}", config::get_str(&cfg, "hardware", "motherboard_backend", "?"));
        }
        Cmd::Setup { yes } => {
            setup::run_setup(!yes)?;
        }
        Cmd::TestHardware => {
            let diag = hardware::detect_all_hardware();
            println!("{}", serde_json::to_string_pretty(&diag)?);
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
    let interval = config::get_f64(&cfg, "service", "poll_interval_sec", 2.0).max(0.2);
    println!("OmaAura daemon started (poll interval: {interval}s)...");

    let mut last_color = get_theme_color(&cfg);
    apply_color(&last_color, &cfg);

    loop {
        std::thread::sleep(Duration::from_secs_f64(interval));
        let current = get_theme_color(&cfg);
        if current != last_color {
            apply_color(&current, &cfg);
            last_color = current;
            println!("[OmaAura] Theme color updated: #{last_color}");
        }
    }
}
