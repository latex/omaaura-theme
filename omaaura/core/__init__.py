"""OmaAura Core Package."""

from .color import calibrate_hash, calibrate_hex, parse_hex
from .config import get_config_path, load_config, save_config
from .hardware import (
    detect_all_hardware,
    detect_i2c_adapters,
    detect_openrgb_devices,
    is_openrgb_available,
    set_ene_color,
    set_openrgb_color,
    test_ene_aura,
)

__all__ = [
    "calibrate_hash",
    "calibrate_hex",
    "parse_hex",
    "get_config_path",
    "load_config",
    "save_config",
    "detect_all_hardware",
    "detect_i2c_adapters",
    "detect_openrgb_devices",
    "is_openrgb_available",
    "set_ene_color",
    "set_openrgb_color",
    "test_ene_aura",
]
