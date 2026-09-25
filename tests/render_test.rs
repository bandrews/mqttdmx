// ABOUTME: Tests for the render loop thread: frame pacing, command timing, status and shutdown.
// ABOUTME: Uses a recording output so every frame the loop sends can be inspected.

mod support;

use std::sync::mpsc;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use mqttdmx::command::{Action, Command, Target};
use mqttdmx::config::{Easing, CHANNEL_COUNT};
use mqttdmx::engine::Engine;
use mqttdmx::output::Output;
use mqttdmx::render::{self, RenderMessage};
use mqttdmx::status::{OutputState, OutputStatus};
use support::eventually;
use tokio::sync::watch;

/// A frame and the moment the render loop handed it over.
type TimedFrame = (Instant, [u8; CHANNEL_COUNT]);

#[derive(Clone)]
struct Recorder {
    frames: Arc<Mutex<Vec<TimedFrame>>>,
    status: Arc<Mutex<OutputStatus>>,
}

impl Recorder {
    fn new() -> Recorder {
        Recorder {
            frames: Arc::new(Mutex::new(Vec::new())),
            status: Arc::new(Mutex::new(OutputStatus {
                driver: "recorder",
                device: None,
                state: OutputState::Connected,
                firmware: None,
                error: None,
            })),
        }
    }

    fn frames(&self) -> Vec<[u8; CHANNEL_COUNT]> {
        self.frames
            .lock()
            .expect("lock")
            .iter()
            .map(|(_, f)| *f)
            .collect()
    }

    fn frame_times(&self) -> Vec<Instant> {
        self.frames
            .lock()
            .expect("lock")
            .iter()
            .map(|(t, _)| *t)
            .collect()
    }
}

impl Output for Recorder {
    fn send(&mut self, frame: &[u8; CHANNEL_COUNT]) {
        self.frames
            .lock()
            .expect("lock")
            .push((Instant::now(), *frame));
    }

    fn status(&self) -> OutputStatus {
        self.status.lock().expect("lock").clone()
    }
}

struct Running {
    recorder: Recorder,
    messages: mpsc::Sender<RenderMessage>,
    levels: watch::Receiver<[u8; CHANNEL_COUNT]>,
    output_status: watch::Receiver<OutputStatus>,
    thread: std::thread::JoinHandle<()>,
}

fn start(frame_rate: u32, startup_level: u8) -> Running {
    let recorder = Recorder::new();
    let (messages, receiver) = mpsc::channel();
    let (levels_tx, levels) = watch::channel([0u8; CHANNEL_COUNT]);
    let (status_tx, output_status) = watch::channel(recorder.status());
    let thread = render::spawn(
        Engine::new(startup_level, 1),
        Box::new(recorder.clone()),
        frame_rate,
        receiver,
        levels_tx,
        status_tx,
    )
    .expect("render thread starts");
    Running {
        recorder,
        messages,
        levels,
        output_status,
        thread,
    }
}

fn set_channel(channel: usize, level: u8, fade_ms: f64) -> RenderMessage {
    RenderMessage::Command(Command {
        target: Target::Channel(channel as u16),
        channels: vec![channel - 1],
        action: Action::Set {
            level,
            fade_ms,
            easing: Easing::Linear,
        },
    })
}

#[test]
fn frames_are_sent_at_the_frame_rate() {
    let running = start(40, 0);
    std::thread::sleep(Duration::from_millis(1000));
    let times = running.recorder.frame_times();
    assert!(
        (36..=44).contains(&times.len()),
        "{} frames in a second",
        times.len()
    );
    let gaps: Vec<Duration> = times.windows(2).map(|w| w[1] - w[0]).collect();
    let late = gaps
        .iter()
        .filter(|gap| **gap > Duration::from_millis(40))
        .count();
    assert!(late <= 2, "{late} gaps over 40 ms: {gaps:?}");
    running
        .messages
        .send(RenderMessage::Shutdown)
        .expect("send");
    running.thread.join().expect("render thread exits cleanly");
}

#[test]
fn the_first_frame_carries_the_startup_level() {
    let running = start(40, 255);
    assert!(eventually(Duration::from_secs(2), || !running
        .recorder
        .frames()
        .is_empty()));
    assert!(running.recorder.frames()[0].iter().all(|&v| v == 255));
    assert_eq!(*running.levels.borrow(), [255u8; CHANNEL_COUNT]);
    running
        .messages
        .send(RenderMessage::Shutdown)
        .expect("send");
    running.thread.join().expect("exits");
}

#[test]
fn a_command_shows_up_within_a_frame_or_two() {
    let running = start(40, 0);
    assert!(eventually(Duration::from_secs(2), || !running
        .recorder
        .frames()
        .is_empty()));
    let sent = Instant::now();
    running
        .messages
        .send(set_channel(7, 200, 0.0))
        .expect("send");
    assert!(eventually(Duration::from_secs(2), || {
        running
            .recorder
            .frames()
            .last()
            .is_some_and(|f| f[6] == 200)
    }));
    let shown = running
        .recorder
        .frames
        .lock()
        .expect("lock")
        .iter()
        .find(|(_, f)| f[6] == 200)
        .map(|(t, _)| *t)
        .expect("frame with the new level");
    assert!(
        shown.duration_since(sent) <= Duration::from_millis(60),
        "took {:?}",
        shown.duration_since(sent)
    );
    assert_eq!(running.levels.borrow()[6], 200);
    running
        .messages
        .send(RenderMessage::Shutdown)
        .expect("send");
    running.thread.join().expect("exits");
}

#[test]
fn a_fade_climbs_steadily_frame_by_frame() {
    let running = start(40, 0);
    running
        .messages
        .send(set_channel(1, 255, 1000.0))
        .expect("send");
    assert!(eventually(Duration::from_secs(3), || {
        running
            .recorder
            .frames()
            .last()
            .is_some_and(|f| f[0] == 255)
    }));
    let levels: Vec<u8> = running.recorder.frames().iter().map(|f| f[0]).collect();
    assert!(
        levels.windows(2).all(|w| w[1] >= w[0]),
        "never steps back: {levels:?}"
    );
    let biggest_step = levels.windows(2).map(|w| w[1] - w[0]).max().unwrap_or(0);
    assert!(
        biggest_step <= 20,
        "a 1 s fade at 40 fps moves about 6.4 per frame; saw {biggest_step}: {levels:?}"
    );
    running
        .messages
        .send(RenderMessage::Shutdown)
        .expect("send");
    running.thread.join().expect("exits");
}

#[test]
fn output_status_changes_are_reported() {
    let mut running = start(40, 0);
    running.output_status.mark_unchanged();
    {
        let mut status = running.recorder.status.lock().expect("lock");
        status.state = OutputState::Disconnected;
        status.error = Some("unplugged".to_string());
    }
    assert!(eventually(Duration::from_secs(2), || {
        running.output_status.has_changed().unwrap_or(false)
    }));
    let reported = running.output_status.borrow_and_update().clone();
    assert_eq!(reported.state, OutputState::Disconnected);
    assert_eq!(reported.error.as_deref(), Some("unplugged"));
    running
        .messages
        .send(RenderMessage::Shutdown)
        .expect("send");
    running.thread.join().expect("exits");
}

#[test]
fn shutdown_sends_one_last_frame_with_everything_applied() {
    let running = start(1, 0);
    assert!(eventually(Duration::from_secs(3), || !running
        .recorder
        .frames()
        .is_empty()));
    running
        .messages
        .send(set_channel(3, 99, 0.0))
        .expect("send");
    running
        .messages
        .send(RenderMessage::Shutdown)
        .expect("send");
    running.thread.join().expect("exits");
    let last = *running.recorder.frames().last().expect("frames");
    assert_eq!(
        last[2], 99,
        "commands queued before shutdown reach the final frame"
    );
}

#[test]
fn the_loop_stops_when_every_sender_is_gone() {
    let running = start(40, 0);
    drop(running.messages);
    let deadline = Instant::now() + Duration::from_secs(2);
    while !running.thread.is_finished() && Instant::now() < deadline {
        std::thread::sleep(Duration::from_millis(10));
    }
    assert!(running.thread.is_finished());
}

/// An output that takes longer than a frame period to send each frame.
struct SlowOutput {
    inner: Recorder,
    delay: Duration,
}

impl Output for SlowOutput {
    fn send(&mut self, frame: &[u8; CHANNEL_COUNT]) {
        std::thread::sleep(self.delay);
        self.inner.send(frame);
    }

    fn status(&self) -> OutputStatus {
        self.inner.status()
    }
}

#[test]
fn commands_and_shutdown_still_arrive_when_every_send_runs_late() {
    let recorder = Recorder::new();
    let (messages, receiver) = mpsc::channel();
    let (levels_tx, _levels) = watch::channel([0u8; CHANNEL_COUNT]);
    let (status_tx, _status) = watch::channel(recorder.status());
    let slow = SlowOutput {
        inner: recorder.clone(),
        delay: Duration::from_millis(40),
    };
    let thread = render::spawn(
        Engine::new(0, 1),
        Box::new(slow),
        40,
        receiver,
        levels_tx,
        status_tx,
    )
    .expect("render thread starts");
    assert!(eventually(Duration::from_secs(2), || !recorder
        .frames()
        .is_empty()));

    messages.send(set_channel(4, 123, 0.0)).expect("send");
    assert!(
        eventually(Duration::from_secs(2), || recorder
            .frames()
            .last()
            .is_some_and(|f| f[3] == 123)),
        "a command must reach a frame even when the output is slower than the frame rate"
    );

    messages.send(RenderMessage::Shutdown).expect("send");
    assert!(
        eventually(Duration::from_secs(2), || thread.is_finished()),
        "shutdown must stop the loop even when the output is slower than the frame rate"
    );
    thread.join().expect("exits");
}
