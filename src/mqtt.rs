// ABOUTME: The MQTT link: receives commands for the render loop and publishes retained status.
// ABOUTME: Reconnects with backoff, resubscribes and republishes status on every connection.

use std::sync::mpsc::Sender;
use std::time::Duration;

use rand::Rng;
use rumqttc::{
    AsyncClient, Event, EventLoop, LastWill, MqttOptions, Outgoing, Packet, Publish, QoS,
    SubscribeReasonCode,
};
use tokio::sync::watch;
use tokio::time::Instant;
use tracing::{debug, error, info, warn};

use crate::command::CommandParser;
use crate::config::{MqttConfig, CHANNEL_COUNT};
use crate::render::RenderMessage;
use crate::status::{health_json, GroupLevels, OutputStatus};

const KEEP_ALIVE: Duration = Duration::from_secs(10);
/// Large enough for any command; an oversized packet makes the broker connection fail.
const MAX_PACKET_SIZE: usize = 1024 * 1024;
const REQUEST_QUEUE: usize = 64;
const FIRST_RECONNECT: Duration = Duration::from_millis(500);
const LONGEST_RECONNECT: Duration = Duration::from_secs(10);
/// A connection that lasts this long resets the reconnect backoff when it fails.
/// A connection that fails sooner, for example because the broker sends an
/// oversized retained message straight after subscribing, keeps backing off.
const STABLE_CONNECTION: Duration = Duration::from_secs(5);
/// Repeat a warning about an unreachable broker after this many further failures.
const FAILURES_BETWEEN_REMINDERS: u32 = 10;
/// At most four group-level updates a second.
const GROUPS_INTERVAL: Duration = Duration::from_millis(250);
/// How soon to try again when the client's request queue was full.
const PUBLISH_RETRY: Duration = Duration::from_millis(100);
const LOG_PAYLOAD_LIMIT: usize = 200;

/// The connection as the event loop last saw it. `generation` counts
/// successful connections, so a reconnect is visible even when the
/// disconnection in between was too short to observe.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct Connection {
    generation: u64,
    connected: bool,
}

/// Where the render loop's status comes from and where commands go.
pub struct Channels {
    pub commands: Sender<RenderMessage>,
    pub levels: watch::Receiver<[u8; CHANNEL_COUNT]>,
    pub output: watch::Receiver<OutputStatus>,
    /// Becomes `true` when mqttdmx is shutting down.
    pub shutdown: watch::Receiver<bool>,
}

/// Runs the MQTT link until shutdown has been announced and the broker has
/// been told goodbye. When the broker is unreachable at shutdown this never
/// returns, so the caller bounds it with a timeout.
pub async fn run(
    config: MqttConfig,
    parser: CommandParser,
    groups: GroupLevels,
    channels: Channels,
) {
    let client_id = config
        .client_id
        .clone()
        .unwrap_or_else(|| format!("mqttdmx-{:08x}", rand::thread_rng().gen::<u32>()));
    let status_topic = format!("{}/status", config.topic);
    let mut options = MqttOptions::new(client_id, config.server.clone(), config.port);
    options
        .set_keep_alive(KEEP_ALIVE)
        .set_clean_session(true)
        .set_max_packet_size(MAX_PACKET_SIZE, MAX_PACKET_SIZE)
        .set_last_will(LastWill::new(
            format!("{status_topic}/online"),
            "offline",
            QoS::AtLeastOnce,
            true,
        ));
    if let (Some(username), Some(password)) = (&config.username, &config.password) {
        options.set_credentials(username, password);
    }
    let (client, eventloop) = AsyncClient::new(options, REQUEST_QUEUE);
    let (connection_tx, connection_rx) = watch::channel(Connection {
        generation: 0,
        connected: false,
    });
    let broker = format!("{}:{}", config.server, config.port);
    let Channels {
        commands,
        levels,
        output,
        shutdown,
    } = channels;

    let events = receive(
        eventloop,
        client.clone(),
        broker,
        parser,
        commands,
        connection_tx,
    );
    let status = StatusPublisher {
        client,
        topic: status_topic,
        groups,
        connection: connection_rx,
        levels,
        output,
        shutdown,
    }
    .run();
    tokio::join!(events, status);
}

/// Drives the MQTT event loop: connects, subscribes, and hands every command
/// to the render loop. Returns once a requested disconnect has been sent.
async fn receive(
    mut eventloop: EventLoop,
    client: AsyncClient,
    broker: String,
    parser: CommandParser,
    commands: Sender<RenderMessage>,
    connection: watch::Sender<Connection>,
) {
    let subscriptions = parser.subscriptions();
    let mut retry_delay = FIRST_RECONNECT;
    let mut failures: u32 = 0;
    let mut last_error: Option<String> = None;
    let mut connected_at: Option<Instant> = None;

    loop {
        match eventloop.poll().await {
            Ok(Event::Incoming(Packet::ConnAck(_))) => {
                if failures > 0 {
                    info!("connected to MQTT broker {broker} after {failures} failed attempts");
                } else {
                    info!("connected to MQTT broker {broker}");
                }
                connected_at = Some(Instant::now());
                for filter in &subscriptions {
                    if let Err(e) = client.try_subscribe(filter.as_str(), QoS::AtLeastOnce) {
                        error!("cannot subscribe to {filter}: {e}");
                    }
                }
                connection.send_modify(|c| {
                    c.generation += 1;
                    c.connected = true;
                });
            }
            Ok(Event::Incoming(Packet::SubAck(ack))) => {
                if ack
                    .return_codes
                    .iter()
                    .any(|code| matches!(code, SubscribeReasonCode::Failure))
                {
                    error!(
                        "the MQTT broker refused a command subscription; commands will not arrive"
                    );
                }
            }
            Ok(Event::Incoming(Packet::Publish(publish))) => {
                handle_command(&publish, &parser, &commands);
            }
            Ok(Event::Outgoing(Outgoing::Disconnect)) => {
                debug!("disconnected from MQTT broker {broker}");
                return;
            }
            Ok(_) => {}
            Err(e) => {
                connection.send_if_modified(|c| std::mem::replace(&mut c.connected, false));
                if connected_at
                    .take()
                    .is_some_and(|since| since.elapsed() >= STABLE_CONNECTION)
                {
                    failures = 0;
                    retry_delay = FIRST_RECONNECT;
                    last_error = None;
                }
                failures += 1;
                let message = e.to_string();
                if last_error.as_deref() != Some(message.as_str()) {
                    warn!("MQTT connection to {broker} failed: {message}; retrying");
                    last_error = Some(message);
                } else if failures % FAILURES_BETWEEN_REMINDERS == 0 {
                    warn!("MQTT broker {broker} still unreachable after {failures} attempts: {message}");
                } else {
                    debug!("MQTT connection to {broker} failed again: {message}");
                }
                tokio::time::sleep(retry_delay).await;
                retry_delay = (retry_delay * 2).min(LONGEST_RECONNECT);
            }
        }
    }
}

fn handle_command(publish: &Publish, parser: &CommandParser, commands: &Sender<RenderMessage>) {
    let topic = &publish.topic;
    if publish.retain {
        warn!(
            "ignoring retained command on {topic}; clear it with: mosquitto_pub -r -n -t '{topic}'"
        );
        return;
    }
    match parser.parse(topic, &publish.payload) {
        Ok(command) => {
            info!("{command}");
            if commands.send(RenderMessage::Command(command)).is_err() {
                debug!("render loop has stopped; command on {topic} dropped");
            }
        }
        Err(e) => warn!(
            "rejected command on {topic}: {e}; payload: {}",
            payload_for_log(&publish.payload)
        ),
    }
}

/// The payload as printable text, shortened for the log.
fn payload_for_log(payload: &[u8]) -> String {
    let text = String::from_utf8_lossy(payload);
    let mut shown: String = text
        .chars()
        .take(LOG_PAYLOAD_LIMIT)
        .flat_map(char::escape_debug)
        .collect();
    if text.chars().count() > LOG_PAYLOAD_LIMIT {
        shown.push_str("...");
    }
    format!("\"{shown}\"")
}

/// Publishes the retained status topics whenever they change or the
/// connection is re-established. Only ever uses `try_publish`, so a slow or
/// absent broker cannot hold anything up.
struct StatusPublisher {
    client: AsyncClient,
    topic: String,
    groups: GroupLevels,
    connection: watch::Receiver<Connection>,
    levels: watch::Receiver<[u8; CHANNEL_COUNT]>,
    output: watch::Receiver<OutputStatus>,
    shutdown: watch::Receiver<bool>,
}

#[derive(Default)]
struct Pending {
    online: bool,
    health: bool,
    groups: bool,
}

impl StatusPublisher {
    async fn run(mut self) {
        let mut pending = Pending::default();
        let mut announced_generation = 0;
        let mut published_groups = String::new();
        let mut last_groups_publish: Option<Instant> = None;
        let mut levels_open = true;
        let mut output_open = true;

        loop {
            let connection = *self.connection.borrow();
            if connection.connected && connection.generation != announced_generation {
                // A new connection: the broker may have lost everything retained.
                announced_generation = connection.generation;
                pending = Pending {
                    online: true,
                    health: true,
                    groups: true,
                };
                published_groups.clear();
                last_groups_publish = None;
            }

            let mut retry_at: Option<Instant> = None;
            if connection.connected {
                if pending.online && self.publish("online", "online".to_string()) {
                    pending.online = false;
                }
                if pending.health {
                    let health = health_json(&self.output.borrow());
                    if self.publish("health", health) {
                        pending.health = false;
                    }
                }
                if pending.groups {
                    let groups = self.groups.to_json(&self.levels.borrow());
                    let due = last_groups_publish.map(|at| at + GROUPS_INTERVAL);
                    if groups == published_groups {
                        pending.groups = false;
                    } else if due.is_some_and(|due| Instant::now() < due) {
                        retry_at = due;
                    } else if self.publish("groups", groups.clone()) {
                        published_groups = groups;
                        last_groups_publish = Some(Instant::now());
                        pending.groups = false;
                    }
                }
                if (pending.online || pending.health || pending.groups) && retry_at.is_none() {
                    retry_at = Some(Instant::now() + PUBLISH_RETRY);
                }
            }

            let wake = async {
                match retry_at {
                    Some(at) => tokio::time::sleep_until(at).await,
                    None => std::future::pending().await,
                }
            };
            let stopping = tokio::select! {
                changed = self.connection.changed() => changed.is_err(),
                changed = self.levels.changed(), if levels_open => {
                    match changed {
                        Ok(()) => pending.groups = true,
                        Err(_) => levels_open = false,
                    }
                    false
                }
                changed = self.output.changed(), if output_open => {
                    match changed {
                        Ok(()) => pending.health = true,
                        Err(_) => output_open = false,
                    }
                    false
                }
                _ = self.shutdown.wait_for(|stopping| *stopping) => true,
                _ = wake => false,
            };
            if stopping {
                self.say_goodbye();
                return;
            }
        }
    }

    /// Queues a retained status message. Returns false if the queue was full.
    fn publish(&self, subtopic: &str, payload: String) -> bool {
        let topic = format!("{}/{subtopic}", self.topic);
        match self
            .client
            .try_publish(topic.as_str(), QoS::AtLeastOnce, true, payload)
        {
            Ok(()) => true,
            Err(e) => {
                debug!("could not queue {topic}: {e}; will retry");
                false
            }
        }
    }

    /// Publishes `offline` and disconnects cleanly. If `offline` cannot be
    /// queued, the connection is left to drop when the process exits, so
    /// that the broker publishes the Last Will instead.
    fn say_goodbye(&self) {
        if self.connection.borrow().connected && !self.publish("online", "offline".to_string()) {
            return;
        }
        if let Err(e) = self.client.try_disconnect() {
            debug!("could not queue the MQTT disconnect: {e}");
        }
    }
}
