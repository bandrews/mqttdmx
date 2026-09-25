// ABOUTME: Tests for DMX output: Enttec Pro framing, device selection, and the serial driver.
// ABOUTME: The driver tests run against a fake widget on a pseudo-terminal, including unplug and replug.

mod support;

use std::time::{Duration, Instant};

use mqttdmx::config::CHANNEL_COUNT;
use mqttdmx::output::devices::{choose_enttec_pro, SerialDevice, UsbDetails};
use mqttdmx::output::enttec::{
    dmx_packet, find_message, firmware_version, widget_parameters_request, EnttecPro,
};
use mqttdmx::output::{NullOutput, Output};
use mqttdmx::status::OutputState;
use support::{eventually, DeviceLink, FakeWidget};

const PATIENCE: Duration = Duration::from_secs(10);

fn frame_with(channel: usize, level: u8) -> [u8; CHANNEL_COUNT] {
    let mut frame = [0u8; CHANNEL_COUNT];
    frame[channel - 1] = level;
    frame
}

/// Calls `send` every 25 ms, as the render loop does, until `done` or PATIENCE runs out.
fn send_until(
    output: &mut dyn Output,
    frame: &[u8; CHANNEL_COUNT],
    mut done: impl FnMut(&dyn Output) -> bool,
) -> bool {
    eventually(PATIENCE, || {
        output.send(frame);
        std::thread::sleep(Duration::from_millis(25));
        done(output)
    })
}

#[test]
fn a_dmx_frame_is_wrapped_in_a_label_6_message() {
    let mut frame = [0u8; CHANNEL_COUNT];
    frame[0] = 1;
    frame[511] = 255;
    let packet = dmx_packet(&frame);
    assert_eq!(packet.len(), 518);
    assert_eq!(
        &packet[..5],
        &[0x7E, 6, 0x01, 0x02, 0x00],
        "start, label, length 513, start code"
    );
    assert_eq!(packet[5], 1);
    assert_eq!(packet[516], 255);
    assert_eq!(packet[517], 0xE7);
}

#[test]
fn the_widget_parameters_request_asks_for_no_user_data() {
    assert_eq!(widget_parameters_request(), [0x7E, 3, 2, 0, 0, 0, 0xE7]);
}

#[test]
fn a_reply_is_found_among_other_bytes() {
    let mut buffer = vec![0x00, 0x12];
    buffer.extend_from_slice(&[0x7E, 5, 2, 0, 9, 9, 0xE7]);
    buffer.extend_from_slice(&[0x7E, 3, 5, 0, 44, 1, 9, 1, 40, 0xE7]);
    assert_eq!(find_message(&buffer, 3), Some(vec![44, 1, 9, 1, 40]));
    assert_eq!(find_message(&buffer, 10), None);
    assert_eq!(find_message(&[0x7E, 3, 5, 0, 44, 1], 3), None, "incomplete");
}

#[test]
fn the_firmware_version_is_most_significant_byte_first() {
    assert_eq!(
        firmware_version(&[44, 1, 9, 1, 40]).as_deref(),
        Some("1.44")
    );
    assert_eq!(firmware_version(&[4, 2, 9, 1, 40]).as_deref(), Some("2.4"));
    assert_eq!(firmware_version(&[44]), None);
}

fn device(path: &str, manufacturer: Option<&str>, product: Option<&str>) -> SerialDevice {
    SerialDevice {
        path: path.to_string(),
        usb: Some(UsbDetails {
            vid: 0x0403,
            pid: 0x6001,
            manufacturer: manufacturer.map(str::to_string),
            product: product.map(str::to_string),
            serial_number: Some("EN000001".to_string()),
        }),
    }
}

#[test]
fn auto_picks_the_one_enttec_interface() {
    let devices = [
        device("/dev/ttyUSB0", Some("FTDI"), Some("FT232R USB UART")),
        device("/dev/ttyUSB1", Some("ENTTEC"), Some("DMX USB PRO")),
        SerialDevice {
            path: "/dev/ttyS0".to_string(),
            usb: None,
        },
    ];
    assert_eq!(choose_enttec_pro(&devices), Ok("/dev/ttyUSB1".to_string()));
}

#[test]
fn auto_matches_on_either_name_ignoring_case() {
    assert!(choose_enttec_pro(&[device("/dev/a", Some("Enttec"), None)]).is_ok());
    assert!(choose_enttec_pro(&[device("/dev/a", None, Some("dmx usb pro"))]).is_ok());
}

#[test]
fn auto_refuses_to_guess() {
    assert_eq!(
        choose_enttec_pro(&[device(
            "/dev/ttyUSB0",
            Some("FTDI"),
            Some("FT232R USB UART")
        )]),
        Err("no Enttec DMX USB Pro found".to_string())
    );
    assert_eq!(
        choose_enttec_pro(&[
            device("/dev/ttyUSB0", Some("ENTTEC"), Some("DMX USB PRO")),
            device("/dev/ttyUSB1", Some("ENTTEC"), Some("DMX USB PRO")),
        ]),
        Err(
            "found several Enttec DMX USB Pro interfaces (/dev/ttyUSB0, /dev/ttyUSB1); set output.device to one of them"
                .to_string()
        )
    );
}

#[test]
fn the_null_output_is_disabled() {
    let mut output = NullOutput;
    output.send(&[255; CHANNEL_COUNT]);
    let status = output.status();
    assert_eq!(status.driver, "null");
    assert_eq!(status.state, OutputState::Disabled);
    assert_eq!(status.device, None);
}

#[test]
#[cfg_attr(
    not(target_os = "linux"),
    ignore = "needs Linux pseudo-terminals; run Lane A"
)]
fn it_connects_reads_the_firmware_and_sends_frames() {
    let widget = FakeWidget::start(true);
    let link = DeviceLink::to(&widget.slave_path);
    let mut output = EnttecPro::new(link.path_str());
    let frame = frame_with(1, 255);
    assert!(send_until(&mut output, &frame, |_| widget.frame_count() >= 3));

    let status = output.status();
    assert_eq!(status.driver, "enttec-usb-dmx-pro");
    assert_eq!(status.state, OutputState::Connected);
    assert_eq!(status.device.as_deref(), Some(link.path_str()));
    assert_eq!(status.firmware.as_deref(), Some("1.44"));
    assert_eq!(status.error, None);
    assert_eq!(
        widget.other_labels(),
        vec![3],
        "only the parameters request besides frames"
    );
    let frames = widget.frames();
    assert!(frames.iter().all(|f| f.len() == CHANNEL_COUNT));
    assert_eq!(
        frames[0][0], 255,
        "the very first frame carries the current levels"
    );
}

#[test]
#[cfg_attr(
    not(target_os = "linux"),
    ignore = "needs Linux pseudo-terminals; run Lane A"
)]
fn an_interface_that_does_not_answer_is_still_used() {
    let widget = FakeWidget::start(false);
    let mut output = EnttecPro::new(&widget.slave_path);
    assert!(send_until(&mut output, &frame_with(2, 9), |_| widget
        .frame_count()
        >= 1));
    let status = output.status();
    assert_eq!(status.state, OutputState::Connected);
    assert_eq!(status.firmware, None);
    assert_eq!(widget.frames()[0][1], 9);
}

#[test]
#[cfg_attr(
    not(target_os = "linux"),
    ignore = "needs Linux pseudo-terminals; run Lane A"
)]
fn a_missing_device_is_reported_and_picked_up_when_it_appears() {
    let link = DeviceLink::to("/nonexistent/tty");
    let mut output = EnttecPro::new(link.path_str());
    output.send(&frame_with(1, 1));
    let status = output.status();
    assert_eq!(status.state, OutputState::Disconnected);
    assert_eq!(status.device, None);
    let error = status.error.expect("an error is reported");
    assert!(error.contains(link.path_str()), "{error}");

    let widget = FakeWidget::start(true);
    link.repoint(&widget.slave_path);
    assert!(send_until(&mut output, &frame_with(1, 77), |o| {
        o.status().state == OutputState::Connected
    }));
    assert!(widget.wait_for_frame(0, PATIENCE, |f| f[0] == 77).is_some());
}

#[test]
#[cfg_attr(
    not(target_os = "linux"),
    ignore = "needs Linux pseudo-terminals; run Lane A"
)]
fn it_recovers_after_the_interface_is_unplugged_and_replugged() {
    let widget = FakeWidget::start(true);
    let link = DeviceLink::to(&widget.slave_path);
    let mut output = EnttecPro::new(link.path_str());
    assert!(send_until(&mut output, &frame_with(1, 10), |_| widget
        .frame_count()
        >= 2));

    widget.unplug();
    assert!(send_until(&mut output, &frame_with(1, 20), |o| {
        o.status().state == OutputState::Disconnected
    }));
    let error = output.status().error.expect("the failure is reported");
    assert!(!error.is_empty());

    let replugged = FakeWidget::start(true);
    link.repoint(&replugged.slave_path);
    assert!(send_until(&mut output, &frame_with(1, 30), |o| {
        o.status().state == OutputState::Connected
    }));
    let first = replugged
        .wait_for_frame(0, PATIENCE, |_| true)
        .expect("frames flow again");
    assert_eq!(
        first[0], 30,
        "the first frame after reconnecting carries the current levels"
    );
    assert_eq!(output.status().error, None);
}

#[test]
#[cfg_attr(
    not(target_os = "linux"),
    ignore = "needs Linux pseudo-terminals; run Lane A"
)]
fn a_device_already_in_use_is_reported() {
    let widget = FakeWidget::start(true);
    let mut first = EnttecPro::new(&widget.slave_path);
    assert!(send_until(&mut first, &frame_with(1, 1), |o| {
        o.status().state == OutputState::Connected
    }));
    let mut second = EnttecPro::new(&widget.slave_path);
    second.send(&frame_with(1, 2));
    let status = second.status();
    assert_eq!(status.state, OutputState::Disconnected);
    assert!(status.error.is_some());
}

#[test]
#[cfg_attr(
    not(target_os = "linux"),
    ignore = "needs Linux pseudo-terminals; run Lane A"
)]
fn an_interface_that_stops_taking_data_is_reported_once_and_kept_open() {
    let widget = FakeWidget::start(true);
    let mut output = EnttecPro::new(&widget.slave_path);
    let frame = frame_with(1, 5);
    assert!(send_until(&mut output, &frame, |o| {
        o.status().state == OutputState::Connected
    }));

    widget.stop_reading();
    let mut longest_send = Duration::ZERO;
    let reported = eventually(PATIENCE, || {
        let started = Instant::now();
        output.send(&frame);
        longest_send = longest_send.max(started.elapsed());
        std::thread::sleep(Duration::from_millis(25));
        output.status().state == OutputState::Disconnected
    });
    assert!(reported, "a stalled interface is reported");
    assert!(
        longest_send <= Duration::from_millis(250),
        "a send never waits long on a stalled interface; the longest took {longest_send:?}"
    );
    let status = output.status();
    assert_eq!(
        status.device.as_deref(),
        Some(widget.slave_path.as_str()),
        "the port stays open while the interface is stalled"
    );
    assert_eq!(
        status.error.as_deref(),
        Some("the interface has stopped accepting data")
    );

    widget.resume_reading();
    assert!(send_until(&mut output, &frame, |o| {
        o.status().state == OutputState::Connected
    }));
    assert_eq!(output.status().error, None);
    assert_eq!(
        widget.other_labels(),
        vec![3],
        "the interface was never closed and reopened while stalled"
    );
}
