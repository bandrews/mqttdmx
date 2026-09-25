// ABOUTME: The status documents mqttdmx publishes: commanded group levels and output health.
// ABOUTME: Group levels are derived from channel levels, so overlapping groups stay consistent.

use serde::Serialize;
use serde_json::{Map, Value};

use crate::config::{Config, CHANNEL_COUNT};

/// Works out each configured group's commanded level from the channel levels.
pub struct GroupLevels {
    groups: Vec<(String, Vec<usize>)>,
}

impl GroupLevels {
    pub fn new(config: &Config) -> Self {
        let groups = config
            .groups
            .iter()
            .map(|(name, group)| {
                let indices = group.channels.iter().map(|&c| usize::from(c) - 1).collect();
                (name.clone(), indices)
            })
            .collect();
        GroupLevels { groups }
    }

    /// `{"<group>": level}`, with `null` for a group whose channels differ or
    /// that has no channels. Groups are listed in name order.
    pub fn to_json(&self, commanded: &[u8; CHANNEL_COUNT]) -> String {
        let mut levels = Map::new();
        for (name, channels) in &self.groups {
            let mut shared = channels.iter().map(|&index| commanded[index]);
            let level = match shared.next() {
                Some(first) if shared.all(|level| level == first) => Value::from(first),
                _ => Value::Null,
            };
            levels.insert(name.clone(), level);
        }
        Value::Object(levels).to_string()
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum OutputState {
    /// Frames are reaching the interface.
    Connected,
    /// The interface is missing or failed; mqttdmx is retrying.
    Disconnected,
    /// The null driver: nothing is sent.
    Disabled,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct OutputStatus {
    pub driver: &'static str,
    pub device: Option<String>,
    pub state: OutputState,
    pub firmware: Option<String>,
    pub error: Option<String>,
}

#[derive(Serialize)]
struct Health<'a> {
    version: &'static str,
    output: &'a OutputStatus,
}

pub fn health_json(output: &OutputStatus) -> String {
    let health = Health {
        version: env!("CARGO_PKG_VERSION"),
        output,
    };
    serde_json::to_string(&health).unwrap_or_else(|_| "{}".to_string())
}
