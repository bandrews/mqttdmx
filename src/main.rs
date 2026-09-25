// ABOUTME: mqttdmx command-line entry point.
// ABOUTME: Loads and validates the configuration, then runs the render loop and the MQTT link.

use std::collections::BTreeSet;
use std::fmt::Write as _;
use std::path::PathBuf;
use std::process::ExitCode;
use std::time::Duration;

use clap::{Parser, ValueEnum};
use rand::Rng;
use tokio::signal::unix::{signal, SignalKind};
use tokio::sync::watch;
use tracing::{error, info, warn};

use mqttdmx::command::CommandParser;
use mqttdmx::config::{self, Config, Driver, LogFormat, Overrides};
use mqttdmx::engine::Engine;
use mqttdmx::output::{self, devices};
use mqttdmx::render::{self, RenderMessage};
use mqttdmx::status::GroupLevels;
use mqttdmx::{logging, mqtt};

/// How long to wait at shutdown for the broker to take the offline notice.
const SHUTDOWN_GRACE: Duration = Duration::from_secs(2);
/// How long to wait at shutdown for blocking work such as a DNS lookup.
const RUNTIME_SHUTDOWN: Duration = Duration::from_millis(100);

#[derive(Parser)]
#[command(
    name = "mqttdmx",
    version,
    about = "MQTT-controlled DMX lighting controller with smooth fades",
    after_help = "Command-line values override the config file. See docs/configuration.md."
)]
struct Cli {
    /// JSON config file
    #[arg(short, long, value_name = "FILE")]
    config: Option<PathBuf>,

    /// MQTT broker host name or IP address [default: localhost]
    #[arg(short, long, value_name = "HOST")]
    server: Option<String>,

    /// MQTT broker port [default: 1883]
    #[arg(short, long)]
    port: Option<u16>,

    /// Base topic for commands and status [default: dmx]
    #[arg(short, long)]
    topic: Option<String>,

    /// MQTT user name (needs --mqtt-password)
    #[arg(long, value_name = "NAME")]
    mqtt_username: Option<String>,

    /// MQTT password (needs --mqtt-username)
    #[arg(long, value_name = "PASSWORD")]
    mqtt_password: Option<String>,

    /// Output driver
    #[arg(long, value_enum)]
    driver: Option<DriverArg>,

    /// Serial device of the DMX interface, or "auto" to find an Enttec DMX USB Pro
    #[arg(short, long, value_name = "PATH")]
    device: Option<String>,

    /// Level every channel starts at, 0-255 [default: 0]
    #[arg(long, value_name = "LEVEL")]
    startup_level: Option<u8>,

    /// Log format
    #[arg(long, value_enum, value_name = "FORMAT")]
    log_format: Option<LogFormatArg>,

    /// Log at debug level
    #[arg(short, long)]
    verbose: bool,

    /// Check the configuration, print a summary and exit
    #[arg(long)]
    check_config: bool,

    /// List serial devices, marking Enttec DMX USB Pro interfaces, and exit
    #[arg(long)]
    list_devices: bool,
}

#[derive(Clone, Copy, ValueEnum)]
enum DriverArg {
    #[value(name = "enttec-usb-dmx-pro")]
    EnttecUsbDmxPro,
    Null,
}

#[derive(Clone, Copy, ValueEnum)]
enum LogFormatArg {
    Text,
    Json,
}

impl Cli {
    fn overrides(&self) -> Overrides {
        Overrides {
            server: self.server.clone(),
            port: self.port,
            topic: self.topic.clone(),
            username: self.mqtt_username.clone(),
            password: self.mqtt_password.clone(),
            driver: self.driver.map(|driver| match driver {
                DriverArg::EnttecUsbDmxPro => Driver::EnttecUsbDmxPro,
                DriverArg::Null => Driver::Null,
            }),
            device: self.device.clone(),
            startup_level: self.startup_level,
            log_format: self.log_format.map(|format| match format {
                LogFormatArg::Text => LogFormat::Text,
                LogFormatArg::Json => LogFormat::Json,
            }),
            verbose: self.verbose,
        }
    }
}

fn main() -> ExitCode {
    let cli = Cli::parse();
    if cli.list_devices {
        print!("{}", devices::listing());
        return ExitCode::SUCCESS;
    }

    let mut config = match &cli.config {
        Some(path) => match config::load(path) {
            Ok(config) => config,
            Err(e) => {
                eprintln!("{e}");
                return ExitCode::FAILURE;
            }
        },
        None => Config::default(),
    };
    config.apply_overrides(&cli.overrides());
    if let Err(problems) = config.validate() {
        eprintln!("Configuration is invalid:");
        for problem in problems {
            eprintln!("  - {problem}");
        }
        return ExitCode::FAILURE;
    }
    if cli.check_config {
        print!("Configuration is valid.\n{}", summary(&config));
        return ExitCode::SUCCESS;
    }

    logging::init(&config.logging);
    match run(config) {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            error!("{e}");
            ExitCode::FAILURE
        }
    }
}

fn output_description(config: &Config) -> String {
    let output = &config.output;
    let driver = match (output.driver, output.device.as_deref()) {
        (Some(Driver::EnttecUsbDmxPro), Some(device)) => format!("enttec-usb-dmx-pro on {device}"),
        (Some(driver), _) => format!("{} driver", driver.name()),
        (None, _) => "no driver".to_string(),
    };
    format!(
        "{driver}, {} frames per second, startup level {}",
        output.frame_rate, output.startup_level
    )
}

fn summary(config: &Config) -> String {
    let channels: BTreeSet<u16> = config
        .groups
        .values()
        .flat_map(|group| group.channels.iter().copied())
        .collect();
    let mut text = String::new();
    let _ = writeln!(
        text,
        "  mqtt: {}:{}, topic {}",
        config.mqtt.server, config.mqtt.port, config.mqtt.topic
    );
    let _ = writeln!(text, "  output: {}", output_description(config));
    let _ = writeln!(
        text,
        "  {} groups using {} channels",
        config.groups.len(),
        channels.len()
    );
    for (name, group) in &config.groups {
        let members = if group.channels.is_empty() {
            "(no channels)".to_string()
        } else {
            group
                .channels
                .iter()
                .map(u16::to_string)
                .collect::<Vec<_>>()
                .join(", ")
        };
        match &group.description {
            Some(description) => {
                let _ = writeln!(text, "    {name}: {members} - {description}");
            }
            None => {
                let _ = writeln!(text, "    {name}: {members}");
            }
        }
    }
    text
}

fn run(config: Config) -> Result<(), String> {
    info!("mqttdmx {} starting", env!("CARGO_PKG_VERSION"));
    info!("output: {}", output_description(&config));
    info!(
        "{} groups; broker {}:{}, base topic {}",
        config.groups.len(),
        config.mqtt.server,
        config.mqtt.port,
        config.mqtt.topic
    );
    if config.output.driver == Some(Driver::Null) {
        warn!("the null output driver is selected: no DMX will be sent");
    }

    let output = output::open(&config.output)?;
    let engine = Engine::new(config.output.startup_level, rand::thread_rng().gen());
    let (commands, received) = std::sync::mpsc::channel();
    let (levels_tx, levels_rx) = watch::channel(engine.commanded_levels());
    let (output_tx, output_rx) = watch::channel(output.status());
    let render_thread = render::spawn(
        engine,
        output,
        config.output.frame_rate,
        received,
        levels_tx,
        output_tx,
    )
    .map_err(|e| format!("cannot start the render loop: {e}"))?;

    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .map_err(|e| format!("cannot start the async runtime: {e}"))?;
    let outcome: Result<(), String> = runtime.block_on(async {
        let mut terminate = signal(SignalKind::terminate())
            .map_err(|e| format!("cannot listen for SIGTERM: {e}"))?;
        let mut interrupt = signal(SignalKind::interrupt())
            .map_err(|e| format!("cannot listen for SIGINT: {e}"))?;
        let (shutdown_tx, shutdown_rx) = watch::channel(false);
        let link = mqtt::run(
            config.mqtt.clone(),
            CommandParser::new(&config),
            GroupLevels::new(&config),
            mqtt::Channels {
                commands: commands.clone(),
                levels: levels_rx,
                output: output_rx,
                shutdown: shutdown_rx,
            },
        );
        tokio::pin!(link);
        let mut watchdog = tokio::time::interval(Duration::from_secs(1));

        let outcome = loop {
            tokio::select! {
                _ = terminate.recv() => {
                    info!("received SIGTERM; shutting down");
                    break Ok(());
                }
                _ = interrupt.recv() => {
                    info!("received SIGINT; shutting down");
                    break Ok(());
                }
                () = &mut link => break Err("the MQTT link stopped unexpectedly".to_string()),
                _ = watchdog.tick() => {
                    if render_thread.is_finished() {
                        break Err("the render loop stopped unexpectedly".to_string());
                    }
                }
            }
        };

        let _ = commands.send(RenderMessage::Shutdown);
        let _ = shutdown_tx.send(true);
        if tokio::time::timeout(SHUTDOWN_GRACE, &mut link).await.is_err() {
            warn!("the MQTT broker did not take the offline notice in time; it will announce offline itself");
        }
        outcome
    });
    // A broker host name being resolved when shutdown starts runs on the
    // runtime's blocking pool; don't wait out the resolver's timeouts.
    runtime.shutdown_timeout(RUNTIME_SHUTDOWN);

    if render_thread.join().is_err() {
        return Err("the render loop failed".to_string());
    }
    outcome?;
    info!("mqttdmx stopped");
    Ok(())
}
