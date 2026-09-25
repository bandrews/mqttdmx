// ABOUTME: Property tests that untrusted input never panics the parser, the config loader or the engine.
// ABOUTME: Release builds abort on panic, so any panic here would take the daemon down.

use mqttdmx::command::CommandParser;
use mqttdmx::config::{Config, Driver, GroupConfig, CHANNEL_COUNT};
use mqttdmx::engine::Engine;
use proptest::prelude::*;

fn parser() -> CommandParser {
    let mut config = Config::default();
    config.output.driver = Some(Driver::Null);
    config.groups.insert(
        "House".to_string(),
        GroupConfig {
            channels: vec![1, 2, 512],
            description: None,
        },
    );
    config.groups.insert(
        "Empty".to_string(),
        GroupConfig {
            channels: vec![],
            description: None,
        },
    );
    CommandParser::new(&config)
}

fn topic() -> impl Strategy<Value = String> {
    prop_oneof![
        Just("dmx".to_string()),
        Just("dmx/group/House".to_string()),
        Just("dmx/group/Empty".to_string()),
        "dmx/group/[a-zA-Z0-9/+#-]{0,12}",
        "dmx/channel/[0-9+-]{0,6}",
        ".{0,20}",
    ]
}

/// JSON-ish payloads built from the keys and value shapes the parser looks at.
fn payload() -> impl Strategy<Value = String> {
    let key = prop_oneof![
        Just("value"),
        Just("fade"),
        Just("easing"),
        Just("pulse"),
        Just("twinkle"),
        Just("min_level"),
        Just("max_level"),
        Just("min_duration"),
        Just("max_duration"),
        Just("group"),
        Just("channel"),
        Just("range"),
        Just("start"),
        Just("end"),
        Just("__proto__"),
    ];
    let value = prop_oneof![
        any::<i64>().prop_map(|n| n.to_string()),
        any::<f64>().prop_map(|n| if n.is_finite() {
            n.to_string()
        } else {
            "1e308".to_string()
        }),
        Just("null".to_string()),
        Just("true".to_string()),
        Just("false".to_string()),
        Just("\"sine\"".to_string()),
        Just("\"slow\"".to_string()),
        Just("\"House\"".to_string()),
        Just("{\"start\": 1, \"end\": 512}".to_string()),
        Just("[1, 2]".to_string()),
        "\"[^\"\\\\]{0,8}\"",
    ];
    prop_oneof![
        prop::collection::vec((key, value), 0..6).prop_map(|fields| {
            let body: Vec<String> = fields
                .into_iter()
                .map(|(k, v)| format!("\"{k}\": {v}"))
                .collect();
            format!("{{{}}}", body.join(", "))
        }),
        any::<i64>().prop_map(|n| n.to_string()),
        ".{0,40}",
    ]
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(2000))]

    #[test]
    fn parsing_never_panics_and_accepted_commands_are_in_range(
        topic in topic(),
        payload in payload(),
        raw in prop::collection::vec(any::<u8>(), 0..64),
    ) {
        let parser = parser();
        let _ = parser.parse(&topic, &raw);
        if let Ok(command) = parser.parse(&topic, payload.as_bytes()) {
            prop_assert!(command.channels.iter().all(|&c| c < CHANNEL_COUNT));
            let mut engine = Engine::new(0, 1);
            engine.apply(&command, 0.0);
            let mut frame = [0u8; CHANNEL_COUNT];
            for t in [0.0, 10.0, 1_000.0, 100_000.0, 90_000_000.0] {
                engine.render(t, &mut frame);
            }
        }
    }

    #[test]
    fn config_parsing_and_validation_never_panic(text in ".{0,200}") {
        if let Ok(config) = Config::from_json(&text) {
            let _ = config.validate();
        }
    }

    #[test]
    fn structured_configs_never_panic_validation(
        port in any::<u16>(),
        frame_rate in any::<u32>(),
        min_level in any::<u8>(),
        max_level in any::<u8>(),
        min_duration in any::<f64>(),
        max_duration in any::<f64>(),
        channels in prop::collection::vec(any::<u16>(), 0..8),
        topic in ".{0,12}",
    ) {
        let mut config = Config::default();
        config.mqtt.port = port;
        config.mqtt.topic = topic;
        config.output.driver = Some(Driver::Null);
        config.output.frame_rate = frame_rate;
        config.twinkle.min_level = min_level;
        config.twinkle.max_level = max_level;
        config.twinkle.min_duration = min_duration;
        config.twinkle.max_duration = max_duration;
        config.groups.insert("G".to_string(), GroupConfig { channels, description: None });
        if config.validate().is_ok() {
            // A valid config must also be usable by everything built from it.
            let parser = CommandParser::new(&config);
            let _ = parser.parse(&format!("{}/group/G", config.mqtt.topic), br#"{"twinkle": true}"#);
        }
    }
}
