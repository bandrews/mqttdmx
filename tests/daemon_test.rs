// ABOUTME: End-to-end tests of the mqttdmx binary against a real mosquitto broker and a fake Enttec Pro.
// ABOUTME: Broker tests run when MQTTDMX_BROKER_TESTS=1 and need mosquitto and its command-line clients.

mod support;

use std::fs;
use std::io::Write;
use std::net::TcpListener;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Output, Stdio};
use std::time::{Duration, Instant};

use serde_json::Value;
use support::{eventually, DeviceLink, FakeWidget};

const BINARY: &str = env!("CARGO_BIN_EXE_mqttdmx");
const PATIENCE: Duration = Duration::from_secs(15);

fn broker_tests_enabled() -> bool {
    if std::env::var("MQTTDMX_BROKER_TESTS").as_deref() == Ok("1") {
        return true;
    }
    eprintln!("skipped: set MQTTDMX_BROKER_TESTS=1 to run broker tests (needs mosquitto)");
    false
}

fn free_port() -> u16 {
    TcpListener::bind("127.0.0.1:0")
        .expect("bind")
        .local_addr()
        .expect("addr")
        .port()
}

struct Broker {
    port: u16,
    process: Child,
}

impl Broker {
    fn start() -> Broker {
        Broker::start_on(free_port())
    }

    fn start_on(port: u16) -> Broker {
        let process = Command::new("mosquitto")
            .args(["-p", &port.to_string()])
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .expect("mosquitto is installed");
        let broker = Broker { port, process };
        assert!(
            eventually(Duration::from_secs(5), || broker
                .publish("probe", "1", false)),
            "mosquitto did not start on port {port}"
        );
        broker
    }

    fn stop(&mut self) {
        let _ = self.process.kill();
        let _ = self.process.wait();
    }

    fn publish(&self, topic: &str, payload: &str, retain: bool) -> bool {
        let port = self.port.to_string();
        let mut args = vec![
            "-h",
            "127.0.0.1",
            "-p",
            &port,
            "-t",
            topic,
            "-m",
            payload,
            "-q",
            "1",
        ];
        if retain {
            args.push("-r");
        }
        Command::new("mosquitto_pub")
            .args(&args)
            .stderr(Stdio::null())
            .status()
            .is_ok_and(|s| s.success())
    }

    /// The retained message on `topic`, if any.
    fn retained(&self, topic: &str) -> Option<String> {
        let output = Command::new("mosquitto_sub")
            .args(["-h", "127.0.0.1", "-p", &self.port.to_string(), "-t", topic])
            .args(["--retained-only", "-C", "1", "-W", "1"])
            .output()
            .ok()?;
        let text = String::from_utf8_lossy(&output.stdout).trim().to_string();
        (!text.is_empty()).then_some(text)
    }

    fn wait_for_retained(&self, topic: &str, check: impl Fn(&str) -> bool) -> Option<String> {
        let deadline = Instant::now() + PATIENCE;
        while Instant::now() < deadline {
            if let Some(message) = self.retained(topic) {
                if check(&message) {
                    return Some(message);
                }
            }
            std::thread::sleep(Duration::from_millis(100));
        }
        None
    }
}

impl Drop for Broker {
    fn drop(&mut self) {
        self.stop();
    }
}

const GROUPS: &str = r#"{
  "groups": {
    "House": {"channels": [1, 2, 3]},
    "Spot": {"channels": [10]},
    "Unused": {"channels": []}
  },
  "ui": {"levels": {"off": 0}}
}"#;

struct Daemon {
    process: Child,
    log: PathBuf,
    _dir: tempfile::TempDir,
}

impl Daemon {
    fn start(broker_port: u16, device: &str) -> Daemon {
        let dir = tempfile::tempdir().expect("tempdir");
        let config = dir.path().join("lights.json");
        fs::write(&config, GROUPS).expect("write config");
        let log = dir.path().join("mqttdmx.log");
        let log_file = fs::File::create(&log).expect("log file");
        let process = Command::new(BINARY)
            .args(["--config", config.to_str().expect("utf-8")])
            .args(["--server", "127.0.0.1", "--port", &broker_port.to_string()])
            .args(["--driver", "enttec-usb-dmx-pro", "--device", device])
            .args(["--startup-level", "255"])
            .env_remove("RUST_LOG")
            .stdout(log_file.try_clone().expect("clone"))
            .stderr(log_file)
            .spawn()
            .expect("mqttdmx starts");
        Daemon {
            process,
            log,
            _dir: dir,
        }
    }

    fn log(&self) -> String {
        fs::read_to_string(&self.log).unwrap_or_default()
    }

    fn wait_for_log(&self, needle: &str) -> bool {
        eventually(PATIENCE, || self.log().contains(needle))
    }

    fn signal(&self, signal: &str) {
        let status = Command::new("kill")
            .args([&format!("-{signal}"), &self.process.id().to_string()])
            .status()
            .expect("kill runs");
        assert!(status.success());
    }

    fn wait_for_exit(&mut self, timeout: Duration) -> Option<std::process::ExitStatus> {
        let deadline = Instant::now() + timeout;
        while Instant::now() < deadline {
            if let Ok(Some(status)) = self.process.try_wait() {
                return Some(status);
            }
            std::thread::sleep(Duration::from_millis(20));
        }
        None
    }
}

impl Drop for Daemon {
    fn drop(&mut self) {
        let _ = self.process.kill();
        let _ = self.process.wait();
    }
}

fn json(text: &str) -> Value {
    serde_json::from_str(text).unwrap_or_else(|e| panic!("not JSON ({e}): {text}"))
}

/// A running broker, widget and daemon, with the daemon online and connected.
fn running_system() -> (Broker, FakeWidget, DeviceLink, Daemon) {
    let broker = Broker::start();
    let widget = FakeWidget::start(true);
    let link = DeviceLink::to(&widget.slave_path);
    let daemon = Daemon::start(broker.port, link.path_str());
    assert!(
        broker
            .wait_for_retained("dmx/status/online", |m| m == "online")
            .is_some(),
        "never came online; log:\n{}",
        daemon.log()
    );
    (broker, widget, link, daemon)
}

#[test]
#[cfg_attr(
    not(target_os = "linux"),
    ignore = "needs Linux pseudo-terminals; run Lane A"
)]
fn status_is_published_and_retained() {
    if !broker_tests_enabled() {
        return;
    }
    let (broker, _widget, link, daemon) = running_system();

    let health = broker
        .wait_for_retained("dmx/status/health", |m| m.contains("connected"))
        .unwrap_or_else(|| panic!("no health; log:\n{}", daemon.log()));
    let health = json(&health);
    assert_eq!(health["version"], env!("CARGO_PKG_VERSION"));
    assert_eq!(health["output"]["driver"], "enttec-usb-dmx-pro");
    assert_eq!(health["output"]["state"], "connected");
    assert_eq!(health["output"]["device"], link.path_str());
    assert_eq!(health["output"]["firmware"], "1.44");
    assert_eq!(health["output"]["error"], Value::Null);

    let groups = broker
        .wait_for_retained("dmx/status/groups", |_| true)
        .expect("groups published");
    assert_eq!(
        json(&groups),
        json(r#"{"House":255,"Spot":255,"Unused":null}"#)
    );
}

#[test]
#[cfg_attr(
    not(target_os = "linux"),
    ignore = "needs Linux pseudo-terminals; run Lane A"
)]
fn commands_reach_the_interface_and_update_the_group_levels() {
    if !broker_tests_enabled() {
        return;
    }
    let (broker, widget, _link, daemon) = running_system();
    assert!(widget
        .wait_for_frame(0, PATIENCE, |f| f.iter().all(|&v| v == 255))
        .is_some());

    let before = widget.frame_count();
    assert!(broker.publish("dmx/group/House", r#"{"value":0,"fade":400}"#, false));
    let dark = widget.wait_for_frame(before, PATIENCE, |f| f[..3] == [0, 0, 0]);
    assert!(
        dark.is_some(),
        "House never reached 0; log:\n{}",
        daemon.log()
    );
    let during: Vec<u8> = widget.frames()[before..].iter().map(|f| f[0]).collect();
    assert!(
        during.iter().any(|&v| v > 0 && v < 255),
        "the fade showed in-between levels: {during:?}"
    );

    assert!(broker
        .wait_for_retained("dmx/status/groups", |m| json(m)["House"] == 0)
        .is_some());

    assert!(broker.publish("dmx/channel/10", "7", false));
    assert!(widget.wait_for_frame(0, PATIENCE, |f| f[9] == 7).is_some());
    assert!(broker.publish("dmx", r#"{"range":{"start":1,"end":512},"value":0}"#, false));
    assert!(widget
        .wait_for_frame(0, PATIENCE, |f| f.iter().all(|&v| v == 0))
        .is_some());
    let groups = broker
        .wait_for_retained("dmx/status/groups", |m| json(m)["Spot"] == 0)
        .expect("groups updated");
    assert_eq!(json(&groups), json(r#"{"House":0,"Spot":0,"Unused":null}"#));
    assert!(daemon
        .log()
        .contains("group House: fade to 0 over 400 ms (linear)"));
}

#[test]
#[cfg_attr(
    not(target_os = "linux"),
    ignore = "needs Linux pseudo-terminals; run Lane A"
)]
fn malformed_commands_are_logged_and_change_nothing() {
    if !broker_tests_enabled() {
        return;
    }
    let (broker, widget, _link, daemon) = running_system();
    assert!(broker.publish("dmx/group/House", "null", false));
    assert!(broker.publish("dmx/group/House", r#"{"value":300}"#, false));
    assert!(broker.publish("dmx/group/Nowhere", "1", false));
    assert!(daemon.wait_for_log("value must be a whole number from 0 to 255 (got 300)"));
    assert!(daemon.wait_for_log("unknown group \"Nowhere\""));
    assert!(daemon
        .log()
        .contains("payload must be a level from 0 to 255 or a JSON object (got null)"));
    let frames = widget.frames();
    assert!(
        frames.iter().all(|f| f.iter().all(|&v| v == 255)),
        "nothing changed"
    );
    assert!(broker.publish("dmx/channel/1", "5", false));
    assert!(
        widget.wait_for_frame(0, PATIENCE, |f| f[0] == 5).is_some(),
        "still accepting commands"
    );
}

#[test]
#[cfg_attr(
    not(target_os = "linux"),
    ignore = "needs Linux pseudo-terminals; run Lane A"
)]
fn retained_commands_are_ignored() {
    if !broker_tests_enabled() {
        return;
    }
    let broker = Broker::start();
    assert!(broker.publish("dmx/group/House", r#"{"value":0}"#, true));
    let widget = FakeWidget::start(true);
    let daemon = Daemon::start(broker.port, &widget.slave_path);
    assert!(daemon.wait_for_log("ignoring retained command on dmx/group/House"));
    std::thread::sleep(Duration::from_millis(300));
    assert!(
        widget.frames().iter().all(|f| f[0] == 255),
        "the retained command was applied"
    );
}

#[test]
#[cfg_attr(
    not(target_os = "linux"),
    ignore = "needs Linux pseudo-terminals; run Lane A"
)]
fn it_reconnects_after_the_broker_restarts() {
    if !broker_tests_enabled() {
        return;
    }
    let (mut broker, widget, _link, daemon) = running_system();
    let port = broker.port;
    broker.stop();
    assert!(daemon.wait_for_log("MQTT connection"), "the loss is logged");
    let broker = Broker::start_on(port);
    assert!(
        broker
            .wait_for_retained("dmx/status/online", |m| m == "online")
            .is_some(),
        "status republished after reconnecting; log:\n{}",
        daemon.log()
    );
    assert!(broker
        .wait_for_retained("dmx/status/groups", |_| true)
        .is_some());
    assert!(broker.publish("dmx/channel/2", "42", false));
    assert!(widget.wait_for_frame(0, PATIENCE, |f| f[1] == 42).is_some());
}

#[test]
#[cfg_attr(
    not(target_os = "linux"),
    ignore = "needs Linux pseudo-terminals; run Lane A"
)]
fn sigterm_publishes_offline_and_holds_the_look() {
    if !broker_tests_enabled() {
        return;
    }
    let (broker, widget, _link, mut daemon) = running_system();
    assert!(broker.publish("dmx/channel/5", "33", false));
    assert!(widget.wait_for_frame(0, PATIENCE, |f| f[4] == 33).is_some());
    daemon.signal("TERM");
    let status = daemon
        .wait_for_exit(Duration::from_secs(5))
        .expect("exits promptly");
    assert!(
        status.success(),
        "clean exit: {status:?}; log:\n{}",
        daemon.log()
    );
    assert_eq!(
        broker.retained("dmx/status/online").as_deref(),
        Some("offline")
    );
    let last = widget.frames().pop().expect("frames");
    assert_eq!(last[4], 33, "the last frame keeps the look");
    assert_eq!(last[0], 255);
}

#[test]
#[cfg_attr(
    not(target_os = "linux"),
    ignore = "needs Linux pseudo-terminals; run Lane A"
)]
fn the_broker_announces_offline_when_the_daemon_dies() {
    if !broker_tests_enabled() {
        return;
    }
    let (broker, _widget, _link, mut daemon) = running_system();
    daemon.signal("KILL");
    daemon
        .wait_for_exit(Duration::from_secs(5))
        .expect("killed");
    assert!(broker
        .wait_for_retained("dmx/status/online", |m| m == "offline")
        .is_some());
}

#[test]
#[cfg_attr(
    not(target_os = "linux"),
    ignore = "needs Linux pseudo-terminals; run Lane A"
)]
fn a_missing_interface_is_reported_and_picked_up_when_plugged_in() {
    if !broker_tests_enabled() {
        return;
    }
    let broker = Broker::start();
    let link = DeviceLink::to("/nonexistent/tty");
    let daemon = Daemon::start(broker.port, link.path_str());
    let health = broker
        .wait_for_retained("dmx/status/health", |m| m.contains("disconnected"))
        .unwrap_or_else(|| panic!("no disconnected health; log:\n{}", daemon.log()));
    assert!(json(&health)["output"]["error"]
        .as_str()
        .is_some_and(|e| e.contains("/nonexistent/tty") || e.contains(link.path_str())));

    let widget = FakeWidget::start(true);
    link.repoint(&widget.slave_path);
    assert!(broker
        .wait_for_retained("dmx/status/health", |m| m.contains("\"connected\""))
        .is_some());
    assert!(widget
        .wait_for_frame(0, PATIENCE, |f| f[0] == 255)
        .is_some());
}

fn run(args: &[&str]) -> Output {
    Command::new(BINARY)
        .args(args)
        .env_remove("RUST_LOG")
        .output()
        .expect("mqttdmx runs")
}

fn write_config(dir: &Path, text: &str) -> PathBuf {
    let path = dir.join("config.json");
    let mut file = fs::File::create(&path).expect("create");
    file.write_all(text.as_bytes()).expect("write");
    path
}

#[test]
fn an_invalid_config_is_rejected_with_every_problem() {
    let dir = tempfile::tempdir().expect("tempdir");
    let path = write_config(
        dir.path(),
        r#"{"output": {"driver": "enttec-usb-dmx-pro", "frame_rate": 99},
            "groups": {"Bad/Name": {"channels": [0]}}}"#,
    );
    let output = run(&["--config", path.to_str().expect("utf-8")]);
    assert_eq!(output.status.code(), Some(1));
    let stderr = String::from_utf8_lossy(&output.stderr);
    for message in [
        "Configuration is invalid:",
        "output.device is required for the enttec-usb-dmx-pro driver",
        "output.frame_rate must be between 1 and 44 (got 99)",
        "groups.Bad/Name: a group name must not contain /, + or #",
        "groups.Bad/Name: channel 0 is outside 1-512",
    ] {
        assert!(
            stderr.contains(message),
            "missing {message:?} in:\n{stderr}"
        );
    }
}

#[test]
fn an_unreadable_config_is_rejected() {
    let output = run(&["--config", "/nonexistent/mqttdmx.json", "--driver", "null"]);
    assert_eq!(output.status.code(), Some(1));
    assert!(String::from_utf8_lossy(&output.stderr).contains("/nonexistent/mqttdmx.json"));

    let dir = tempfile::tempdir().expect("tempdir");
    let path = write_config(dir.path(), r#"{"output": {"startup_levle": 1}}"#);
    let output = run(&[
        "--config",
        path.to_str().expect("utf-8"),
        "--driver",
        "null",
    ]);
    assert_eq!(output.status.code(), Some(1));
    assert!(String::from_utf8_lossy(&output.stderr).contains("unknown field `startup_levle`"));
}

#[test]
fn check_config_summarises_a_valid_config() {
    let dir = tempfile::tempdir().expect("tempdir");
    let path = write_config(dir.path(), GROUPS);
    let output = run(&[
        "--config",
        path.to_str().expect("utf-8"),
        "--driver",
        "null",
        "--check-config",
    ]);
    assert_eq!(
        output.status.code(),
        Some(0),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("Configuration is valid."), "{stdout}");
    assert!(stdout.contains("3 groups using 4 channels"), "{stdout}");
    assert!(stdout.contains("output: null driver"), "{stdout}");
    assert!(
        stdout.contains("mqtt: localhost:1883, topic dmx"),
        "{stdout}"
    );
}

#[test]
fn list_devices_runs_without_a_config() {
    let output = run(&["--list-devices"]);
    assert_eq!(output.status.code(), Some(0));
    assert!(String::from_utf8_lossy(&output.stdout).contains("output.device"));
}

#[test]
fn version_is_reported() {
    let output = run(&["--version"]);
    assert_eq!(output.status.code(), Some(0));
    assert!(String::from_utf8_lossy(&output.stdout).contains(env!("CARGO_PKG_VERSION")));
}

#[test]
#[cfg_attr(
    not(target_os = "linux"),
    ignore = "needs Linux pseudo-terminals; run Lane A"
)]
fn a_connection_that_keeps_failing_straight_away_backs_off_and_logs_once() {
    if !broker_tests_enabled() {
        return;
    }
    let broker = Broker::start();
    // A retained message larger than mqttdmx accepts on a command topic: the
    // broker sends it on every subscription, and every connection fails.
    let dir = tempfile::tempdir().expect("tempdir");
    let oversized = dir.path().join("oversized");
    fs::write(&oversized, vec![b'x'; 1_100_000]).expect("write");
    let status = Command::new("mosquitto_pub")
        .args(["-h", "127.0.0.1", "-p", &broker.port.to_string()])
        .args(["-t", "dmx/group/House", "-r", "-q", "1", "-f"])
        .arg(&oversized)
        .status()
        .expect("mosquitto_pub runs");
    assert!(status.success());

    let widget = FakeWidget::start(true);
    let daemon = Daemon::start(broker.port, &widget.slave_path);
    std::thread::sleep(Duration::from_secs(6));
    let log = daemon.log();
    let connections = log.matches("connected to MQTT broker").count();
    let warnings = log
        .lines()
        .filter(|line| line.contains("WARN") && line.contains("MQTT connection to"))
        .count();
    assert!(
        (1..=5).contains(&connections),
        "backoff should allow about 4 connections in 6 s, saw {connections}; log:\n{log}"
    );
    assert_eq!(
        warnings, 1,
        "the repeated failure is logged once; log:\n{log}"
    );
    assert!(log.contains("payload size limit exceeded"), "log:\n{log}");
}

#[test]
#[cfg_attr(
    not(target_os = "linux"),
    ignore = "needs Linux pseudo-terminals; run Lane A"
)]
fn a_retained_command_published_while_connected_is_applied() {
    if !broker_tests_enabled() {
        return;
    }
    let (broker, widget, _link, daemon) = running_system();
    // The broker forwards a retained publish to connected subscribers without
    // the retain flag, so it is applied; only replays on (re)subscribe are ignored.
    assert!(broker.publish("dmx/channel/3", "9", true));
    assert!(widget.wait_for_frame(0, PATIENCE, |f| f[2] == 9).is_some());
    assert!(!daemon.log().contains("ignoring retained command"));
}
