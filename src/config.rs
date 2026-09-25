// ABOUTME: Configuration file model, command-line overrides and startup validation.
// ABOUTME: Unknown top-level keys are ignored; unknown keys inside a section are errors.

use std::collections::{BTreeMap, BTreeSet};
use std::fmt;
use std::path::{Path, PathBuf};

use serde::Deserialize;

/// Number of channels in one DMX universe.
pub const CHANNEL_COUNT: usize = 512;

/// Highest frame rate a full 512-channel DMX universe can carry.
pub const MAX_FRAME_RATE: u32 = 44;

/// Shortest and longest time a twinkle may take to move to a new level, in ms.
/// The floor keeps twinkle from becoming a strobe.
pub const TWINKLE_MIN_DURATION_MS: f64 = 200.0;
pub const TWINKLE_MAX_DURATION_MS: f64 = 600_000.0;

#[derive(Debug, Clone, PartialEq, Default, Deserialize)]
#[serde(default)]
pub struct Config {
    pub mqtt: MqttConfig,
    pub output: OutputConfig,
    pub fade: FadeConfig,
    pub twinkle: TwinkleConfig,
    pub groups: BTreeMap<String, GroupConfig>,
    pub logging: LoggingConfig,
}

#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct MqttConfig {
    pub server: String,
    pub port: u16,
    pub topic: String,
    pub client_id: Option<String>,
    pub username: Option<String>,
    pub password: Option<String>,
}

impl Default for MqttConfig {
    fn default() -> Self {
        Self {
            server: "localhost".to_string(),
            port: 1883,
            topic: "dmx".to_string(),
            client_id: None,
            username: None,
            password: None,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
pub enum Driver {
    #[serde(rename = "enttec-usb-dmx-pro")]
    EnttecUsbDmxPro,
    #[serde(rename = "null")]
    Null,
}

impl Driver {
    pub fn name(self) -> &'static str {
        match self {
            Driver::EnttecUsbDmxPro => "enttec-usb-dmx-pro",
            Driver::Null => "null",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct OutputConfig {
    pub driver: Option<Driver>,
    pub device: Option<String>,
    pub frame_rate: u32,
    pub startup_level: u8,
}

impl Default for OutputConfig {
    fn default() -> Self {
        Self {
            driver: None,
            device: None,
            frame_rate: 40,
            startup_level: 0,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Easing {
    #[default]
    Linear,
    Sine,
}

impl Easing {
    pub fn name(self) -> &'static str {
        match self {
            Easing::Linear => "linear",
            Easing::Sine => "sine",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Default, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct FadeConfig {
    pub easing: Easing,
}

#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct TwinkleConfig {
    pub min_level: u8,
    pub max_level: u8,
    pub min_duration: f64,
    pub max_duration: f64,
    pub easing: Easing,
}

impl Default for TwinkleConfig {
    fn default() -> Self {
        Self {
            min_level: 100,
            max_level: 255,
            min_duration: 500.0,
            max_duration: 2000.0,
            easing: Easing::Sine,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct GroupConfig {
    pub channels: Vec<u16>,
    #[serde(default)]
    pub description: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum LogLevel {
    Error,
    Warn,
    #[default]
    Info,
    Debug,
    Trace,
}

impl LogLevel {
    pub fn name(self) -> &'static str {
        match self {
            LogLevel::Error => "error",
            LogLevel::Warn => "warn",
            LogLevel::Info => "info",
            LogLevel::Debug => "debug",
            LogLevel::Trace => "trace",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum LogFormat {
    #[default]
    Text,
    Json,
}

#[derive(Debug, Clone, PartialEq, Default, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct LoggingConfig {
    pub level: LogLevel,
    pub format: LogFormat,
}

/// Values given on the command line. Each one set replaces the file's value.
#[derive(Debug, Clone, Default)]
pub struct Overrides {
    pub server: Option<String>,
    pub port: Option<u16>,
    pub topic: Option<String>,
    pub username: Option<String>,
    pub password: Option<String>,
    pub driver: Option<Driver>,
    pub device: Option<String>,
    pub startup_level: Option<u8>,
    pub log_format: Option<LogFormat>,
    pub verbose: bool,
}

#[derive(Debug)]
pub enum ConfigError {
    Read {
        path: PathBuf,
        source: std::io::Error,
    },
    Parse(serde_json::Error),
}

impl fmt::Display for ConfigError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ConfigError::Read { path, source } => {
                write!(f, "cannot read config file {}: {source}", path.display())
            }
            ConfigError::Parse(source) => write!(f, "invalid config file: {source}"),
        }
    }
}

impl std::error::Error for ConfigError {}

/// Reads and parses a config file. The result still needs `validate`.
pub fn load(path: &Path) -> Result<Config, ConfigError> {
    let text = std::fs::read_to_string(path).map_err(|source| ConfigError::Read {
        path: path.to_path_buf(),
        source,
    })?;
    Config::from_json(&text)
}

impl Config {
    pub fn from_json(text: &str) -> Result<Config, ConfigError> {
        serde_json::from_str(text).map_err(ConfigError::Parse)
    }

    pub fn apply_overrides(&mut self, overrides: &Overrides) {
        fn replace<T: Clone>(target: &mut T, value: &Option<T>) {
            if let Some(value) = value {
                *target = value.clone();
            }
        }
        fn replace_optional<T: Clone>(target: &mut Option<T>, value: &Option<T>) {
            if value.is_some() {
                *target = value.clone();
            }
        }

        replace(&mut self.mqtt.server, &overrides.server);
        replace(&mut self.mqtt.port, &overrides.port);
        replace(&mut self.mqtt.topic, &overrides.topic);
        replace_optional(&mut self.mqtt.username, &overrides.username);
        replace_optional(&mut self.mqtt.password, &overrides.password);
        replace_optional(&mut self.output.driver, &overrides.driver);
        replace_optional(&mut self.output.device, &overrides.device);
        replace(&mut self.output.startup_level, &overrides.startup_level);
        replace(&mut self.logging.format, &overrides.log_format);
        if overrides.verbose {
            self.logging.level = LogLevel::Debug;
        }
    }

    /// Checks everything the types alone cannot, and returns every problem found.
    pub fn validate(&self) -> Result<(), Vec<String>> {
        let mut errors = Vec::new();
        self.validate_mqtt(&mut errors);
        self.validate_output(&mut errors);
        self.validate_twinkle(&mut errors);
        self.validate_groups(&mut errors);
        if errors.is_empty() {
            Ok(())
        } else {
            Err(errors)
        }
    }

    fn validate_mqtt(&self, errors: &mut Vec<String>) {
        let mqtt = &self.mqtt;
        if mqtt.server.trim().is_empty() {
            errors.push("mqtt.server must not be empty".to_string());
        }
        if mqtt.port == 0 {
            errors.push("mqtt.port must not be 0".to_string());
        }
        let topic = &mqtt.topic;
        if topic.is_empty() {
            errors.push("mqtt.topic must not be empty".to_string());
        } else if topic.contains('+') || topic.contains('#') {
            errors.push(format!(
                "mqtt.topic must not contain the wildcards + or # (got {topic:?})"
            ));
        } else if topic.starts_with('/') || topic.ends_with('/') {
            errors.push(format!(
                "mqtt.topic must not start or end with / (got {topic:?})"
            ));
        } else if topic.split('/').any(str::is_empty) {
            errors.push(format!(
                "mqtt.topic must not contain an empty level (got {topic:?})"
            ));
        }
        if mqtt
            .client_id
            .as_deref()
            .is_some_and(|id| id.trim().is_empty())
        {
            errors.push("mqtt.client_id must not be empty when set".to_string());
        }
        if mqtt.username.is_some() != mqtt.password.is_some() {
            errors.push("mqtt.username and mqtt.password must be set together".to_string());
        }
    }

    fn validate_output(&self, errors: &mut Vec<String>) {
        let output = &self.output;
        match output.driver {
            None => {
                errors.push("output.driver is required: enttec-usb-dmx-pro or null".to_string())
            }
            Some(Driver::EnttecUsbDmxPro) => {
                if output
                    .device
                    .as_deref()
                    .is_none_or(|device| device.trim().is_empty())
                {
                    errors.push(
                        "output.device is required for the enttec-usb-dmx-pro driver: a serial device path or \"auto\""
                            .to_string(),
                    );
                }
            }
            Some(Driver::Null) => {}
        }
        if !(1..=MAX_FRAME_RATE).contains(&output.frame_rate) {
            errors.push(format!(
                "output.frame_rate must be between 1 and {MAX_FRAME_RATE} (got {})",
                output.frame_rate
            ));
        }
    }

    fn validate_twinkle(&self, errors: &mut Vec<String>) {
        let twinkle = &self.twinkle;
        errors.extend(twinkle_problems(
            "twinkle.",
            twinkle.min_level,
            twinkle.max_level,
            twinkle.min_duration,
            twinkle.max_duration,
        ));
    }

    fn validate_groups(&self, errors: &mut Vec<String>) {
        for (name, group) in &self.groups {
            if name.is_empty() {
                errors.push("groups: a group name must not be empty".to_string());
            } else if name.contains(['/', '+', '#']) {
                errors.push(format!(
                    "groups.{name}: a group name must not contain /, + or #"
                ));
            }
            let mut seen = BTreeSet::new();
            let mut repeated = BTreeSet::new();
            for &channel in &group.channels {
                if !(1..=CHANNEL_COUNT as u16).contains(&channel) {
                    errors.push(format!(
                        "groups.{name}: channel {channel} is outside 1-{CHANNEL_COUNT}"
                    ));
                } else if !seen.insert(channel) && repeated.insert(channel) {
                    errors.push(format!(
                        "groups.{name}: channel {channel} is listed more than once"
                    ));
                }
            }
        }
    }
}

/// Checks a complete set of twinkle settings. `prefix` names where they came
/// from in the messages, e.g. `"twinkle."` for the config file.
pub fn twinkle_problems(
    prefix: &str,
    min_level: u8,
    max_level: u8,
    min_duration: f64,
    max_duration: f64,
) -> Vec<String> {
    let mut problems = Vec::new();
    if min_level > max_level {
        problems.push(format!(
            "{prefix}min_level ({min_level}) must not be greater than {prefix}max_level ({max_level})"
        ));
    }
    let mut durations_in_range = true;
    for (field, value) in [
        ("min_duration", min_duration),
        ("max_duration", max_duration),
    ] {
        if !(TWINKLE_MIN_DURATION_MS..=TWINKLE_MAX_DURATION_MS).contains(&value) {
            durations_in_range = false;
            problems.push(format!(
                "{prefix}{field} must be between {TWINKLE_MIN_DURATION_MS} and {TWINKLE_MAX_DURATION_MS} ms (got {value})"
            ));
        }
    }
    if durations_in_range && min_duration > max_duration {
        problems.push(format!(
            "{prefix}min_duration ({min_duration}) must not be greater than {prefix}max_duration ({max_duration})"
        ));
    }
    problems
}
