// ABOUTME: Tests for turning MQTT topics and payloads into validated lighting commands.
// ABOUTME: Covers every payload shape and the rejection of malformed messages.

use mqttdmx::command::{Action, CommandParser, PulseSpeed, Target, TwinkleParams};
use mqttdmx::config::{Config, Driver, Easing, GroupConfig};

fn parser() -> CommandParser {
    let mut config = Config::default();
    config.output.driver = Some(Driver::Null);
    config.groups.insert(
        "House".to_string(),
        GroupConfig {
            channels: vec![95, 71, 278],
            description: None,
        },
    );
    config.groups.insert(
        "HouseLeft".to_string(),
        GroupConfig {
            channels: vec![95],
            description: None,
        },
    );
    config.groups.insert(
        "Unused".to_string(),
        GroupConfig {
            channels: vec![],
            description: None,
        },
    );
    CommandParser::new(&config)
}

fn parse(topic: &str, payload: &str) -> mqttdmx::command::Command {
    parser()
        .parse(topic, payload.as_bytes())
        .unwrap_or_else(|e| panic!("{topic} {payload:?} should parse: {e}"))
}

fn reject(topic: &str, payload: &str) -> String {
    match parser().parse(topic, payload.as_bytes()) {
        Ok(command) => panic!("{topic} {payload:?} should be rejected, got {command:?}"),
        Err(error) => error.to_string(),
    }
}

fn set(level: u8, fade_ms: f64, easing: Easing) -> Action {
    Action::Set {
        level,
        fade_ms,
        easing,
    }
}

#[test]
fn subscriptions_cover_exactly_the_command_topics() {
    assert_eq!(
        parser().subscriptions(),
        vec![
            "dmx".to_string(),
            "dmx/channel/+".to_string(),
            "dmx/group/+".to_string()
        ]
    );
}

#[test]
fn group_value_sets_every_channel_of_the_group() {
    let command = parse("dmx/group/House", r#"{"value":255}"#);
    assert_eq!(command.target, Target::Group("House".to_string()));
    assert_eq!(
        command.channels,
        vec![94, 70, 277],
        "0-based channel indices"
    );
    assert_eq!(command.action, set(255, 0.0, Easing::Linear));
}

#[test]
fn group_value_with_fade_uses_the_configured_easing() {
    let command = parse("dmx/group/House", r#"{"value":0,"fade":10000}"#);
    assert_eq!(command.action, set(0, 10000.0, Easing::Linear));
}

#[test]
fn a_fade_may_name_its_easing() {
    let command = parse(
        "dmx/group/House",
        r#"{"value":84,"fade":1000,"easing":"sine"}"#,
    );
    assert_eq!(command.action, set(84, 1000.0, Easing::Sine));
}

#[test]
fn zero_fade_is_an_instant_set() {
    let command = parse("dmx/group/House", r#"{"value":168,"fade":0}"#);
    assert_eq!(command.action, set(168, 0.0, Easing::Linear));
}

#[test]
fn the_config_fade_easing_is_the_default() {
    let mut config = Config::default();
    config.output.driver = Some(Driver::Null);
    config.fade.easing = Easing::Sine;
    config.groups.insert(
        "G".to_string(),
        GroupConfig {
            channels: vec![1],
            description: None,
        },
    );
    let command = CommandParser::new(&config)
        .parse("dmx/group/G", br#"{"value":1,"fade":10}"#)
        .expect("parses");
    assert_eq!(command.action, set(1, 10.0, Easing::Sine));
}

#[test]
fn a_bare_number_sets_a_topic_addressed_target() {
    assert_eq!(
        parse("dmx/group/House", "255").action,
        set(255, 0.0, Easing::Linear)
    );
    assert_eq!(
        parse("dmx/group/House", " 0\n").action,
        set(0, 0.0, Easing::Linear)
    );
    let command = parse("dmx/channel/95", "128");
    assert_eq!(command.target, Target::Channel(95));
    assert_eq!(command.channels, vec![94]);
    assert_eq!(command.action, set(128, 0.0, Easing::Linear));
}

#[test]
fn a_whole_number_written_with_a_decimal_point_is_accepted() {
    assert_eq!(
        parse("dmx/channel/1", "255.0").action,
        set(255, 0.0, Easing::Linear)
    );
}

#[test]
fn channel_topics_accept_the_full_action_set() {
    assert_eq!(
        parse("dmx/channel/512", r#"{"value":10,"fade":500}"#).action,
        set(10, 500.0, Easing::Linear)
    );
    assert_eq!(
        parse("dmx/channel/1", r#"{"pulse":"fast"}"#).action,
        Action::Pulse {
            speed: PulseSpeed::Fast,
            easing: Easing::Linear
        }
    );
}

#[test]
fn pulse_speeds_are_parsed() {
    assert_eq!(
        parse("dmx/group/HouseLeft", r#"{"pulse":"slow"}"#).action,
        Action::Pulse {
            speed: PulseSpeed::Slow,
            easing: Easing::Linear
        }
    );
    assert_eq!(PulseSpeed::Slow.timings_ms(), (1500.0, 2000.0, 1500.0));
    assert_eq!(PulseSpeed::Fast.timings_ms(), (500.0, 500.0, 500.0));
}

#[test]
fn twinkle_uses_config_defaults_for_missing_fields() {
    let command = parse("dmx/group/House", r#"{"twinkle":true}"#);
    assert_eq!(
        command.action,
        Action::TwinkleStart(TwinkleParams {
            min_level: 100,
            max_level: 255,
            min_duration_ms: 500.0,
            max_duration_ms: 2000.0,
            easing: Easing::Sine,
        })
    );
}

#[test]
fn twinkle_fields_override_the_defaults() {
    let command = parse(
        "dmx/group/House",
        r#"{"twinkle":true,"min_level":150,"max_level":200,"min_duration":800,"max_duration":2500,"easing":"linear"}"#,
    );
    assert_eq!(
        command.action,
        Action::TwinkleStart(TwinkleParams {
            min_level: 150,
            max_level: 200,
            min_duration_ms: 800.0,
            max_duration_ms: 2500.0,
            easing: Easing::Linear,
        })
    );
}

#[test]
fn twinkle_stop_may_fade() {
    assert_eq!(
        parse("dmx/group/House", r#"{"twinkle":false}"#).action,
        Action::TwinkleStop {
            fade_ms: 0.0,
            easing: Easing::Linear
        }
    );
    assert_eq!(
        parse("dmx/group/House", r#"{"twinkle":false,"fade":1000}"#).action,
        Action::TwinkleStop {
            fade_ms: 1000.0,
            easing: Easing::Linear
        }
    );
}

#[test]
fn twinkle_stop_may_name_the_easing_of_its_return_fade() {
    assert_eq!(
        parse(
            "dmx/group/House",
            r#"{"twinkle":false,"fade":1000,"easing":"sine"}"#
        )
        .action,
        Action::TwinkleStop {
            fade_ms: 1000.0,
            easing: Easing::Sine
        }
    );
}

#[test]
fn empty_groups_are_valid_targets() {
    let command = parse("dmx/group/Unused", r#"{"value":255}"#);
    assert_eq!(command.target, Target::Group("Unused".to_string()));
    assert!(command.channels.is_empty());
}

#[test]
fn base_topic_range_and_channel_targets_parse() {
    let all_off = parse("dmx", r#"{"range":{"start":1,"end":512},"value":0}"#);
    assert_eq!(all_off.target, Target::Range { start: 1, end: 512 });
    assert_eq!(all_off.channels, (0..512).collect::<Vec<usize>>());
    assert_eq!(all_off.action, set(0, 0.0, Easing::Linear));

    let discovery = parse("dmx", r#"{"range":{"start":1,"end":256},"value":255}"#);
    assert_eq!(discovery.channels.len(), 256);

    let found = parse("dmx", r#"{"channel":133,"value":255}"#);
    assert_eq!(found.target, Target::Channel(133));
    assert_eq!(found.channels, vec![132]);
}

#[test]
fn base_topic_accepts_a_group_target() {
    let command = parse("dmx", r#"{"group":"House","value":0,"fade":2000}"#);
    assert_eq!(command.target, Target::Group("House".to_string()));
    assert_eq!(command.action, set(0, 2000.0, Easing::Linear));
}

#[test]
fn commands_describe_themselves_for_the_log() {
    assert_eq!(
        parse("dmx/group/House", "255").to_string(),
        "group House: set to 255"
    );
    assert_eq!(
        parse("dmx/group/House", r#"{"value":0,"fade":3000}"#).to_string(),
        "group House: fade to 0 over 3000 ms (linear)"
    );
    assert_eq!(
        parse("dmx/channel/5", r#"{"pulse":"fast"}"#).to_string(),
        "channel 5: pulse (fast)"
    );
    assert_eq!(
        parse("dmx", r#"{"range":{"start":1,"end":512},"twinkle":true}"#).to_string(),
        "channels 1-512: twinkle between 100 and 255 every 500-2000 ms (sine)"
    );
    assert_eq!(
        parse("dmx/group/House", r#"{"twinkle":false,"fade":1000}"#).to_string(),
        "group House: stop twinkle, return over 1000 ms (linear)"
    );
    assert_eq!(
        parse("dmx/group/House", r#"{"twinkle":false}"#).to_string(),
        "group House: stop twinkle"
    );
}

#[test]
fn malformed_payloads_are_rejected_with_a_reason() {
    let cases = [
        ("dmx/group/House", "", "empty payload"),
        ("dmx/group/House", "   ", "empty payload"),
        ("dmx/group/House", "null", "payload must be a level from 0 to 255 or a JSON object (got null)"),
        ("dmx/group/House", "\"255\"", "payload must be a level from 0 to 255 or a JSON object (got \"255\")"),
        ("dmx/group/House", "[255]", "payload must be a level from 0 to 255 or a JSON object (got [255])"),
        ("dmx/group/House", "on", "payload is neither a number nor valid JSON"),
        ("dmx/group/House", "{\"value\":", "payload is neither a number nor valid JSON"),
        ("dmx/group/House", "300", "a level must be a whole number from 0 to 255 (got 300)"),
        ("dmx/group/House", "-1", "a level must be a whole number from 0 to 255 (got -1)"),
        ("dmx/group/House", "127.5", "a level must be a whole number from 0 to 255 (got 127.5)"),
        ("dmx/group/House", r#"{"value":300}"#, "value must be a whole number from 0 to 255 (got 300)"),
        ("dmx/group/House", r#"{"value":"255"}"#, "value must be a whole number from 0 to 255 (got \"255\")"),
        ("dmx/group/House", r#"{"value":null}"#, "value must be a whole number from 0 to 255 (got null)"),
        ("dmx/group/House", r#"{"value":1,"fade":-1}"#, "fade must be a number of ms from 0 to 86400000 (got -1)"),
        ("dmx/group/House", r#"{"value":1,"fade":"1000"}"#, "fade must be a number of ms from 0 to 86400000 (got \"1000\")"),
        ("dmx/group/House", r#"{"value":1,"fade":1e21}"#, "fade must be a number of ms from 0 to 86400000 (got 1e+21)"),
        ("dmx/group/House", r#"{"value":1,"fdae":1000}"#, "unknown field \"fdae\""),
        ("dmx/group/House", r#"{"value":1,"b":1,"a":2}"#, "unknown fields \"a\", \"b\""),
        ("dmx/group/House", r#"{"value":1,"easing":"ease-in-out"}"#, "easing must be \"linear\" or \"sine\" (got \"ease-in-out\")"),
        ("dmx/group/House", r#"{}"#, "no action: expected \"value\", \"pulse\" or \"twinkle\""),
        ("dmx/group/House", r#"{"fade":1000}"#, "no action: expected \"value\", \"pulse\" or \"twinkle\""),
        ("dmx/group/House", r#"{"value":1,"pulse":"slow"}"#, "only one action per message (got \"value\" and \"pulse\")"),
        ("dmx/group/House", r#"{"value":1,"twinkle":true}"#, "only one action per message (got \"value\" and \"twinkle\")"),
        ("dmx/group/House", r#"{"pulse":"medium"}"#, "pulse must be \"slow\" or \"fast\" (got \"medium\")"),
        ("dmx/group/House", r#"{"pulse":"slow","fade":100}"#, "\"fade\" does not apply to pulse"),
        ("dmx/group/House", r#"{"pulse":"slow","easing":"sine"}"#, "\"easing\" does not apply to pulse"),
        ("dmx/group/House", r#"{"twinkle":"yes"}"#, "twinkle must be true or false (got \"yes\")"),
        ("dmx/group/House", r#"{"twinkle":true,"fade":100}"#, "\"fade\" does not apply to starting a twinkle"),
        ("dmx/group/House", r#"{"twinkle":false,"min_level":1}"#, "\"min_level\" only applies to starting a twinkle"),
        ("dmx/group/House", r#"{"value":1,"max_level":1}"#, "\"max_level\" only applies to starting a twinkle"),
        ("dmx/group/House", r#"{"twinkle":true,"min_level":256}"#, "min_level must be a whole number from 0 to 255 (got 256)"),
        ("dmx/group/House", r#"{"twinkle":true,"min_level":200,"max_level":100}"#, "min_level (200) must not be greater than max_level (100)"),
        ("dmx/group/House", r#"{"twinkle":true,"min_duration":0,"max_duration":0}"#, "min_duration must be between 200 and 600000 ms (got 0); max_duration must be between 200 and 600000 ms (got 0)"),
        ("dmx/group/House", r#"{"twinkle":true,"max_level":50}"#, "min_level (100) must not be greater than max_level (50)"),
        ("dmx/group/Nowhere", r#"{"value":1}"#, "unknown group \"Nowhere\""),
        ("dmx/channel/0", "255", "channel must be from 1 to 512 (got \"0\")"),
        ("dmx/channel/513", "255", "channel must be from 1 to 512 (got \"513\")"),
        ("dmx/channel/abc", "255", "channel must be from 1 to 512 (got \"abc\")"),
        ("dmx/channel/+5", "255", "channel must be from 1 to 512 (got \"+5\")"),
        ("dmx", "255", "the base topic needs a JSON object with a target and an action"),
        ("dmx", r#"{"value":255}"#, "the base topic needs one target: \"group\", \"channel\" or \"range\""),
        ("dmx", r#"{"channel":1,"group":"House","value":255}"#, "the base topic needs one target: \"group\", \"channel\" or \"range\" (got \"group\" and \"channel\")"),
        ("dmx", r#"{"channel":0,"value":255}"#, "channel must be from 1 to 512 (got 0)"),
        ("dmx", r#"{"channel":"5","value":255}"#, "channel must be from 1 to 512 (got \"5\")"),
        ("dmx", r#"{"group":5,"value":255}"#, "group must be a group name (got 5)"),
        ("dmx", r#"{"group":"Nowhere","value":255}"#, "unknown group \"Nowhere\""),
        ("dmx", r#"{"range":{"start":0,"end":512},"value":0}"#, "range must have 1 <= start <= end <= 512 (got start 0, end 512)"),
        ("dmx", r#"{"range":{"start":10,"end":5},"value":0}"#, "range must have 1 <= start <= end <= 512 (got start 10, end 5)"),
        ("dmx", r#"{"range":{"start":1},"value":0}"#, "range must be {\"start\": <channel>, \"end\": <channel>}"),
        ("dmx", r#"{"range":{"start":1,"end":2,"step":1},"value":0}"#, "range must be {\"start\": <channel>, \"end\": <channel>}"),
        ("dmx", r#"{"range":[1,2],"value":0}"#, "range must be {\"start\": <channel>, \"end\": <channel>}"),
        ("dmx", r#"{"channels":{"1":255}}"#, "unknown field \"channels\""),
        ("dmx/other/thing", "255", "unrecognised topic"),
        ("dmx/group/", "255", "unrecognised topic"),
        ("dmx/group/a/b", "255", "unrecognised topic"),
        ("elsewhere", "255", "unrecognised topic"),
    ];
    for (topic, payload, expected) in cases {
        let error = reject(topic, payload);
        assert_eq!(error, expected, "{topic} {payload:?}");
    }
}

#[test]
fn payloads_must_be_utf8() {
    let error = parser()
        .parse("dmx/channel/1", &[0xff, 0xfe])
        .expect_err("invalid UTF-8");
    assert_eq!(error.to_string(), "payload is not UTF-8 text");
}

#[test]
fn a_custom_base_topic_is_honoured() {
    let mut config = Config::default();
    config.output.driver = Some(Driver::Null);
    config.mqtt.topic = "venue/lights".to_string();
    config.groups.insert(
        "A".to_string(),
        GroupConfig {
            channels: vec![3],
            description: None,
        },
    );
    let parser = CommandParser::new(&config);
    assert_eq!(
        parser.subscriptions(),
        vec![
            "venue/lights".to_string(),
            "venue/lights/channel/+".to_string(),
            "venue/lights/group/+".to_string()
        ]
    );
    assert_eq!(
        parser
            .parse("venue/lights/group/A", b"1")
            .expect("parses")
            .channels,
        vec![2]
    );
    assert_eq!(
        parser
            .parse("venue/lights", br#"{"channel":4,"value":1}"#)
            .expect("parses")
            .channels,
        vec![3]
    );
    assert!(parser.parse("dmx/group/A", b"1").is_err());
}
