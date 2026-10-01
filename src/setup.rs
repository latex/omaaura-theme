//! Interactive setup wizard (`omaaura setup`).
//!
//! Probes I2C buses, ENE Aura controllers and OpenRGB devices, writes
//! `~/.config/omaaura/config.toml` and installs the `systemd --user` service.

use std::io::{BufRead as _, Write as _};
use std::path::PathBuf;
use std::process::Command;

use anyhow::Result;
use toml::Value as TomlValue;

use crate::config::{self, save_config};
use crate::hardware;

const SERVICE_UNIT: &str = include_str!("../service/omaaura.service");
/// Marker used to prove a `omaaura.service` on disk belongs to this plugin.
const SERVICE_MARKER: &str = "OmaAura";

const RESET: &str = "\u{1b}[0m";
const BOLD: &str = "\u{1b}[1m";
const GREEN: &str = "\u{1b}[32m";
const YELLOW: &str = "\u{1b}[33m";
const BLUE: &str = "\u{1b}[34m";
const CYAN: &str = "\u{1b}[36m";
const RED: &str = "\u{1b}[31m";

fn print_header() {
    println!("{BOLD}{CYAN}");
    println!("╔═══════════════════════════════════════════════════════╗");
    println!("║            OmaAura Hardware & RGB Setup Wizard        ║");
    println!("║          ASUS Aura & Omarchy Theme Integrator         ║");
    println!("╚═══════════════════════════════════════════════════════╝");
    println!("{RESET}");
}

fn ask_yes_no(prompt: &str, default: bool, non_interactive: bool) -> bool {
    if non_interactive {
        return default;
    }
    let suffix = if default { " [Y/n]: " } else { " [y/N]: " };
    print!("{BOLD}{prompt}{suffix}{RESET}");
    let _ = std::io::stdout().flush();

    let mut line = String::new();
    if std::io::stdin().lock().read_line(&mut line).is_err() {
        println!();
        return default;
    }
    let ans = line.trim().to_lowercase();
    if ans.is_empty() {
        return default;
    }
    matches!(ans.as_str(), "y" | "yes" | "s" | "sim")
}

/// Run the setup wizard and return the config that was written.
pub fn run_setup(interactive: bool) -> Result<TomlValue> {
    print_header();
    let non_interactive = !interactive;

    println!("{BLUE}==> [1/4] Sondando barramentos I2C e hardware de GPU...{RESET}");
    let diag = hardware::detect_all_hardware();
    let adapters = diag["i2c_adapters"].as_array().cloned().unwrap_or_default();
    let gpu_bus = diag["gpu_bus_candidate"].as_str().unwrap_or("/dev/i2c-1").to_string();
    let ene = &diag["ene_aura"];

    println!("    Barramentos I2C encontrados: {}", adapters.len());
    for ad in &adapters {
        let tag = if ad["is_nvidia"].as_bool().unwrap_or(false) {
            format!("{GREEN}[NVIDIA GPU]{RESET}")
        } else if ad["is_smbus"].as_bool().unwrap_or(false) {
            format!("{YELLOW}[SMBus]{RESET}")
        } else {
            "[Outro]".to_string()
        };
        let rw = if ad["writable"].as_bool().unwrap_or(false) {
            format!("{GREEN}RW OK{RESET}")
        } else {
            format!("{RED}Sem permissão W{RESET}")
        };
        println!("    - {}: {} | {} | {}", ad["dev_path"].as_str().unwrap_or("?"), ad["name"].as_str().unwrap_or("?"), tag, rw);
    }

    println!();
    println!("{BLUE}==> [2/4] Testando controlador ENE Aura (0x67) na GPU...{RESET}");
    let mut gpu_backend = "none";
    if ene["success"].as_bool().unwrap_or(false) {
        println!("    {GREEN}✔ Controlador ENE Aura detectado com sucesso em {gpu_bus} (0x67)!{RESET}");
        gpu_backend = "ene_i2c";
        if interactive && ask_yes_no("    Deseja testar um pulso de luz (Ciano) na GPU agora?", true, false) {
            match hardware::set_ene_color("89dceb", &gpu_bus, 0x67) {
                Ok(true) => println!("    {GREEN}✔ Pulso de iluminação enviado com sucesso.{RESET}"),
                Ok(false) => println!("    {YELLOW}⚠ Dispositivo indisponível para o teste.{RESET}"),
                Err(e) => println!("    {RED}✘ Erro no teste de iluminação: {e}{RESET}"),
            }
        }
    } else {
        println!("    {YELLOW}⚠ ENE Aura: {}{RESET}", ene["message"].as_str().unwrap_or("desconhecido"));
        if !ene["permission_ok"].as_bool().unwrap_or(false) && ene["exists"].as_bool().unwrap_or(false) {
            println!("    {RED}Aviso: Para acesso sem root ao I2C, adicione seu usuário ao grupo 'i2c':{RESET}");
            println!("    {BOLD}sudo usermod -aG i2c $USER{RESET}");
        }
    }

    println!();
    println!("{BLUE}==> [3/4] Verificando integração com OpenRGB...{RESET}");
    let mut motherboard_backend = "none";
    if diag["openrgb_installed"].as_bool().unwrap_or(false) {
        println!("    {GREEN}✔ OpenRGB está instalado no sistema.{RESET}");
        let devices = diag["openrgb_devices"].as_array().cloned().unwrap_or_default();
        if devices.is_empty() {
            println!("    {YELLOW}ℹ Nenhum dispositivo ativo listado no momento pelo OpenRGB.{RESET}");
        } else {
            println!("    Dispositivos detectados ({}):", devices.len());
            for dev in &devices {
                println!("      [{}] {}", dev["id"], dev["name"].as_str().unwrap_or("?"));
            }
        }
        motherboard_backend = "openrgb";
    } else {
        println!("    {YELLOW}⚠ OpenRGB não encontrado no PATH.{RESET}");
        println!("    Para controlar a placa-mãe ROG e fans ARGB, instale com:");
        println!("    {BOLD}omarchy pkg add openrgb{RESET}");
    }

    let mut root = config::default_config();
    if let TomlValue::Table(tbl) = &mut root
        && let Some(TomlValue::Table(hw)) = tbl.get_mut("hardware") {
            hw.insert("gpu_backend".into(), TomlValue::String(gpu_backend.into()));
            hw.insert("gpu_i2c_bus".into(), TomlValue::String(gpu_bus));
            hw.insert("gpu_i2c_addr".into(), TomlValue::Integer(0x67));
            hw.insert("motherboard_backend".into(), TomlValue::String(motherboard_backend.into()));
            hw.insert("openrgb_devices".into(), TomlValue::String("all".into()));
        }

    println!();
    println!("{BLUE}==> [4/4] Gravando configuração e serviços...{RESET}");
    let config_path = save_config(&root)?;
    println!("    {GREEN}✔ Configuração salva com sucesso em:{RESET} {}", config_path.display());

    install_service(non_interactive)?;

    println!();
    println!("{BOLD}{GREEN}Setup concluído com sucesso!{RESET}");
    println!("Use {BOLD}omaaura status{RESET} para verificar o estado ou {BOLD}omaaura sync{RESET} para sincronizar o tema.");
    Ok(root)
}

fn service_dest() -> PathBuf {
    config::home().join(".config/systemd/user/omaaura.service")
}

fn install_service(non_interactive: bool) -> Result<()> {
    if !ask_yes_no(
        "Deseja instalar o serviço de sincronização automática no systemd --user?",
        true,
        non_interactive,
    ) {
        return Ok(());
    }

    let dest = service_dest();
    if let Some(parent) = dest.parent() {
        std::fs::create_dir_all(parent)?;
    }

    // Ownership guard (addresses marketplace objection #2): do not silently
    // clobber a service unit that does not belong to this plugin.
    if let Ok(existing) = std::fs::read_to_string(&dest)
        && !existing.contains(SERVICE_MARKER) {
            let backup = dest.with_extension("service.omaaura-bak");
            std::fs::copy(&dest, &backup)?;
            println!(
                "    {YELLOW}⚠ {RESET}Serviço existente não pertence ao OmaAura; backup em {}",
                backup.display()
            );
        }

    std::fs::write(&dest, SERVICE_UNIT)?;
    println!("    {GREEN}✔ Arquivo omaaura.service copiado para {}{RESET}", dest.display());

    let _ = Command::new("systemctl").args(["--user", "daemon-reload"]).status();
    if ask_yes_no("Deseja habilitar e iniciar agora o serviço omaaura.service?", true, non_interactive) {
        match Command::new("systemctl").args(["--user", "enable", "--now", "omaaura.service"]).status() {
            Ok(_) => println!("    {GREEN}✔ omaaura.service habilitado e iniciado!{RESET}"),
            Err(e) => println!("    {YELLOW}⚠ Aviso ao configurar systemd: {e}{RESET}"),
        }
    }
    Ok(())
}

