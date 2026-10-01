//! `omaaura probe` — USB hardware fingerprint & RGB controllability classifier.
//!
//! Answers the question "can this device's RGB be controlled by software?" from
//! the hardware itself:
//!
//! * enumerates USB devices from sysfs (no root needed);
//! * inspects HID report descriptors for LED (0x08), LampArray (0x59) and
//!   vendor (0xFFxx) usage pages;
//! * cross-references the backend registry to mark already-controllable devices;
//! * consults a small knowledge base of known button-only devices;
//! * `--deep` (root) enumerates hidden string descriptors and probes the
//!   C-Media vendor register/flash read commands.

use std::collections::HashSet;
use std::os::fd::AsRawFd;
use std::path::{Path, PathBuf};

use serde_json::{Value as Json, json};

use crate::backend::registry::Registry;
use crate::config;

/// Knowledge base: known devices → verdict + note (checked before heuristics).
const KNOWN_DEVICES: &[(&str, Verdict, &str)] = &[
    ("0b05:1939", Verdict::Controllable, "ASUS AURA LED Controller (placa-mãe, via OpenRGB)"),
    ("3142:a010", Verdict::ButtonOnly, "FIFINE AM8 — RGB só por botão físico (sem software, nem no Windows)"),
    ("3142:0001", Verdict::ButtonOnly, "FIFINE K670 — config só pela ferramenta Windows CM6400"),
    ("2207:0019", Verdict::NotRgb, "Ulanzi D200/D200H — deck LCD, não é RGB"),
];

/// RGB controllability verdict.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Verdict {
    Controllable,
    GenericHidLed,
    ButtonOnly,
    Unknown,
    NotRgb,
}

impl Verdict {
    fn label(self) -> &'static str {
        match self {
            Self::Controllable => "controlável (backend)",
            Self::GenericHidLed => "controlável (HID LED/LampArray)",
            Self::ButtonOnly => "SÓ BOTÃO (sem software)",
            Self::Unknown => "desconhecido (interface vendor)",
            Self::NotRgb => "não-RGB",
        }
    }
}

#[derive(Debug)]
struct HidInfo {
    name: String,
    usage_pages: Vec<u32>,
}

#[derive(Debug)]
struct UsbDevice {
    devname: String,
    vid: String,
    pid: String,
    manufacturer: String,
    product: String,
    serial: String,
    interfaces: Vec<(String, String, String)>, // (number, class, subclass)
    hid: Vec<HidInfo>,
}

fn read_trim(path: &Path) -> String {
    std::fs::read_to_string(path).map_or_else(|_| String::new(), |s| s.trim().to_string())
}

/// Extracts Usage Page values from a HID report descriptor (minimal item parser).
fn usage_pages(desc: &[u8]) -> Vec<u32> {
    let mut pages = HashSet::new();
    let mut i = 0;
    while i < desc.len() {
        let prefix = desc[i];
        if prefix == 0xFE {
            // Long item: [0xFE, size, tag, data...]
            if i + 2 >= desc.len() {
                break;
            }
            i += 3 + desc[i + 1] as usize;
            continue;
        }
        let size = match prefix & 0x03 {
            0 => 0,
            1 => 1,
            2 => 2,
            _ => 4,
        };
        let itype = (prefix >> 2) & 0x03; // 1 = Global
        let tag = prefix >> 4;
        if i + 1 + size > desc.len() {
            break;
        }
        if itype == 1 && tag == 0 {
            let mut v = 0u32;
            for (k, &b) in desc[i + 1..i + 1 + size].iter().enumerate() {
                v |= u32::from(b) << (8 * k);
            }
            pages.insert(v);
        }
        i += 1 + size;
    }
    let mut v: Vec<u32> = pages.into_iter().collect();
    v.sort_unstable();
    v
}

fn hid_for(vid: &str, pid: &str) -> Vec<HidInfo> {
    let mut out = Vec::new();
    let Ok(entries) = std::fs::read_dir("/sys/class/hidraw") else {
        return out;
    };
    for e in entries.flatten() {
        let dev = e.path().join("device");
        let uevent = read_trim(&dev.join("uevent"));
        let id = uevent.lines().find_map(|l| l.strip_prefix("HID_ID=")).unwrap_or("");
        // HID_ID=0003:0000VID:0000PID
        let upper = id.to_uppercase();
        let want = format!(":{vid}:");
        let tail = format!(":{pid}");
        if !(upper.contains(&want) && upper.ends_with(&tail)) {
            continue;
        }
        let name = uevent.lines().find_map(|l| l.strip_prefix("HID_NAME=")).unwrap_or("").to_string();
        let usage_pages = std::fs::read(dev.join("report_descriptor")).map_or_else(|_| Vec::new(), |d| usage_pages(&d));
        out.push(HidInfo { name, usage_pages });
    }
    out
}

fn collect_devices() -> Vec<UsbDevice> {
    let mut devices = Vec::new();
    let Ok(entries) = std::fs::read_dir("/sys/bus/usb/devices") else {
        return devices;
    };
    for e in entries.flatten() {
        let dir = e.path();
        let vid = read_trim(&dir.join("idVendor"));
        let pid = read_trim(&dir.join("idProduct"));
        if vid.is_empty() {
            continue;
        }
        let devname = dir.file_name().map_or_else(String::new, |n| n.to_string_lossy().into_owned());

        // Interfaces are separate sysfs entries named "<devname>:<cfg>.<iface>".
        let mut interfaces = Vec::new();
        if let Ok(all) = std::fs::read_dir("/sys/bus/usb/devices") {
            for ie in all.flatten() {
                let iname = ie.file_name().to_string_lossy().into_owned();
                if let Some(rest) = iname.strip_prefix(&format!("{devname}:")) {
                    let num = rest.split('.').next_back().unwrap_or("").to_string();
                    let idir = ie.path();
                    interfaces.push((
                        num,
                        read_trim(&idir.join("bInterfaceClass")),
                        read_trim(&idir.join("bInterfaceSubClass")),
                    ));
                }
            }
        }

        devices.push(UsbDevice {
            devname,
            hid: hid_for(&vid, &pid),
            vid,
            pid,
            manufacturer: read_trim(&dir.join("manufacturer")),
            product: read_trim(&dir.join("product")),
            serial: read_trim(&dir.join("serial")),
            interfaces,
        });
    }
    devices.sort_by(|a, b| (&a.vid, &a.pid).cmp(&(&b.vid, &b.pid)));
    devices
}

fn classify(dev: &UsbDevice, controllable_names: &HashSet<String>) -> Verdict {
    let id = format!("{}:{}", dev.vid.to_lowercase(), dev.pid.to_lowercase());
    let product = dev.product.to_lowercase();

    // Device already driven by a backend (name match against detected devices).
    if !product.is_empty()
        && controllable_names.iter().any(|n| !n.is_empty() && (n.contains(&product) || product.contains(n)))
    {
        return Verdict::Controllable;
    }
    if KNOWN_DEVICES.iter().any(|(k, _, _)| *k == id) {
        return KNOWN_DEVICES.iter().find(|(k, _, _)| *k == id).map_or(Verdict::Unknown, |(_, v, _)| *v);
    }
    // HID LED (0x08) or LampArray (0x59) => generically controllable.
    if dev.hid.iter().any(|h| h.usage_pages.iter().any(|p| *p == 0x08 || *p == 0x59)) {
        return Verdict::GenericHidLed;
    }
    // Vendor-defined HID usage page (0xFFxx) => unknown, worth RE.
    if dev.hid.iter().any(|h| h.usage_pages.iter().any(|p| (0xFF00..=0xFFFF).contains(p))) {
        return Verdict::Unknown;
    }
    // Vendor-specific USB interface class 0xFF => unknown.
    if dev.interfaces.iter().any(|(_, class, _)| class == "255") {
        return Verdict::Unknown;
    }
    Verdict::NotRgb
}

fn known_note(dev: &UsbDevice) -> Option<&'static str> {
    let id = format!("{}:{}", dev.vid.to_lowercase(), dev.pid.to_lowercase());
    KNOWN_DEVICES.iter().find(|(k, _, _)| *k == id).map(|(_, _, note)| *note)
}

/// Run the probe. `deep` requires root for hidden strings / vendor commands.
pub fn run(deep: bool, json_out: bool) -> anyhow::Result<()> {
    let cfg = config::load_config();
    let registry = Registry::load();

    // Names of devices already driven by a backend => controllable.
    let mut controllable_names: HashSet<String> = registry
        .detect_all(&cfg)
        .into_iter()
        .map(|d| d.name.to_lowercase())
        .collect();
    controllable_names.extend(
        crate::backend::builtin::openrgb::detect_openrgb_devices()
            .into_iter()
            .map(|d| d.name.to_lowercase()),
    );

    let devices = collect_devices();
    let mut rows = Vec::new();
    for dev in &devices {
        rows.push((dev, classify(dev, &controllable_names)));
    }

    if json_out {
        let arr: Vec<Json> = rows
            .iter()
            .map(|(d, v)| {
                json!({
                    "usb": format!("{}:{}", d.vid, d.pid),
                    "sysfs": d.devname,
                    "manufacturer": d.manufacturer,
                    "product": d.product,
                    "serial": d.serial,
                    "verdict": v.label(),
                    "note": known_note(d),
                    "hid": d.hid.iter().map(|h| json!({
                        "name": h.name,
                        "usage_pages": h.usage_pages.iter().map(|p| format!("0x{p:04X}")).collect::<Vec<_>>(),
                    })).collect::<Vec<_>>(),
                })
            })
            .collect();
        println!("{}", serde_json::to_string_pretty(&arr)?);
        return Ok(());
    }

    println!("{:<11} {:<34} VEREDITO RGB", "USB", "DISPOSITIVO");
    for (d, v) in &rows {
        let name = if d.product.is_empty() { d.manufacturer.clone() } else { d.product.clone() };
        let color = match v {
            Verdict::Controllable | Verdict::GenericHidLed => "\u{1b}[32m",
            Verdict::ButtonOnly => "\u{1b}[33m",
            Verdict::Unknown => "\u{1b}[36m",
            Verdict::NotRgb => "\u{1b}[90m",
        };
        println!("{:<11} {:<34} {color}{}\u{1b}[0m", format!("{}:{}", d.vid, d.pid), name, v.label());
        if let Some(n) = known_note(d) {
            println!("            └─ {n}");
        }
    }

    if deep {
        deep_probe();
    } else {
        println!("\n(dica: `omaaura probe --deep` com sudo enumera strings ocultas e testa o protocolo vendor C-Media)");
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// Deep probe (root): hidden USB string descriptors + C-Media vendor reads
// ---------------------------------------------------------------------------

const USBDEVFS_CONTROL: libc::c_ulong = 0xC018_5500;

#[repr(C)]
struct CtrlTransfer {
    b_request_type: u8,
    b_request: u8,
    w_value: u16,
    w_index: u16,
    w_length: u16,
    timeout: u32,
    data: *mut u8,
}

fn control(fd: i32, req_type: u8, req: u8, value: u16, index: u16, buf: &mut [u8]) -> Result<usize, i32> {
    let mut t = CtrlTransfer {
        b_request_type: req_type,
        b_request: req,
        w_value: value,
        w_index: index,
        w_length: buf.len() as u16,
        timeout: 1500,
        data: buf.as_mut_ptr(),
    };
    // SAFETY: kernel reads the struct and the buffer we pass.
    let ret = unsafe { libc::ioctl(fd, USBDEVFS_CONTROL, &mut t) };
    if ret < 0 { Err(std::io::Error::last_os_error().raw_os_error().unwrap_or(-1)) } else { Ok(ret as usize) }
}

fn device_node(dir: &Path) -> Option<PathBuf> {
    let bus: u32 = read_trim(&dir.join("busnum")).parse().ok()?;
    let dev: u32 = read_trim(&dir.join("devnum")).parse().ok()?;
    Some(PathBuf::from(format!("/dev/bus/usb/{bus:03}/{dev:03}")))
}

fn deep_probe() {
    println!("\n== deep probe (root) ==");
    let Ok(entries) = std::fs::read_dir("/sys/bus/usb/devices") else {
        return;
    };
    for e in entries.flatten() {
        let dir = e.path();
        if read_trim(&dir.join("idVendor")).is_empty() {
            continue;
        }
        let Some(node) = device_node(&dir) else { continue };
        let Ok(file) = std::fs::OpenOptions::new().read(true).write(true).open(&node) else {
            println!("  {} ({}): sem acesso — rode com sudo", node.display(), read_trim(&dir.join("product")));
            continue;
        };
        let fd = file.as_raw_fd();
        let label = read_trim(&dir.join("product"));
        let vid = read_trim(&dir.join("idVendor"));
        let pid = read_trim(&dir.join("idProduct"));
        println!("  {vid}:{pid} \"{label}\"");

        // Hidden string descriptors.
        let mut buf = [0u8; 255];
        for i in 0u16..8 {
            if let Ok(n) = control(fd, 0x80, 0x06, (0x03 << 8) | i, 0x0409, &mut buf)
                && n >= 2 {
                    let s = decode_utf16le(&buf[2..n]);
                    if !s.trim().is_empty() {
                        println!("     str[{i}] = {s:?}");
                    }
                }
        }
        // C-Media vendor read (register + flash).
        for (req, addr, what) in [(0x02u8, 0x0000u16, "reg"), (0x04, 0x0000, "flash")] {
            match control(fd, 0xC3, req, addr, 0, &mut buf) {
                Ok(n) => println!("     cmedia {what} read: {n}B {}", to_ascii(&buf[..n.min(24)])),
                Err(e) => println!("     cmedia {what} read: errno={e} (não suportado)"),
            }
        }
    }
}

fn decode_utf16le(bytes: &[u8]) -> String {
    let mut s = String::new();
    for c in bytes.as_chunks::<2>().0 {
        let u = u16::from_le_bytes([c[0], c[1]]);
        if u == 0 {
            break;
        }
        s.push(char::from_u32(u32::from(u)).unwrap_or('?'));
    }
    s
}

fn to_ascii(bytes: &[u8]) -> String {
    bytes.iter().map(|&b| if (0x20..0x7f).contains(&b) { b as char } else { '.' }).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_usage_pages() {
        // FIFINE AM8 consumer-control descriptor.
        let desc = [
            0x05, 0x0C, 0x09, 0x01, 0xA1, 0x01, 0x15, 0x00, 0x25, 0x01, 0x09, 0xE9, 0x09, 0xEA, 0xB5, 0x09,
            0xB6, 0x09, 0xE2, 0x09, 0xB3, 0x09, 0xCD, 0x09, 0xB7, 0x75, 0x01, 0x95, 0x08, 0x81, 0x42, 0xC0,
        ];
        assert_eq!(usage_pages(&desc), vec![0x0C]);
    }
}
