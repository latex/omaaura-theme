"""Shared LED color calibration for OmaAura.

Why calibration is needed
-------------------------
Monitors are sRGB emissive surfaces: a "pastel" color such as Catppuccin's
red ``#ed8796`` (saturation ~0.43) still reads clearly as *red* on screen.

Addressable LEDs (ARGB strips, Aura headers) behave differently: they run at a
much higher luminance and mix additively, so a low-saturation color bleeds into
the neighboring channels and is perceived as *washed-out white with a hint of
color* (the classic "white with 10% red" complaint).

To make the physical LED match what the user sees in the widget swatch we
normalise every color before writing it to hardware:

* clamp **saturation** to a minimum floor (so pastel -> vivid), and
* drive **value** to full brightness (bright, saturated LEDs).

The transform is intentionally **idempotent** (it uses floors, not
multipliers), so it can safely run twice: once to build the UI preview and
again at apply time, without the color drifting.

This module is the single source of truth for the calibration.  Both the CLI
(`bin/omaaura`), the palette generator (`bin/get-palette.py`) and the widget
preview consume it, which is what guarantees ``icone == LED``.
"""

from __future__ import annotations

import colorsys

# A color below this saturation is considered a grey/white and is emitted as
# pure white (there is no hue worth preserving).  Dark wallpapers frequently
# yield near-black "colors" (e.g. #060707) that must not be turned into an
# arbitrary saturated hue.
GREY_SATURATION = 0.15

# Defaults.  Exposed through ``~/.config/omaaura/config.toml`` so they can be
# tuned per-monitor without editing code.
#
# ``saturation_floor = 1.0`` renders the pure hue at full brightness, which is
# what makes pastel theme colors (Catppuccin's red #ed8796 etc.) finally look
# *red* on additive LEDs instead of "white with a hint of red".
DEFAULT_SATURATION_FLOOR = 1.0
DEFAULT_VALUE_TARGET = 1.0
DEFAULT_FALLBACK = "89b4fa"


def parse_hex(hex_code: str) -> tuple[float, float, float]:
    """Return normalised ``(r, g, b)`` floats in the 0.0-1.0 range."""
    clean = hex_code.lstrip("#").strip()
    if len(clean) != 6:
        raise ValueError(f"Invalid hex color: {hex_code!r}")
    return (
        int(clean[0:2], 16) / 255.0,
        int(clean[2:4], 16) / 255.0,
        int(clean[4:6], 16) / 255.0,
    )


def calibrate_hex(
    hex_code: str,
    saturation_floor: float = DEFAULT_SATURATION_FLOOR,
    value_target: float = DEFAULT_VALUE_TARGET,
    fallback: str = DEFAULT_FALLBACK,
) -> str:
    """Calibrate a hex color for LED hardware, returning ``rrggbb`` (no ``#``).

    Idempotent: ``calibrate_hex(calibrate_hex(x)) == calibrate_hex(x)`` because
    only saturation/value floors are applied.
    """
    try:
        r, g, b = parse_hex(hex_code)
    except (ValueError, TypeError):
        return fallback

    h, s, v = colorsys.rgb_to_hsv(r, g, b)

    if s < GREY_SATURATION:
        # Neutral color: no hue to saturate, keep it bright white.
        return "ffffff"

    s = max(s, saturation_floor)
    v = max(v, value_target)
    s = min(1.0, max(0.0, s))
    v = min(1.0, max(0.0, v))

    rf, gf, bf = colorsys.hsv_to_rgb(h, s, v)
    return f"{int(round(rf * 255)):02x}{int(round(gf * 255)):02x}{int(round(bf * 255)):02x}"


def calibrate_hash(
    hex_code: str,
    saturation_floor: float = DEFAULT_SATURATION_FLOOR,
    value_target: float = DEFAULT_VALUE_TARGET,
    fallback: str = DEFAULT_FALLBACK,
) -> str:
    """Same as :func:`calibrate_hex` but returns a ``#rrggbb`` string."""
    return "#" + calibrate_hex(
        hex_code,
        saturation_floor=saturation_floor,
        value_target=value_target,
        fallback=fallback,
    )
