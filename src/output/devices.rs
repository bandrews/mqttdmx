// ABOUTME: Finds serial devices (through sysfs on Linux) and picks out Enttec DMX USB Pro interfaces.
// ABOUTME: Also produces the listing printed by --list-devices.

use std::fmt::Write as _;
use std::path::Path;

use serialport::SerialPortType;

#[derive(Debug, Clone, PartialEq)]
pub struct UsbDetails {
    pub vid: u16,
    pub pid: u16,
    pub manufacturer: Option<String>,
    pub product: Option<String>,
    pub serial_number: Option<String>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct SerialDevice {
    pub path: String,
    pub usb: Option<UsbDetails>,
}

/// Every serial device the system knows about. On Linux, empty when sysfs is
/// not available, for example in a container started without /sys.
pub fn serial_devices() -> Vec<SerialDevice> {
    // On Linux serialport panics when /sys/class/tty is missing, so check first.
    if cfg!(target_os = "linux") && !Path::new("/sys/class/tty").is_dir() {
        return Vec::new();
    }
    let Ok(ports) = serialport::available_ports() else {
        return Vec::new();
    };
    ports
        .into_iter()
        .map(|port| SerialDevice {
            path: port.port_name,
            usb: match port.port_type {
                SerialPortType::UsbPort(info) => Some(UsbDetails {
                    vid: info.vid,
                    pid: info.pid,
                    manufacturer: info.manufacturer,
                    product: info.product,
                    serial_number: info.serial_number,
                }),
                _ => None,
            },
        })
        .collect()
}

/// True for a USB serial device whose manufacturer or product name says it is
/// an Enttec DMX USB Pro.
pub fn is_enttec_pro(device: &SerialDevice) -> bool {
    let Some(usb) = &device.usb else {
        return false;
    };
    let mentions = |field: &Option<String>, needle: &str| {
        field
            .as_deref()
            .is_some_and(|value| value.to_ascii_lowercase().contains(needle))
    };
    mentions(&usb.manufacturer, "enttec") || mentions(&usb.product, "dmx usb pro")
}

/// The path of the only Enttec DMX USB Pro among `devices`. Refuses to guess
/// when there is none or more than one.
pub fn choose_enttec_pro(devices: &[SerialDevice]) -> Result<String, String> {
    let found: Vec<&str> = devices
        .iter()
        .filter(|device| is_enttec_pro(device))
        .map(|device| device.path.as_str())
        .collect();
    match found.as_slice() {
        [] => Err("no Enttec DMX USB Pro found".to_string()),
        [one] => Ok((*one).to_string()),
        several => Err(format!(
            "found several Enttec DMX USB Pro interfaces ({}); set output.device to one of them",
            several.join(", ")
        )),
    }
}

pub fn find_enttec_pro() -> Result<String, String> {
    choose_enttec_pro(&serial_devices())
}

/// The /dev/serial/by-id links and the devices they point to.
fn stable_names() -> Vec<(String, String)> {
    let Ok(entries) = std::fs::read_dir("/dev/serial/by-id") else {
        return Vec::new();
    };
    let mut names: Vec<(String, String)> = entries
        .flatten()
        .map(|entry| {
            let link = entry.path();
            let target = std::fs::canonicalize(&link)
                .map(|t| t.display().to_string())
                .unwrap_or_else(|_| "(broken link)".to_string());
            (link.display().to_string(), target)
        })
        .collect();
    names.sort();
    names
}

/// The text printed by `--list-devices`.
pub fn listing() -> String {
    let devices = serial_devices();
    let (usb, other): (Vec<&SerialDevice>, Vec<&SerialDevice>) =
        devices.iter().partition(|device| device.usb.is_some());
    let mut text = String::new();

    if usb.is_empty() {
        text.push_str("No USB serial devices found.\n");
    } else {
        text.push_str("USB serial devices:\n");
        for device in &usb {
            let Some(details) = &device.usb else { continue };
            let _ = write!(
                text,
                "  {}  USB {:04x}:{:04x}  {} {}",
                device.path,
                details.vid,
                details.pid,
                details.manufacturer.as_deref().unwrap_or("(unknown maker)"),
                details.product.as_deref().unwrap_or("(unknown product)"),
            );
            if let Some(serial) = &details.serial_number {
                let _ = write!(text, "  serial {serial}");
            }
            if is_enttec_pro(device) {
                text.push_str("  <- Enttec DMX USB Pro");
            }
            text.push('\n');
        }
    }
    if !other.is_empty() {
        let _ = writeln!(text, "({} other serial ports not shown)", other.len());
    }

    let names = stable_names();
    if !names.is_empty() {
        text.push_str("\nStable names in /dev/serial/by-id:\n");
        for (link, target) in names {
            let _ = writeln!(text, "  {link} -> {target}");
        }
    }

    text.push_str(
        "\nSet output.device (or --device) to a stable name above, or to \"auto\" to pick the one Enttec DMX USB Pro.\n",
    );
    text
}
