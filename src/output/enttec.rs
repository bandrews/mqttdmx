// ABOUTME: Enttec DMX USB Pro driver: message framing, the widget parameters query, and reconnects.
// ABOUTME: Sends one label 6 frame per call; a stalled interface is waited out, a failed one reopened.

use std::io::{Read, Write};
use std::time::{Duration, Instant};

use serialport::{ClearBuffer, SerialPort, TTYPort};
use tracing::{info, warn};

use crate::config::CHANNEL_COUNT;
use crate::output::devices::find_enttec_pro;
use crate::output::Output;
use crate::status::{OutputState, OutputStatus};

const START_OF_MESSAGE: u8 = 0x7E;
const END_OF_MESSAGE: u8 = 0xE7;
/// "Get Widget Parameters": answered with the firmware version; does not interrupt output.
const LABEL_WIDGET_PARAMETERS: u8 = 3;
/// "Output Only Send DMX Packet": the widget repeats the frame until the next one.
const LABEL_SEND_DMX: u8 = 6;
const DMX_START_CODE: u8 = 0x00;

const DRIVER_NAME: &str = "enttec-usb-dmx-pro";
/// The widget is a USB FIFO and ignores the baud rate.
const BAUD_RATE: u32 = 57_600;
/// A write that takes longer than this means the interface has stopped listening.
const WRITE_TIMEOUT: Duration = Duration::from_millis(100);
const REPLY_TIMEOUT: Duration = Duration::from_millis(500);
const FIRST_RETRY: Duration = Duration::from_millis(250);
const LONGEST_RETRY: Duration = Duration::from_secs(5);
/// A connection that lasts this long resets the retry delay after it fails.
const STABLE_CONNECTION: Duration = Duration::from_secs(5);
/// How long frames may go undelivered before the interface is reported as stalled.
const STALL_LIMIT: Duration = Duration::from_secs(1);
const STALLED: &str = "the interface has stopped accepting data";

/// What became of one frame.
enum Delivery {
    Sent,
    /// The interface is still holding an earlier frame, or the write timed out.
    Waiting,
    Failed(String),
}

/// The message that sends one full DMX frame.
pub fn dmx_packet(frame: &[u8; CHANNEL_COUNT]) -> Vec<u8> {
    let length = CHANNEL_COUNT + 1;
    let mut packet = Vec::with_capacity(length + 5);
    packet.extend_from_slice(&[
        START_OF_MESSAGE,
        LABEL_SEND_DMX,
        (length & 0xFF) as u8,
        (length >> 8) as u8,
        DMX_START_CODE,
    ]);
    packet.extend_from_slice(frame);
    packet.push(END_OF_MESSAGE);
    packet
}

/// The message that asks the widget for its parameters, with no user data.
pub fn widget_parameters_request() -> [u8; 7] {
    [
        START_OF_MESSAGE,
        LABEL_WIDGET_PARAMETERS,
        2,
        0,
        0,
        0,
        END_OF_MESSAGE,
    ]
}

/// The data of the first complete message with `label` in `buffer`.
pub fn find_message(buffer: &[u8], label: u8) -> Option<Vec<u8>> {
    let mut index = 0;
    while let Some(offset) = buffer[index..].iter().position(|&b| b == START_OF_MESSAGE) {
        let start = index + offset;
        let header = buffer.get(start..start + 4)?;
        let length = usize::from(header[2]) | (usize::from(header[3]) << 8);
        let end = start + 4 + length;
        match buffer.get(end) {
            None => return None,
            Some(&END_OF_MESSAGE) if header[1] == label => {
                return Some(buffer[start + 4..end].to_vec());
            }
            Some(&END_OF_MESSAGE) => index = end + 1,
            Some(_) => index = start + 1,
        }
    }
    None
}

/// The firmware version from a widget parameters reply: "<MSB>.<LSB>".
pub fn firmware_version(data: &[u8]) -> Option<String> {
    match data {
        [lsb, msb, ..] => Some(format!("{msb}.{lsb}")),
        _ => None,
    }
}

enum DeviceSelector {
    Path(String),
    Auto,
}

/// Drives an Enttec DMX USB Pro, opening it on demand and reopening it after
/// a failure, without ever blocking for long. An interface that stops taking
/// data is kept open and waited out: closing a USB serial port with data
/// still queued can block for up to 30 seconds.
pub struct EnttecPro {
    selector: DeviceSelector,
    port: Option<TTYPort>,
    status: OutputStatus,
    retry_delay: Duration,
    next_attempt: Instant,
    connected_at: Option<Instant>,
    waiting_since: Option<Instant>,
}

impl EnttecPro {
    /// `device` is a serial device path, or `auto` to find the interface.
    pub fn new(device: &str) -> Self {
        let selector = if device == "auto" {
            DeviceSelector::Auto
        } else {
            DeviceSelector::Path(device.to_string())
        };
        EnttecPro {
            selector,
            port: None,
            status: disconnected(None),
            retry_delay: FIRST_RETRY,
            next_attempt: Instant::now(),
            connected_at: None,
            waiting_since: None,
        }
    }

    fn connect(&mut self) -> Result<(), String> {
        let path = match &self.selector {
            DeviceSelector::Path(path) => path.clone(),
            DeviceSelector::Auto => find_enttec_pro()?,
        };
        let mut port = serialport::new(&path, BAUD_RATE)
            .timeout(WRITE_TIMEOUT)
            .open_native()
            .map_err(|e| format!("cannot open {path}: {e}"))?;
        let _ = port.clear(ClearBuffer::All);
        let firmware = query_firmware(&mut port);
        match &firmware {
            Some(version) => info!("DMX interface connected on {path}, firmware {version}"),
            None => warn!(
                "DMX interface connected on {path}, but it did not answer the widget parameters \
                 request; sending frames anyway. Check that it is an Enttec DMX USB Pro"
            ),
        }
        self.port = Some(port);
        self.connected_at = Some(Instant::now());
        self.status = OutputStatus {
            driver: DRIVER_NAME,
            device: Some(path),
            state: OutputState::Connected,
            firmware,
            error: None,
        };
        Ok(())
    }

    fn lose_connection(&mut self, error: String) {
        let device = self.status.device.clone().unwrap_or_default();
        warn!("DMX interface on {device} failed: {error}; reconnecting");
        if self
            .connected_at
            .take()
            .is_some_and(|since| since.elapsed() >= STABLE_CONNECTION)
        {
            self.retry_delay = FIRST_RETRY;
        }
        self.port = None;
        self.waiting_since = None;
        self.status = disconnected(Some(error));
        self.schedule_retry();
    }

    fn frame_sent(&mut self) {
        self.waiting_since = None;
        if self.status.state != OutputState::Connected {
            let device = self.status.device.clone().unwrap_or_default();
            info!("DMX interface on {device} is accepting data again");
            self.status.state = OutputState::Connected;
            self.status.error = None;
        }
    }

    fn frame_waiting(&mut self) {
        let since = *self.waiting_since.get_or_insert_with(Instant::now);
        if self.status.state == OutputState::Connected && since.elapsed() >= STALL_LIMIT {
            let device = self.status.device.clone().unwrap_or_default();
            warn!("DMX interface on {device} has stopped accepting data; skipping frames until it does");
            self.status.state = OutputState::Disconnected;
            self.status.error = Some(STALLED.to_string());
        }
    }

    fn schedule_retry(&mut self) {
        self.next_attempt = Instant::now() + self.retry_delay;
        self.retry_delay = (self.retry_delay * 2).min(LONGEST_RETRY);
    }
}

impl Output for EnttecPro {
    fn send(&mut self, frame: &[u8; CHANNEL_COUNT]) {
        if self.port.is_none() {
            if Instant::now() < self.next_attempt {
                return;
            }
            if let Err(error) = self.connect() {
                if self.status.error.as_deref() != Some(error.as_str()) {
                    warn!("DMX interface unavailable: {error}; retrying");
                }
                self.status = disconnected(Some(error));
                self.schedule_retry();
                return;
            }
        }
        let delivery = match self.port.as_mut() {
            Some(port) => deliver(port, &dmx_packet(frame)),
            None => return,
        };
        match delivery {
            Delivery::Sent => self.frame_sent(),
            Delivery::Waiting => self.frame_waiting(),
            Delivery::Failed(error) => self.lose_connection(error),
        }
    }

    fn status(&self) -> OutputStatus {
        self.status.clone()
    }
}

fn disconnected(error: Option<String>) -> OutputStatus {
    OutputStatus {
        driver: DRIVER_NAME,
        device: None,
        state: OutputState::Disconnected,
        firmware: None,
        error,
    }
}

/// Writes `packet` unless an earlier frame is still queued for the interface,
/// so that a stalled interface costs at most one write timeout.
fn deliver(port: &mut TTYPort, packet: &[u8]) -> Delivery {
    match port.bytes_to_write() {
        Ok(0) => {}
        Ok(_) => return Delivery::Waiting,
        Err(error) => return Delivery::Failed(error.to_string()),
    }
    match port.write_all(packet) {
        Ok(()) => Delivery::Sent,
        Err(error) if error.kind() == std::io::ErrorKind::TimedOut => Delivery::Waiting,
        Err(error) => Delivery::Failed(error.to_string()),
    }
}

/// Asks the widget for its parameters and returns its firmware version, or
/// `None` if no answer arrives in time.
fn query_firmware(port: &mut TTYPort) -> Option<String> {
    port.write_all(&widget_parameters_request()).ok()?;
    let deadline = Instant::now() + REPLY_TIMEOUT;
    let mut buffer = Vec::new();
    let mut chunk = [0u8; 64];
    while Instant::now() < deadline {
        match port.read(&mut chunk) {
            Ok(count) => {
                buffer.extend_from_slice(&chunk[..count]);
                if let Some(data) = find_message(&buffer, LABEL_WIDGET_PARAMETERS) {
                    return firmware_version(&data);
                }
            }
            Err(error) if error.kind() == std::io::ErrorKind::TimedOut => {}
            Err(_) => return None,
        }
    }
    None
}
