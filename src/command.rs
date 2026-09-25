// ABOUTME: Turns MQTT topics and payloads into validated lighting commands.
// ABOUTME: A message is checked completely; any problem rejects it with a reason.

use std::collections::HashMap;
use std::fmt;

use serde_json::{Map, Value};

use crate::config::{twinkle_problems, Config, Easing, TwinkleConfig, CHANNEL_COUNT};

/// Longest fade a command may ask for: 24 hours, in ms.
pub const MAX_FADE_MS: f64 = 86_400_000.0;

const TARGET_FIELDS: [&str; 3] = ["group", "channel", "range"];
const ACTION_FIELDS: [&str; 9] = [
    "value",
    "fade",
    "easing",
    "pulse",
    "twinkle",
    "min_level",
    "max_level",
    "min_duration",
    "max_duration",
];
const TWINKLE_FIELDS: [&str; 4] = ["min_level", "max_level", "min_duration", "max_duration"];

/// Which channels a command acts on, as the sender named them.
#[derive(Debug, Clone, PartialEq)]
pub enum Target {
    Group(String),
    Channel(u16),
    Range { start: u16, end: u16 },
}

impl fmt::Display for Target {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Target::Group(name) => write!(f, "group {name}"),
            Target::Channel(channel) => write!(f, "channel {channel}"),
            Target::Range { start, end } => write!(f, "channels {start}-{end}"),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PulseSpeed {
    Slow,
    Fast,
}

impl PulseSpeed {
    /// Rise, hold and fall times in ms.
    pub fn timings_ms(self) -> (f64, f64, f64) {
        match self {
            PulseSpeed::Slow => (1500.0, 2000.0, 1500.0),
            PulseSpeed::Fast => (500.0, 500.0, 500.0),
        }
    }

    fn name(self) -> &'static str {
        match self {
            PulseSpeed::Slow => "slow",
            PulseSpeed::Fast => "fast",
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct TwinkleParams {
    pub min_level: u8,
    pub max_level: u8,
    pub min_duration_ms: f64,
    pub max_duration_ms: f64,
    pub easing: Easing,
}

#[derive(Debug, Clone, PartialEq)]
pub enum Action {
    /// Go to `level`, instantly when `fade_ms` is 0.
    Set {
        level: u8,
        fade_ms: f64,
        easing: Easing,
    },
    Pulse {
        speed: PulseSpeed,
        easing: Easing,
    },
    TwinkleStart(TwinkleParams),
    /// Stop twinkling and return to the commanded level.
    TwinkleStop {
        fade_ms: f64,
        easing: Easing,
    },
}

impl fmt::Display for Action {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Action::Set { level, fade_ms, .. } if *fade_ms == 0.0 => write!(f, "set to {level}"),
            Action::Set {
                level,
                fade_ms,
                easing,
            } => {
                write!(f, "fade to {level} over {fade_ms} ms ({})", easing.name())
            }
            Action::Pulse { speed, .. } => write!(f, "pulse ({})", speed.name()),
            Action::TwinkleStart(params) => write!(
                f,
                "twinkle between {} and {} every {}-{} ms ({})",
                params.min_level,
                params.max_level,
                params.min_duration_ms,
                params.max_duration_ms,
                params.easing.name()
            ),
            Action::TwinkleStop { fade_ms, .. } if *fade_ms == 0.0 => write!(f, "stop twinkle"),
            Action::TwinkleStop { fade_ms, easing } => write!(
                f,
                "stop twinkle, return over {fade_ms} ms ({})",
                easing.name()
            ),
        }
    }
}

/// A validated command. `channels` holds 0-based channel indices.
#[derive(Debug, Clone, PartialEq)]
pub struct Command {
    pub target: Target,
    pub channels: Vec<usize>,
    pub action: Action,
}

impl fmt::Display for Command {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}: {}", self.target, self.action)
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct CommandError(String);

impl CommandError {
    fn new(message: impl Into<String>) -> Self {
        CommandError(message.into())
    }
}

impl fmt::Display for CommandError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

impl std::error::Error for CommandError {}

type ParseResult<T> = Result<T, CommandError>;

/// Parses and validates commands against the configured topic and groups.
pub struct CommandParser {
    topic: String,
    channel_prefix: String,
    group_prefix: String,
    groups: HashMap<String, Vec<usize>>,
    fade_easing: Easing,
    twinkle: TwinkleConfig,
}

impl CommandParser {
    pub fn new(config: &Config) -> Self {
        let topic = config.mqtt.topic.clone();
        let groups = config
            .groups
            .iter()
            .map(|(name, group)| {
                let indices = group.channels.iter().map(|&c| usize::from(c) - 1).collect();
                (name.clone(), indices)
            })
            .collect();
        CommandParser {
            channel_prefix: format!("{topic}/channel/"),
            group_prefix: format!("{topic}/group/"),
            topic,
            groups,
            fade_easing: config.fade.easing,
            twinkle: config.twinkle.clone(),
        }
    }

    /// The topic filters mqttdmx subscribes to. They do not overlap, so each
    /// command arrives once, and none of them matches the status topics.
    pub fn subscriptions(&self) -> Vec<String> {
        vec![
            self.topic.clone(),
            format!("{}+", self.channel_prefix),
            format!("{}+", self.group_prefix),
        ]
    }

    pub fn parse(&self, topic: &str, payload: &[u8]) -> ParseResult<Command> {
        let text = std::str::from_utf8(payload)
            .map_err(|_| CommandError::new("payload is not UTF-8 text"))?
            .trim();

        if topic == self.topic {
            return self.parse_addressed_payload(text);
        }
        if let Some(name) = topic.strip_prefix(&self.group_prefix) {
            if !name.is_empty() && !name.contains('/') {
                let target = Target::Group(name.to_string());
                let channels = self.group_channels(name)?;
                let action = self.parse_topic_payload(text)?;
                return Ok(Command {
                    target,
                    channels,
                    action,
                });
            }
        }
        if let Some(number) = topic.strip_prefix(&self.channel_prefix) {
            if !number.contains('/') {
                let channel = parse_channel_segment(number)?;
                let action = self.parse_topic_payload(text)?;
                return Ok(Command {
                    target: Target::Channel(channel),
                    channels: vec![usize::from(channel) - 1],
                    action,
                });
            }
        }
        Err(CommandError::new("unrecognised topic"))
    }

    fn group_channels(&self, name: &str) -> ParseResult<Vec<usize>> {
        self.groups
            .get(name)
            .cloned()
            .ok_or_else(|| CommandError::new(format!("unknown group {name:?}")))
    }

    /// A payload for a target named by the topic: a bare level or an action object.
    fn parse_topic_payload(&self, text: &str) -> ParseResult<Action> {
        match parse_json(text)? {
            Value::Number(number) => {
                let level = whole_level(&Value::Number(number.clone())).ok_or_else(|| {
                    CommandError::new(format!(
                        "a level must be a whole number from 0 to 255 (got {number})"
                    ))
                })?;
                Ok(Action::Set {
                    level,
                    fade_ms: 0.0,
                    easing: self.fade_easing,
                })
            }
            Value::Object(fields) => {
                reject_unknown_fields(&fields, &ACTION_FIELDS)?;
                self.parse_action(&fields)
            }
            other => Err(CommandError::new(format!(
                "payload must be a level from 0 to 255 or a JSON object (got {other})"
            ))),
        }
    }

    /// A payload on the base topic: an object naming both target and action.
    fn parse_addressed_payload(&self, text: &str) -> ParseResult<Command> {
        let Value::Object(fields) = parse_json(text)? else {
            return Err(CommandError::new(
                "the base topic needs a JSON object with a target and an action",
            ));
        };
        let allowed: Vec<&str> = TARGET_FIELDS
            .iter()
            .chain(ACTION_FIELDS.iter())
            .copied()
            .collect();
        reject_unknown_fields(&fields, &allowed)?;
        let (target, channels) = self.parse_target(&fields)?;
        let action = self.parse_action(&fields)?;
        Ok(Command {
            target,
            channels,
            action,
        })
    }

    fn parse_target(&self, fields: &Map<String, Value>) -> ParseResult<(Target, Vec<usize>)> {
        let present: Vec<&str> = TARGET_FIELDS
            .iter()
            .copied()
            .filter(|key| fields.contains_key(*key))
            .collect();
        let key = match present.as_slice() {
            [key] => *key,
            [] => {
                return Err(CommandError::new(
                    "the base topic needs one target: \"group\", \"channel\" or \"range\"",
                ))
            }
            several => {
                return Err(CommandError::new(format!(
                    "the base topic needs one target: \"group\", \"channel\" or \"range\" (got {})",
                    quoted_list(several)
                )))
            }
        };
        let value = &fields[key];
        match key {
            "group" => {
                let name = value.as_str().ok_or_else(|| {
                    CommandError::new(format!("group must be a group name (got {value})"))
                })?;
                Ok((Target::Group(name.to_string()), self.group_channels(name)?))
            }
            "channel" => {
                let channel = channel_number(value).ok_or_else(|| {
                    CommandError::new(format!(
                        "channel must be from 1 to {CHANNEL_COUNT} (got {value})"
                    ))
                })?;
                Ok((Target::Channel(channel), vec![usize::from(channel) - 1]))
            }
            _ => {
                let (start, end) = parse_range(value)?;
                let channels = (usize::from(start) - 1..usize::from(end)).collect();
                Ok((Target::Range { start, end }, channels))
            }
        }
    }

    fn parse_action(&self, fields: &Map<String, Value>) -> ParseResult<Action> {
        let value = optional(fields, "value", level_field)?;
        let fade = optional(fields, "fade", fade_field)?;
        let easing = optional(fields, "easing", easing_field)?;
        let pulse = optional(fields, "pulse", pulse_field)?;
        let twinkle = optional(fields, "twinkle", twinkle_field)?;

        let actions: Vec<&str> = [
            ("value", value.is_some()),
            ("pulse", pulse.is_some()),
            ("twinkle", twinkle.is_some()),
        ]
        .into_iter()
        .filter_map(|(name, present)| present.then_some(name))
        .collect();
        if actions.is_empty() {
            return Err(CommandError::new(
                "no action: expected \"value\", \"pulse\" or \"twinkle\"",
            ));
        }
        if actions.len() > 1 {
            return Err(CommandError::new(format!(
                "only one action per message (got {})",
                quoted_list(&actions)
            )));
        }

        let starting_twinkle = twinkle == Some(true);
        if !starting_twinkle {
            if let Some(field) = TWINKLE_FIELDS.iter().find(|f| fields.contains_key(**f)) {
                return Err(CommandError::new(format!(
                    "{field:?} only applies to starting a twinkle"
                )));
            }
        }

        if let Some(level) = value {
            return Ok(Action::Set {
                level,
                fade_ms: fade.unwrap_or(0.0),
                easing: easing.unwrap_or(self.fade_easing),
            });
        }
        if let Some(speed) = pulse {
            for field in ["fade", "easing"] {
                if fields.contains_key(field) {
                    return Err(CommandError::new(format!(
                        "{field:?} does not apply to pulse"
                    )));
                }
            }
            return Ok(Action::Pulse {
                speed,
                easing: self.fade_easing,
            });
        }
        if starting_twinkle {
            if fields.contains_key("fade") {
                return Err(CommandError::new(
                    "\"fade\" does not apply to starting a twinkle",
                ));
            }
            return self.parse_twinkle_start(fields, easing);
        }
        Ok(Action::TwinkleStop {
            fade_ms: fade.unwrap_or(0.0),
            easing: easing.unwrap_or(self.fade_easing),
        })
    }

    fn parse_twinkle_start(
        &self,
        fields: &Map<String, Value>,
        easing: Option<Easing>,
    ) -> ParseResult<Action> {
        let defaults = &self.twinkle;
        let params = TwinkleParams {
            min_level: optional(fields, "min_level", level_field)?.unwrap_or(defaults.min_level),
            max_level: optional(fields, "max_level", level_field)?.unwrap_or(defaults.max_level),
            min_duration_ms: optional(fields, "min_duration", number_field)?
                .unwrap_or(defaults.min_duration),
            max_duration_ms: optional(fields, "max_duration", number_field)?
                .unwrap_or(defaults.max_duration),
            easing: easing.unwrap_or(defaults.easing),
        };
        let problems = twinkle_problems(
            "",
            params.min_level,
            params.max_level,
            params.min_duration_ms,
            params.max_duration_ms,
        );
        if problems.is_empty() {
            Ok(Action::TwinkleStart(params))
        } else {
            Err(CommandError::new(problems.join("; ")))
        }
    }
}

fn parse_json(text: &str) -> ParseResult<Value> {
    if text.is_empty() {
        return Err(CommandError::new("empty payload"));
    }
    serde_json::from_str(text)
        .map_err(|_| CommandError::new("payload is neither a number nor valid JSON"))
}

fn reject_unknown_fields(fields: &Map<String, Value>, allowed: &[&str]) -> ParseResult<()> {
    let mut unknown: Vec<&str> = fields
        .keys()
        .map(String::as_str)
        .filter(|key| !allowed.contains(key))
        .collect();
    unknown.sort_unstable();
    match unknown.as_slice() {
        [] => Ok(()),
        [one] => Err(CommandError::new(format!("unknown field {one:?}"))),
        several => Err(CommandError::new(format!(
            "unknown fields {}",
            several
                .iter()
                .map(|k| format!("{k:?}"))
                .collect::<Vec<_>>()
                .join(", ")
        ))),
    }
}

/// Parses `fields[key]` with `parse` when the key is present, including when it is `null`.
fn optional<T>(
    fields: &Map<String, Value>,
    key: &str,
    parse: fn(&str, &Value) -> ParseResult<T>,
) -> ParseResult<Option<T>> {
    fields.get(key).map(|value| parse(key, value)).transpose()
}

fn whole_level(value: &Value) -> Option<u8> {
    let number = value.as_f64()?;
    if number.fract() == 0.0 && (0.0..=255.0).contains(&number) {
        Some(number as u8)
    } else {
        None
    }
}

fn level_field(key: &str, value: &Value) -> ParseResult<u8> {
    whole_level(value).ok_or_else(|| {
        CommandError::new(format!(
            "{key} must be a whole number from 0 to 255 (got {value})"
        ))
    })
}

fn fade_field(key: &str, value: &Value) -> ParseResult<f64> {
    value
        .as_f64()
        .filter(|ms| (0.0..=MAX_FADE_MS).contains(ms))
        .ok_or_else(|| {
            CommandError::new(format!(
                "{key} must be a number of ms from 0 to {MAX_FADE_MS} (got {value})"
            ))
        })
}

fn number_field(key: &str, value: &Value) -> ParseResult<f64> {
    value
        .as_f64()
        .ok_or_else(|| CommandError::new(format!("{key} must be a number of ms (got {value})")))
}

fn easing_field(key: &str, value: &Value) -> ParseResult<Easing> {
    match value.as_str() {
        Some("linear") => Ok(Easing::Linear),
        Some("sine") => Ok(Easing::Sine),
        _ => Err(CommandError::new(format!(
            "{key} must be \"linear\" or \"sine\" (got {value})"
        ))),
    }
}

fn pulse_field(key: &str, value: &Value) -> ParseResult<PulseSpeed> {
    match value.as_str() {
        Some("slow") => Ok(PulseSpeed::Slow),
        Some("fast") => Ok(PulseSpeed::Fast),
        _ => Err(CommandError::new(format!(
            "{key} must be \"slow\" or \"fast\" (got {value})"
        ))),
    }
}

fn twinkle_field(key: &str, value: &Value) -> ParseResult<bool> {
    value
        .as_bool()
        .ok_or_else(|| CommandError::new(format!("{key} must be true or false (got {value})")))
}

fn channel_number(value: &Value) -> Option<u16> {
    let number = value.as_u64()?;
    (1..=CHANNEL_COUNT as u64)
        .contains(&number)
        .then_some(number as u16)
}

fn parse_channel_segment(segment: &str) -> ParseResult<u16> {
    let number = if segment.bytes().all(|b| b.is_ascii_digit()) {
        segment.parse::<u16>().ok()
    } else {
        None
    };
    number
        .filter(|n| (1..=CHANNEL_COUNT as u16).contains(n))
        .ok_or_else(|| {
            CommandError::new(format!(
                "channel must be from 1 to {CHANNEL_COUNT} (got {segment:?})"
            ))
        })
}

fn parse_range(value: &Value) -> ParseResult<(u16, u16)> {
    let shape_error =
        || CommandError::new("range must be {\"start\": <channel>, \"end\": <channel>}");
    let fields = value.as_object().ok_or_else(shape_error)?;
    if fields.len() != 2 {
        return Err(shape_error());
    }
    let start = fields
        .get("start")
        .and_then(Value::as_u64)
        .ok_or_else(shape_error)?;
    let end = fields
        .get("end")
        .and_then(Value::as_u64)
        .ok_or_else(shape_error)?;
    if 1 <= start && start <= end && end <= CHANNEL_COUNT as u64 {
        Ok((start as u16, end as u16))
    } else {
        Err(CommandError::new(format!(
            "range must have 1 <= start <= end <= {CHANNEL_COUNT} (got start {start}, end {end})"
        )))
    }
}

fn quoted_list(items: &[&str]) -> String {
    items
        .iter()
        .map(|item| format!("{item:?}"))
        .collect::<Vec<_>>()
        .join(" and ")
}
