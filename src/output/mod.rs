// ABOUTME: DMX output drivers behind one interface: the Enttec DMX USB Pro and a null driver.
// ABOUTME: The render loop hands every frame to `send` and reports `status` to MQTT.

pub mod devices;
pub mod enttec;

use crate::config::{Driver, OutputConfig, CHANNEL_COUNT};
use crate::status::{OutputState, OutputStatus};

pub trait Output: Send {
    /// Sends one frame. Called once per render tick; must not block for long.
    fn send(&mut self, frame: &[u8; CHANNEL_COUNT]);
    fn status(&self) -> OutputStatus;
}

/// Sends nothing, for testing without hardware.
pub struct NullOutput;

impl Output for NullOutput {
    fn send(&mut self, _frame: &[u8; CHANNEL_COUNT]) {}

    fn status(&self) -> OutputStatus {
        OutputStatus {
            driver: Driver::Null.name(),
            device: None,
            state: OutputState::Disabled,
            firmware: None,
            error: None,
        }
    }
}

/// The output driver a validated configuration asks for.
pub fn open(config: &OutputConfig) -> Result<Box<dyn Output>, String> {
    match (config.driver, config.device.as_deref()) {
        (Some(Driver::Null), _) => Ok(Box::new(NullOutput)),
        (Some(Driver::EnttecUsbDmxPro), Some(device)) => {
            Ok(Box::new(enttec::EnttecPro::new(device)))
        }
        (Some(Driver::EnttecUsbDmxPro), None) => {
            Err("output.device is required for the enttec-usb-dmx-pro driver".to_string())
        }
        (None, _) => Err("output.driver is required".to_string()),
    }
}
