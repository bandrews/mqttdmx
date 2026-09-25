// ABOUTME: Tests for loading, overriding and validating the mqttdmx configuration.
// ABOUTME: Covers defaults, the shipped example and every validation rule.

use mqttdmx::config::{Config, Driver, Easing, LogFormat, LogLevel, Overrides};

fn valid_null_config() -> Config {
    let mut config = Config::default();
    config.output.driver = Some(Driver::Null);
    config
}

fn validation_errors(config: &Config) -> Vec<String> {
    config.validate().err().unwrap_or_default()
}

#[test]
fn defaults_match_the_documentation() {
    let config = Config::default();
    assert_eq!(config.mqtt.server, "localhost");
    assert_eq!(config.mqtt.port, 1883);
    assert_eq!(config.mqtt.topic, "dmx");
    assert_eq!(config.mqtt.client_id, None);
    assert_eq!(config.output.driver, None);
    assert_eq!(config.output.device, None);
    assert_eq!(config.output.frame_rate, 40);
    assert_eq!(config.output.startup_level, 0);
    assert_eq!(config.fade.easing, Easing::Linear);
    assert_eq!(config.twinkle.min_level, 100);
    assert_eq!(config.twinkle.max_level, 255);
    assert_eq!(config.twinkle.min_duration, 500.0);
    assert_eq!(config.twinkle.max_duration, 2000.0);
    assert_eq!(config.twinkle.easing, Easing::Sine);
    assert!(config.groups.is_empty());
    assert_eq!(config.logging.level, LogLevel::Info);
    assert_eq!(config.logging.format, LogFormat::Text);
}

#[test]
fn the_driver_is_required() {
    let errors = validation_errors(&Config::default());
    assert_eq!(
        errors,
        vec!["output.driver is required: enttec-usb-dmx-pro or null".to_string()]
    );
}

#[test]
fn a_null_driver_config_with_defaults_is_valid() {
    assert_eq!(valid_null_config().validate(), Ok(()));
}

#[test]
fn the_enttec_driver_requires_a_device() {
    let mut config = Config::default();
    config.output.driver = Some(Driver::EnttecUsbDmxPro);
    assert_eq!(
        validation_errors(&config),
        vec![
            "output.device is required for the enttec-usb-dmx-pro driver: a serial device path or \"auto\""
                .to_string()
        ]
    );
    config.output.device = Some("auto".to_string());
    assert_eq!(config.validate(), Ok(()));
}

#[test]
fn the_example_config_loads_and_validates() {
    let text = std::fs::read_to_string(concat!(env!("CARGO_MANIFEST_DIR"), "/config.example.json"))
        .expect("config.example.json is readable");
    let config = Config::from_json(&text).expect("config.example.json parses");
    assert_eq!(config.validate(), Ok(()));
    assert_eq!(config.output.driver, Some(Driver::EnttecUsbDmxPro));
    assert_eq!(config.output.device.as_deref(), Some("auto"));
    assert!(!config.groups.is_empty());
}

#[test]
fn unknown_top_level_keys_are_ignored() {
    let config = Config::from_json(r#"{"ui": {"levels": {"off": 0}}, "_comment": "shared file"}"#)
        .expect("unknown top-level keys are allowed");
    assert_eq!(config.mqtt.topic, "dmx");
}

#[test]
fn unknown_keys_inside_a_section_are_rejected() {
    for text in [
        r#"{"output": {"startup_levle": 255}}"#,
        r#"{"mqtt": {"host": "broker"}}"#,
        r#"{"twinkle": {"minBrightness": 100}}"#,
        r#"{"groups": {"House": {"chanels": [1]}}}"#,
        r#"{"fade": {"curve": "sine"}}"#,
        r#"{"logging": {"colour": true}}"#,
    ] {
        let error = Config::from_json(text).expect_err(text).to_string();
        assert!(error.contains("unknown field"), "{text}: {error}");
    }
}

#[test]
fn wrongly_typed_values_are_rejected_when_parsing() {
    for text in [
        r#"{"output": {"startup_level": 256}}"#,
        r#"{"output": {"driver": "enttec-open-usb-dmx"}}"#,
        r#"{"mqtt": {"port": "1883"}}"#,
        r#"{"fade": {"easing": "ease-in-out"}}"#,
        r#"{"groups": {"House": {"channels": [1.5]}}}"#,
        r#"{"logging": {"level": "verbose"}}"#,
        r#"not json"#,
    ] {
        assert!(Config::from_json(text).is_err(), "{text} should not parse");
    }
}

#[test]
fn every_validation_problem_is_reported_at_once() {
    let text = r#"{
        "mqtt": {"server": "", "port": 0, "topic": "dmx/#", "client_id": "", "username": "only-user"},
        "output": {"driver": "null", "frame_rate": 45},
        "twinkle": {"min_level": 200, "max_level": 100, "min_duration": 100, "max_duration": 700000},
        "groups": {
            "": {"channels": [1]},
            "Bad/Name": {"channels": [1]},
            "Wild+": {"channels": [1]},
            "Hash#": {"channels": [1]},
            "Range": {"channels": [0, 513, 12]},
            "Dupes": {"channels": [5, 6, 5]}
        }
    }"#;
    let config = Config::from_json(text).expect("structurally valid");
    let errors = validation_errors(&config);
    let expected = [
        "mqtt.server must not be empty",
        "mqtt.port must not be 0",
        "mqtt.topic must not contain the wildcards + or # (got \"dmx/#\")",
        "mqtt.client_id must not be empty when set",
        "mqtt.username and mqtt.password must be set together",
        "output.frame_rate must be between 1 and 44 (got 45)",
        "twinkle.min_level (200) must not be greater than twinkle.max_level (100)",
        "twinkle.min_duration must be between 200 and 600000 ms (got 100)",
        "twinkle.max_duration must be between 200 and 600000 ms (got 700000)",
        "groups: a group name must not be empty",
        "groups.Bad/Name: a group name must not contain /, + or #",
        "groups.Wild+: a group name must not contain /, + or #",
        "groups.Hash#: a group name must not contain /, + or #",
        "groups.Range: channel 0 is outside 1-512",
        "groups.Range: channel 513 is outside 1-512",
        "groups.Dupes: channel 5 is listed more than once",
    ];
    for message in expected {
        assert!(
            errors.iter().any(|e| e == message),
            "missing error {message:?} in {errors:#?}"
        );
    }
    assert_eq!(
        errors.len(),
        expected.len(),
        "unexpected extra errors: {errors:#?}"
    );
}

#[test]
fn topic_shapes_are_checked() {
    for (topic, message) in [
        ("", "mqtt.topic must not be empty"),
        (
            "/dmx",
            "mqtt.topic must not start or end with / (got \"/dmx\")",
        ),
        (
            "dmx/",
            "mqtt.topic must not start or end with / (got \"dmx/\")",
        ),
        (
            "a//b",
            "mqtt.topic must not contain an empty level (got \"a//b\")",
        ),
        (
            "dmx/+",
            "mqtt.topic must not contain the wildcards + or # (got \"dmx/+\")",
        ),
    ] {
        let mut config = valid_null_config();
        config.mqtt.topic = topic.to_string();
        assert_eq!(
            validation_errors(&config),
            vec![message.to_string()],
            "topic {topic:?}"
        );
    }
    let mut config = valid_null_config();
    config.mqtt.topic = "venue/lights/dmx".to_string();
    assert_eq!(config.validate(), Ok(()));
}

#[test]
fn twinkle_durations_must_be_ordered() {
    let mut config = valid_null_config();
    config.twinkle.min_duration = 3000.0;
    config.twinkle.max_duration = 1000.0;
    assert_eq!(
        validation_errors(&config),
        vec![
            "twinkle.min_duration (3000) must not be greater than twinkle.max_duration (1000)"
                .to_string()
        ]
    );
}

#[test]
fn command_line_overrides_win_over_the_file() {
    let mut config = Config::from_json(
        r#"{"mqtt": {"server": "file-host", "port": 1000, "topic": "file"},
            "output": {"driver": "null", "startup_level": 10},
            "logging": {"level": "warn", "format": "text"}}"#,
    )
    .expect("parses");
    config.apply_overrides(&Overrides {
        server: Some("cli-host".to_string()),
        port: Some(2000),
        topic: Some("cli".to_string()),
        username: Some("user".to_string()),
        password: Some("secret".to_string()),
        driver: Some(Driver::EnttecUsbDmxPro),
        device: Some("/dev/ttyUSB3".to_string()),
        startup_level: Some(255),
        log_format: Some(LogFormat::Json),
        verbose: true,
    });
    assert_eq!(config.mqtt.server, "cli-host");
    assert_eq!(config.mqtt.port, 2000);
    assert_eq!(config.mqtt.topic, "cli");
    assert_eq!(config.mqtt.username.as_deref(), Some("user"));
    assert_eq!(config.mqtt.password.as_deref(), Some("secret"));
    assert_eq!(config.output.driver, Some(Driver::EnttecUsbDmxPro));
    assert_eq!(config.output.device.as_deref(), Some("/dev/ttyUSB3"));
    assert_eq!(config.output.startup_level, 255);
    assert_eq!(config.logging.format, LogFormat::Json);
    assert_eq!(config.logging.level, LogLevel::Debug);
}

#[test]
fn absent_overrides_leave_the_file_values_alone() {
    let mut config =
        Config::from_json(r#"{"mqtt": {"server": "file-host"}, "output": {"startup_level": 10}}"#)
            .expect("parses");
    config.apply_overrides(&Overrides::default());
    assert_eq!(config.mqtt.server, "file-host");
    assert_eq!(config.output.startup_level, 10);
    assert_eq!(config.logging.level, LogLevel::Info);
}

#[test]
fn loading_a_missing_file_names_the_path() {
    let error = mqttdmx::config::load(std::path::Path::new("/nonexistent/mqttdmx.json"))
        .expect_err("missing file");
    assert!(
        error.to_string().contains("/nonexistent/mqttdmx.json"),
        "{error}"
    );
}

#[test]
fn loading_a_file_reads_its_contents() {
    let dir = tempfile::tempdir().expect("tempdir");
    let path = dir.path().join("mqttdmx.json");
    std::fs::write(
        &path,
        r#"{"output": {"driver": "null", "startup_level": 42}}"#,
    )
    .expect("write");
    let config = mqttdmx::config::load(&path).expect("loads");
    assert_eq!(config.output.startup_level, 42);
}
