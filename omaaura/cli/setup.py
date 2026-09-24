"""Interactive setup wizard for OmaAura.

Tests local I2C buses, ENE Aura controllers (ASUS GPUs), and OpenRGB devices,
then generates ~/.config/omaaura/config.toml and configures the systemd user service.
"""

import argparse
import os
import shutil
import subprocess
import sys
from pathlib import Path
from typing import Any, Dict

from ..core.config import DEFAULT_CONFIG, get_config_path, load_config, save_config
from ..core.hardware import (
    detect_all_hardware,
    is_openrgb_available,
    set_ene_color,
    test_ene_aura,
)

# Terminal color codes
C_RESET = "\033[0m"
C_BOLD = "\033[1m"
C_GREEN = "\033[32m"
C_YELLOW = "\033[33m"
C_BLUE = "\033[34m"
C_MAGENTA = "\033[35m"
C_CYAN = "\033[36m"
C_RED = "\033[31m"


def _print_header() -> None:
    print(f"{C_BOLD}{C_CYAN}")
    print("╔═══════════════════════════════════════════════════════╗")
    print("║            OmaAura Hardware & RGB Setup Wizard        ║")
    print("║          ASUS Aura & Omarchy Theme Integrator         ║")
    print("╚═══════════════════════════════════════════════════════╝")
    print(f"{C_RESET}")


def _ask_yes_no(prompt: str, default: bool = True, non_interactive: bool = False) -> bool:
    if non_interactive:
        return default
    choice = " [Y/n]: " if default else " [y/N]: "
    try:
        ans = input(f"{C_BOLD}{prompt}{choice}{C_RESET}").strip().lower()
        if not ans:
            return default
        return ans in ("y", "yes", "s", "sim")
    except (EOFError, KeyboardInterrupt):
        print()
        return default


def run_setup(interactive: bool = True) -> Dict[str, Any]:
    """Execute the OmaAura setup wizard."""
    _print_header()
    non_interactive = not interactive

    print(f"{C_BLUE}==> [1/4] Sondando barramentos I2C e hardware de GPU...{C_RESET}")
    diag = detect_all_hardware()
    adapters = diag["i2c_adapters"]
    gpu_bus = diag["gpu_bus_candidate"]
    ene_status = diag["ene_aura"]

    print(f"    Barramentos I2C encontrados: {len(adapters)}")
    for ad in adapters:
        tag = f"{C_GREEN}[NVIDIA GPU]{C_RESET}" if ad.get("is_nvidia") else f"{C_YELLOW}[SMBus]{C_RESET}" if ad.get("is_smbus") else "[Outro]"
        rw_status = f"{C_GREEN}RW OK{C_RESET}" if ad.get("writable") else f"{C_RED}Sem permissão W{C_RESET}"
        print(f"    - {ad['dev_path']}: {ad['name']} | {tag} | {rw_status}")

    print()
    print(f"{C_BLUE}==> [2/4] Testando controlador ENE Aura (0x67) na GPU...{C_RESET}")
    gpu_backend = "none"
    if ene_status.get("success"):
        print(f"    {C_GREEN}✔ Controlador ENE Aura detectado com sucesso em {gpu_bus} (0x67)!{C_RESET}")
        gpu_backend = "ene_i2c"

        if interactive:
            test_rgb = _ask_yes_no("    Deseja testar um pulso de luz (Ciano) na GPU agora?", default=True)
            if test_rgb:
                try:
                    set_ene_color("89dceb", dev_path=gpu_bus, addr=0x67)
                    print(f"    {C_GREEN}✔ Pulso de iluminação enviado com sucesso.{C_RESET}")
                except Exception as ex:
                    print(f"    {C_RED}✘ Erro no teste de iluminação: {ex}{C_RESET}")
    else:
        print(f"    {C_YELLOW}⚠ ENE Aura: {ene_status.get('message')}{C_RESET}")
        if not ene_status.get("permission_ok") and ene_status.get("exists"):
            print(f"    {C_RED}Aviso: Para acesso sem root ao I2C, adicione seu usuário ao grupo 'i2c':{C_RESET}")
            print(f"    {C_BOLD}sudo usermod -aG i2c $USER{C_RESET}")

    print()
    print(f"{C_BLUE}==> [3/4] Verificando integração com OpenRGB...{C_RESET}")
    motherboard_backend = "none"
    openrgb_devices_cfg: Any = "all"
    if diag.get("openrgb_installed"):
        print(f"    {C_GREEN}✔ OpenRGB está instalado no sistema.{C_RESET}")
        devices = diag.get("openrgb_devices", [])
        if devices:
            print(f"    Dispositivos detectados ({len(devices)}):")
            for dev in devices:
                print(f"      [{dev['id']}] {dev['name']}")
            motherboard_backend = "openrgb"
        else:
            print(f"    {C_YELLOW}ℹ Nenhum dispositivo ativo listado no momento pelo OpenRGB.{C_RESET}")
            motherboard_backend = "openrgb"
    else:
        print(f"    {C_YELLOW}⚠ OpenRGB não encontrado no PATH.{C_RESET}")
        print("    Para controlar a placa-mãe ROG e fans ARGB, instale com:")
        print(f"    {C_BOLD}omarchy pkg add openrgb{C_RESET}")

    # Build configuration
    config = {
        "hardware": {
            "gpu_backend": gpu_backend,
            "gpu_i2c_bus": gpu_bus,
            "gpu_i2c_addr": 0x67,
            "motherboard_backend": motherboard_backend,
            "openrgb_devices": openrgb_devices_cfg,
        },
        "theme": {
            "sync_mode": "accent",
            "calibrate_led": True,
            "saturation_floor": 1.0,
            "value_target": 1.0,
            "fallback_color": "89b4fa",
        },
        "service": {
            "enabled": True,
            "poll_interval_sec": 2.0,
        },
    }

    print()
    print(f"{C_BLUE}==> [4/4] Gravando configuração e serviços...{C_RESET}")
    config_path = save_config(config)
    print(f"    {C_GREEN}✔ Configuração salva com sucesso em:{C_RESET} {config_path}")

    # Systemd user service installation
    service_src = Path(__file__).resolve().parent.parent / "service" / "omaaura.service"
    service_dst_dir = Path.home() / ".config" / "systemd" / "user"
    service_dst = service_dst_dir / "omaaura.service"

    install_service = _ask_yes_no(
        "Deseja instalar o serviço de sincronização automática no systemd --user?",
        default=True,
        non_interactive=non_interactive,
    )

    if install_service and service_src.exists():
        service_dst_dir.mkdir(parents=True, exist_ok=True)
        shutil.copy2(service_src, service_dst)
        print(f"    {C_GREEN}✔ Arquivo omaaura.service copiado para {service_dst}{C_RESET}")

        try:
            subprocess.run(["systemctl", "--user", "daemon-reload"], check=False)
            enable_service = _ask_yes_no(
                "Deseja habilitar e iniciar agora o serviço omaaura.service?",
                default=True,
                non_interactive=non_interactive,
            )
            if enable_service:
                subprocess.run(["systemctl", "--user", "enable", "--now", "omaaura.service"], check=False)
                print(f"    {C_GREEN}✔ omaaura.service habilitado e iniciado!{C_RESET}")
        except Exception as err:
            print(f"    {C_YELLOW}⚠ Aviso ao configurar systemd: {err}{C_RESET}")

    print()
    print(f"{C_BOLD}{C_GREEN}Setup concluído com sucesso!{C_RESET}")
    print(f"Use {C_BOLD}omaaura status{C_RESET} para verificar o estado ou {C_BOLD}omaaura sync{C_RESET} para sincronizar o tema.")
    return config


def main() -> None:
    parser = argparse.ArgumentParser(description="OmaAura Setup Wizard")
    parser.add_argument(
        "--yes",
        "-y",
        action="store_true",
        help="Execute without interactive prompts using detected best defaults",
    )
    args = parser.parse_args()
    run_setup(interactive=not args.yes)


if __name__ == "__main__":
    main()
