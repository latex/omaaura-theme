"""Hardware detection and LED control for OmaAura.

Supports:
1. Direct Linux I2C ioctl communication for ENE Aura controllers (e.g. ASUS TUF / ROG GPUs at 0x67).
2. OpenRGB integration for ASUS ROG motherboards, ARGB headers, and RAM.
"""

import ctypes
import fcntl
import json
import os
import re
import shutil
import subprocess
import sys
import time
from pathlib import Path
from typing import Any, Dict, List, Optional, Tuple

# Linux I2C ioctl definitions
I2C_RDWR = 0x0707


class _I2CMsg(ctypes.Structure):
    _fields_ = [
        ("addr", ctypes.c_uint16),
        ("flags", ctypes.c_uint16),
        ("len", ctypes.c_uint16),
        ("buf", ctypes.POINTER(ctypes.c_uint8)),
    ]


class _I2CRdwrIoctlData(ctypes.Structure):
    _fields_ = [
        ("msgs", ctypes.POINTER(_I2CMsg)),
        ("nmsgs", ctypes.c_uint32),
    ]


def detect_i2c_adapters() -> List[Dict[str, Any]]:
    """Detect all Linux I2C adapter buses and their access permissions."""
    adapters = []
    sys_bus = Path("/sys/bus/i2c/devices")

    if not sys_bus.exists():
        return adapters

    for item in sorted(sys_bus.glob("i2c-*")):
        name_file = item / "name"
        bus_name = name_file.read_text().strip() if name_file.exists() else "Unknown"
        dev_path = f"/dev/{item.name}"
        exists = os.path.exists(dev_path)
        readable = os.access(dev_path, os.R_OK) if exists else False
        writable = os.access(dev_path, os.W_OK) if exists else False

        is_nvidia = "nvidia" in bus_name.lower()
        is_smbus = "smbus" in bus_name.lower() or "piix4" in bus_name.lower()

        adapters.append(
            {
                "id": item.name,
                "dev_path": dev_path,
                "name": bus_name,
                "exists": exists,
                "readable": readable,
                "writable": writable,
                "is_nvidia": is_nvidia,
                "is_smbus": is_smbus,
            }
        )

    return adapters


def _i2c_write(fd: int, addr: int, data: List[int]) -> None:
    buf = (ctypes.c_uint8 * len(data))(*data)
    msg = _I2CMsg(addr, 0, len(data), buf)
    data_struct = _I2CRdwrIoctlData(ctypes.pointer(msg), 1)
    ret = fcntl.ioctl(fd, I2C_RDWR, data_struct)
    if ret < 0:
        raise OSError(f"ioctl I2C_RDWR returned {ret}")


def _ene_write_reg(fd: int, addr: int, reg: int, val: int) -> None:
    _i2c_write(fd, addr, [0x00, (reg >> 8) & 0xFF, reg & 0xFF])
    _i2c_write(fd, addr, [0x01, val & 0xFF])


def _ene_write_block(fd: int, addr: int, reg: int, byte_list: List[int]) -> None:
    _i2c_write(fd, addr, [0x00, (reg >> 8) & 0xFF, reg & 0xFF])
    for val in byte_list:
        _i2c_write(fd, addr, [0x01, val & 0xFF])


def set_ene_color(
    hex_str: str,
    dev_path: str = "/dev/i2c-1",
    addr: int = 0x67,
) -> bool:
    """Set RGB color on ENE Aura chip (ASUS GPUs) directly via Linux I2C ioctl."""
    clean_hex = hex_str.lstrip("#").strip()
    if len(clean_hex) != 6:
        raise ValueError(f"Invalid hex color string: '{hex_str}'")

    r = int(clean_hex[0:2], 16)
    g = int(clean_hex[2:4], 16)
    b = int(clean_hex[4:6], 16)

    if not os.path.exists(dev_path):
        return False

    fd = -1
    try:
        fd = os.open(dev_path, os.O_RDWR)
        mode = 1 if (r > 0 or g > 0 or b > 0) else 0

        _ene_write_reg(fd, addr, 0x8021, mode)
        _ene_write_reg(fd, addr, 0x8022, 2)
        _ene_write_reg(fd, addr, 0x8023, 0)
        _ene_write_reg(fd, addr, 0x8020, 0)

        # ENE Aura controllers use RBG byte ordering
        rbg_block = [r, b, g] * 4

        # V2 registers (0x8160 effect, 0x8100 direct)
        _ene_write_block(fd, addr, 0x8160, rbg_block)
        _ene_write_block(fd, addr, 0x8100, rbg_block)

        # V1 registers (0x8010 effect, 0x8000 direct)
        _ene_write_block(fd, addr, 0x8010, rbg_block)
        _ene_write_block(fd, addr, 0x8000, rbg_block)

        # Latch and commit changes
        _ene_write_reg(fd, addr, 0x80A0, 0x01)
        time.sleep(0.02)
        _ene_write_reg(fd, addr, 0x80A0, 0xAA)
        return True
    finally:
        if fd >= 0:
            try:
                os.close(fd)
            except OSError:
                pass


def test_ene_aura(
    dev_path: str = "/dev/i2c-1",
    addr: int = 0x67,
) -> Dict[str, Any]:
    """Test I2C communication with ENE Aura controller at given device and address."""
    result: Dict[str, Any] = {
        "dev_path": dev_path,
        "addr": hex(addr),
        "exists": False,
        "permission_ok": False,
        "success": False,
        "message": "",
    }

    if not os.path.exists(dev_path):
        result["message"] = f"Device node {dev_path} does not exist."
        return result

    result["exists"] = True
    can_rw = os.access(dev_path, os.R_OK | os.W_OK)
    result["permission_ok"] = can_rw

    if not can_rw:
        result["message"] = (
            f"Insufficient permissions to access {dev_path}. "
            "Ensure your user is in the 'i2c' group or udev rules are set."
        )
        return result

    fd = -1
    try:
        fd = os.open(dev_path, os.O_RDWR)
        # Probe address with a harmless register select (0x8020 mode register)
        _i2c_write(fd, addr, [0x00, 0x80, 0x20])
        result["success"] = True
        result["message"] = f"Successfully communicated with ENE Aura at {dev_path} (addr {hex(addr)})."
    except Exception as e:
        result["success"] = False
        result["message"] = f"Failed to communicate with ENE Aura at {dev_path}: {e}"
    finally:
        if fd >= 0:
            try:
                os.close(fd)
            except OSError:
                pass

    return result


def is_openrgb_available() -> bool:
    """Check if openrgb executable is installed in PATH."""
    return shutil.which("openrgb") is not None


# GPUs are driven directly through the ENE I2C backend.  OpenRGB must *not*
# touch them, otherwise it fights the direct write and turns the GPU off or
# leaves it in a stale firmware mode.
_GPU_NAME_PATTERN = re.compile(
    r"geforce|radeon|\brtx\b|\bgtx\b|\brx\s?\d|nvidia|quadro|arc\s+a\d|intel arc",
    re.IGNORECASE,
)


def _is_gpu_device(name: str) -> bool:
    return bool(_GPU_NAME_PATTERN.search(name or ""))


# Detecting OpenRGB devices costs ~8s (OpenRGB re-enumerates everything on every
# launch).  Cache the resolved non-GPU target IDs for a while so a color change
# only pays for a single OpenRGB invocation.
_TARGETS_TTL_SEC = 1800.0


def _targets_cache_file() -> Path:
    state_dir = Path(os.environ.get("XDG_STATE_HOME", Path.home() / ".local/state")) / "omaaura-theme"
    return state_dir / "openrgb_targets.json"


def _read_targets_cache() -> Optional[List[int]]:
    path = _targets_cache_file()
    try:
        data = json.loads(path.read_text())
        if time.time() - float(data["ts"]) <= _TARGETS_TTL_SEC:
            ids = data.get("ids")
            if isinstance(ids, list) and ids:
                return [int(x) for x in ids]
    except Exception:
        pass
    return None


def _write_targets_cache(ids: List[int]) -> None:
    path = _targets_cache_file()
    try:
        path.parent.mkdir(parents=True, exist_ok=True)
        path.write_text(json.dumps({"ts": time.time(), "ids": [int(x) for x in ids]}))
    except Exception:
        pass


def resolve_openrgb_targets(device_ids: Optional[Any]) -> Optional[List[int]]:
    """Resolve which OpenRGB device IDs should receive the color.

    Returns a list of IDs, or ``None`` to mean "apply to every device".

    ``"all"`` is interpreted as "every *non-GPU* device" so that the
    motherboard, ARGB headers and RAM are covered while the GPU keeps being
    handled by the direct I2C backend.
    """
    if device_ids is None or device_ids == "all":
        cached = _read_targets_cache()
        if cached is not None:
            return cached
        devices = detect_openrgb_devices()
        non_gpu = [int(d["id"]) for d in devices if not _is_gpu_device(d.get("name", ""))]
        if devices and non_gpu:
            _write_targets_cache(non_gpu)
            return non_gpu
        # Detection unavailable: be conservative and let OpenRGB do its default.
        return None
    if isinstance(device_ids, (list, tuple)):
        return [int(x) for x in device_ids]
    if isinstance(device_ids, int):
        return [device_ids]
    return None


def detect_openrgb_devices(timeout: float = 12.0) -> List[Dict[str, Any]]:
    """List detected OpenRGB devices by running `openrgb --list-devices`."""
    if not is_openrgb_available():
        return []

    try:
        proc = subprocess.run(
            ["openrgb", "--list-devices"],
            capture_output=True,
            text=True,
            timeout=timeout,
        )
        output = proc.stdout
        devices = []
        pattern = re.compile(r"^(\d+):\s*(.+)$")
        for line in output.splitlines():
            line_str = line.strip()
            match = pattern.match(line_str)
            if match:
                dev_id = int(match.group(1))
                dev_name = match.group(2).strip()
                devices.append({"id": dev_id, "name": dev_name})
        return devices
    except Exception:
        return []


def set_openrgb_color(
    hex_str: str,
    device_ids: Optional[Any] = "all",
    timeout: float = 25.0,
) -> bool:
    """Set RGB color using OpenRGB CLI.

    Unless explicit device IDs are given, the color is applied to every
    OpenRGB device **except GPUs** (which are handled by the ENE I2C backend).
    """
    if not is_openrgb_available():
        return False

    clean_hex = hex_str.lstrip("#").strip()
    if len(clean_hex) != 6:
        raise ValueError(f"Invalid hex color string: '{hex_str}'")

    targets = resolve_openrgb_targets(device_ids)

    def _run(args: List[str]) -> bool:
        proc = subprocess.run(
            args,
            stdout=subprocess.DEVNULL,
            stderr=subprocess.DEVNULL,
            timeout=timeout,
            check=False,
        )
        if proc.returncode != 0:
            print(
                f"Warning: OpenRGB command {' '.join(args)} exited with code {proc.returncode}",
                file=sys.stderr,
            )
            return False
        return True

    if targets is None:
        # Could not determine devices: apply to all (OpenRGB default).
        return _run(["openrgb", "-c", clean_hex])

    ok = True
    for dev_id in targets:
        ok = _run(["openrgb", "-d", str(dev_id), "-c", clean_hex]) and ok
    return ok


def detect_all_hardware() -> Dict[str, Any]:
    """Perform a full hardware diagnostic check."""
    adapters = detect_i2c_adapters()

    # Find the most likely GPU I2C bus (NVIDIA adapter)
    gpu_bus = "/dev/i2c-1"
    nvidia_buses = [a["dev_path"] for a in adapters if a.get("is_nvidia")]
    if nvidia_buses:
        # Default to first NVIDIA adapter if i2c-1 is among them
        if "/dev/i2c-1" in nvidia_buses:
            gpu_bus = "/dev/i2c-1"
        else:
            gpu_bus = nvidia_buses[0]

    ene_test = test_ene_aura(dev_path=gpu_bus, addr=0x67)
    openrgb_avail = is_openrgb_available()
    openrgb_devices = detect_openrgb_devices() if openrgb_avail else []

    return {
        "i2c_adapters": adapters,
        "gpu_bus_candidate": gpu_bus,
        "ene_aura": ene_test,
        "openrgb_installed": openrgb_avail,
        "openrgb_devices": openrgb_devices,
    }
