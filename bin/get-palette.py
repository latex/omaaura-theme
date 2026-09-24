#!/usr/bin/env python3
"""OmaAura palette generator.

Produces the JSON consumed by ``BarWidget.qml`` containing:

* ``theme``      -> the Omarchy theme palette (accent, red, …)
* ``background`` -> dominant/vibrant colors extracted from the wallpaper

Every entry carries the **raw** source color as ``hex`` and the **calibrated**
LED color as ``displayHex``.  ``displayHex`` is computed with the exact same
calibration used when writing to hardware, so the swatch the user clicks is
literally the color the LEDs will show.
"""

import json
import subprocess
import colorsys
import sys
import tomllib
from pathlib import Path

PACKAGE_ROOT = Path(__file__).resolve().parent.parent
if str(PACKAGE_ROOT) not in sys.path:
    sys.path.insert(0, str(PACKAGE_ROOT))

from omaaura.core.color import (  # noqa: E402
    DEFAULT_SATURATION_FLOOR,
    DEFAULT_VALUE_TARGET,
    calibrate_hash,
)
from omaaura.core.config import load_config  # noqa: E402


def _calibration_params():
    theme = load_config().get("theme", {})
    return {
        "saturation_floor": float(theme.get("saturation_floor", DEFAULT_SATURATION_FLOOR)),
        "value_target": float(theme.get("value_target", DEFAULT_VALUE_TARGET)),
    }


def preview(hex_code, params=None):
    """Return the calibrated '#rrggbb' that will be sent to the LEDs."""
    params = params or _calibration_params()
    return calibrate_hash(hex_code, **params)


def _entry(name, hex_code, params):
    hex_code = "#" + hex_code.lstrip("#").lower()
    return {"name": name, "hex": hex_code, "displayHex": preview(hex_code, params)}


def get_palette():
    params = _calibration_params()
    theme_colors = []
    theme_colors_path = Path.home() / ".local/state/omarchy/current/theme/colors.toml"
    if theme_colors_path.exists():
        try:
            data = tomllib.loads(theme_colors_path.read_text())
            keys = ["accent", "red", "orange", "yellow", "green", "cyan", "blue", "magenta"]
            for k in keys:
                if k in data and isinstance(data[k], str) and data[k].startswith("#"):
                    theme_colors.append(_entry(k, data[k], params))
        except Exception:
            pass

    bg_colors = []
    bg_path = Path.home() / ".local/state/omarchy/current/background"
    if bg_path.exists():
        try:
            real_bg = bg_path.resolve()
            cmd = [
                "magick", str(real_bg), "-scale", "64x64!", "-depth", "8",
                "+dither", "-colors", "16", "-format", "%c", "histogram:info:",
            ]
            lines = subprocess.check_output(cmd, stderr=subprocess.DEVNULL).decode().splitlines()
            entries = []
            for line in lines:
                parts = line.strip().split()
                if len(parts) >= 3 and parts[2].startswith("#"):
                    hex_code = parts[2][1:7].lower()
                    count = int(parts[0].rstrip(":"))
                    r = int(hex_code[0:2], 16) / 255.0
                    g = int(hex_code[2:4], 16) / 255.0
                    b = int(hex_code[4:6], 16) / 255.0
                    h, s, v = colorsys.rgb_to_hsv(r, g, b)
                    entries.append({"hex": "#" + hex_code, "count": count, "h": h, "s": s, "v": v})

            if entries:
                # 1. Cor dominante mais frequente
                dom = max(entries, key=lambda x: x["count"])
                bg_colors.append(_entry("Fundo Dominante", dom["hex"], params))

                # 2. Cor de destaque / vibrante da cena
                vibrants = [e for e in entries if e["s"] > 0.15 and e["v"] > 0.15]
                if vibrants:
                    vib = max(vibrants, key=lambda x: x["count"] * (x["s"] ** 0.7) * (x["v"] ** 0.7))
                    if vib["hex"] != dom["hex"]:
                        bg_colors.append(_entry("Destaque Wallpaper", vib["hex"], params))

                # 3. Cor secundária frequente
                secondary = [e for e in entries if e["hex"] != dom["hex"]]
                if secondary and len(bg_colors) < 3:
                    sec = max(secondary, key=lambda x: x["count"])
                    bg_colors.append(_entry("Secundária Wallpaper", sec["hex"], params))
        except Exception:
            pass

    return {"theme": theme_colors, "background": bg_colors}


if __name__ == "__main__":
    print(json.dumps(get_palette()))
