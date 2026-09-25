// ABOUTME: Tests for the status documents mqttdmx publishes: group levels and health.
// ABOUTME: Checks the exact JSON shapes that consumers read.

use mqttdmx::config::{Config, Driver, GroupConfig, CHANNEL_COUNT};
use mqttdmx::status::{health_json, GroupLevels, OutputState, OutputStatus};

fn config_with_groups(groups: &[(&str, &[u16])]) -> Config {
    let mut config = Config::default();
    config.output.driver = Some(Driver::Null);
    for (name, channels) in groups {
        config.groups.insert(
            name.to_string(),
            GroupConfig {
                channels: channels.to_vec(),
                description: None,
            },
        );
    }
    config
}

#[test]
fn a_group_reports_the_level_its_channels_share() {
    let groups = GroupLevels::new(&config_with_groups(&[("House", &[1, 2]), ("Spot", &[3])]));
    let mut commanded = [0u8; CHANNEL_COUNT];
    commanded[0] = 255;
    commanded[1] = 255;
    commanded[2] = 84;
    assert_eq!(groups.to_json(&commanded), r#"{"House":255,"Spot":84}"#);
}

#[test]
fn a_group_whose_channels_differ_reports_null() {
    let groups = GroupLevels::new(&config_with_groups(&[("House", &[1, 2])]));
    let mut commanded = [0u8; CHANNEL_COUNT];
    commanded[0] = 255;
    assert_eq!(groups.to_json(&commanded), r#"{"House":null}"#);
}

#[test]
fn a_group_without_channels_reports_null() {
    let groups = GroupLevels::new(&config_with_groups(&[("Unused", &[])]));
    assert_eq!(
        groups.to_json(&[255u8; CHANNEL_COUNT]),
        r#"{"Unused":null}"#
    );
}

#[test]
fn overlapping_groups_each_follow_their_own_channels() {
    let groups = GroupLevels::new(&config_with_groups(&[
        ("All", &[1, 2, 3]),
        ("AllButStage", &[1, 2]),
        ("Stage", &[3]),
    ]));
    let mut commanded = [0u8; CHANNEL_COUNT];
    commanded[2] = 255;
    assert_eq!(
        groups.to_json(&commanded),
        r#"{"All":null,"AllButStage":0,"Stage":255}"#
    );
}

#[test]
fn no_groups_gives_an_empty_object() {
    let groups = GroupLevels::new(&config_with_groups(&[]));
    assert_eq!(groups.to_json(&[0u8; CHANNEL_COUNT]), "{}");
}

#[test]
fn health_reports_the_version_and_output() {
    let status = OutputStatus {
        driver: "enttec-usb-dmx-pro",
        device: Some("/dev/ttyUSB0".to_string()),
        state: OutputState::Connected,
        firmware: Some("1.44".to_string()),
        error: None,
    };
    let json: serde_json::Value = serde_json::from_str(&health_json(&status)).expect("valid JSON");
    assert_eq!(
        json,
        serde_json::json!({
            "version": env!("CARGO_PKG_VERSION"),
            "output": {
                "driver": "enttec-usb-dmx-pro",
                "device": "/dev/ttyUSB0",
                "state": "connected",
                "firmware": "1.44",
                "error": null
            }
        })
    );
}

#[test]
fn output_states_serialise_in_lowercase() {
    for (state, name) in [
        (OutputState::Connected, "connected"),
        (OutputState::Disconnected, "disconnected"),
        (OutputState::Disabled, "disabled"),
    ] {
        let status = OutputStatus {
            driver: "null",
            device: None,
            state,
            firmware: None,
            error: Some("why".to_string()),
        };
        let json: serde_json::Value =
            serde_json::from_str(&health_json(&status)).expect("valid JSON");
        assert_eq!(json["output"]["state"], name);
        assert_eq!(json["output"]["error"], "why");
    }
}
