// ABOUTME: Shared test support: a fake Enttec DMX USB Pro on a pseudo-terminal.
// ABOUTME: It decodes the frames mqttdmx sends and can answer the widget parameters request.

#![allow(dead_code)]

use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

use serialport::{SerialPort, TTYPort};

/// Firmware version the fake widget reports: 1.44.
pub const FIRMWARE_LSB: u8 = 44;
pub const FIRMWARE_MSB: u8 = 1;

#[derive(Default)]
struct Received {
    frames: Vec<Vec<u8>>,
    other_labels: Vec<u8>,
}

/// A pseudo-terminal pair whose master side behaves like an Enttec DMX USB Pro.
/// mqttdmx opens the slave side through `link`, a symlink the test can repoint.
pub struct FakeWidget {
    received: Arc<Mutex<Received>>,
    stop: Arc<AtomicBool>,
    paused: Arc<AtomicBool>,
    thread: Option<JoinHandle<()>>,
    slave: Option<TTYPort>,
    pub slave_path: String,
}

impl FakeWidget {
    pub fn start(answers_parameters: bool) -> FakeWidget {
        let (mut master, slave) = TTYPort::pair().expect("pseudo-terminal pair");
        let slave_path = slave.name().expect("slave has a name");
        master
            .set_timeout(Duration::from_millis(20))
            .expect("master timeout");
        let received = Arc::new(Mutex::new(Received::default()));
        let stop = Arc::new(AtomicBool::new(false));
        let paused = Arc::new(AtomicBool::new(false));
        let thread = {
            let received = Arc::clone(&received);
            let stop = Arc::clone(&stop);
            let paused = Arc::clone(&paused);
            std::thread::spawn(move || serve(master, received, stop, paused, answers_parameters))
        };
        FakeWidget {
            received,
            stop,
            paused,
            thread: Some(thread),
            slave: Some(slave),
            slave_path,
        }
    }

    /// Every DMX frame received so far: the 512 channel bytes after the start code.
    pub fn frames(&self) -> Vec<Vec<u8>> {
        self.received.lock().expect("lock").frames.clone()
    }

    pub fn frame_count(&self) -> usize {
        self.received.lock().expect("lock").frames.len()
    }

    /// Labels of every message other than a DMX frame.
    pub fn other_labels(&self) -> Vec<u8> {
        self.received.lock().expect("lock").other_labels.clone()
    }

    /// Waits for a frame, received after the first `skip` frames, that satisfies `check`.
    pub fn wait_for_frame(
        &self,
        skip: usize,
        timeout: Duration,
        check: impl Fn(&[u8]) -> bool,
    ) -> Option<Vec<u8>> {
        let deadline = Instant::now() + timeout;
        while Instant::now() < deadline {
            if let Some(frame) = self.frames().into_iter().skip(skip).find(|f| check(f)) {
                return Some(frame);
            }
            std::thread::sleep(Duration::from_millis(5));
        }
        None
    }

    /// Simulates a hung interface that is still plugged in: it stops reading.
    pub fn stop_reading(&self) {
        self.paused.store(true, Ordering::SeqCst);
    }

    pub fn resume_reading(&self) {
        self.paused.store(false, Ordering::SeqCst);
    }

    /// Simulates unplugging the interface: both ends of the pseudo-terminal close.
    pub fn unplug(mut self) {
        self.shut_down();
    }

    fn shut_down(&mut self) {
        self.stop.store(true, Ordering::SeqCst);
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
        self.slave.take();
    }
}

impl Drop for FakeWidget {
    fn drop(&mut self) {
        self.shut_down();
    }
}

fn serve(
    mut master: TTYPort,
    received: Arc<Mutex<Received>>,
    stop: Arc<AtomicBool>,
    paused: Arc<AtomicBool>,
    answers: bool,
) {
    let mut buffer: Vec<u8> = Vec::new();
    let mut chunk = [0u8; 4096];
    while !stop.load(Ordering::SeqCst) {
        if paused.load(Ordering::SeqCst) {
            std::thread::sleep(Duration::from_millis(5));
            continue;
        }
        match master.read(&mut chunk) {
            Ok(0) => {}
            Ok(n) => buffer.extend_from_slice(&chunk[..n]),
            Err(e) if e.kind() == std::io::ErrorKind::TimedOut => {}
            Err(_) => std::thread::sleep(Duration::from_millis(5)),
        }
        while let Some((label, data, used)) = next_message(&buffer) {
            buffer.drain(..used);
            if label == 6 {
                if let Some((_, channels)) = data.split_first() {
                    received
                        .lock()
                        .expect("lock")
                        .frames
                        .push(channels.to_vec());
                }
            } else {
                received.lock().expect("lock").other_labels.push(label);
                if label == 3 && answers {
                    let reply = [0x7E, 3, 5, 0, FIRMWARE_LSB, FIRMWARE_MSB, 9, 1, 40, 0xE7];
                    let _ = master.write_all(&reply);
                }
            }
        }
    }
}

/// Finds the first complete widget message: (label, data, bytes consumed).
fn next_message(buffer: &[u8]) -> Option<(u8, Vec<u8>, usize)> {
    let start = buffer.iter().position(|&b| b == 0x7E)?;
    let header = buffer.get(start..start + 4)?;
    let length = usize::from(header[2]) | (usize::from(header[3]) << 8);
    let end = start + 4 + length;
    if *buffer.get(end)? != 0xE7 {
        // Not a real message start; skip this byte.
        return Some((0, Vec::new(), start + 1));
    }
    Some((header[1], buffer[start + 4..end].to_vec(), end + 1))
}

/// A symlink that stands in for a stable device name such as /dev/serial/by-id/...
pub struct DeviceLink {
    _dir: tempfile::TempDir,
    pub path: PathBuf,
}

impl DeviceLink {
    pub fn to(target: &str) -> DeviceLink {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("dmx-interface");
        std::os::unix::fs::symlink(target, &path).expect("symlink");
        DeviceLink { _dir: dir, path }
    }

    pub fn repoint(&self, target: &str) {
        std::fs::remove_file(&self.path).expect("remove link");
        std::os::unix::fs::symlink(target, &self.path).expect("symlink");
    }

    pub fn path_str(&self) -> &str {
        self.path.to_str().expect("utf-8 path")
    }

    pub fn exists(&self) -> bool {
        Path::new(&self.path).exists()
    }
}

/// Polls `check` until it returns true or `timeout` passes.
pub fn eventually(timeout: Duration, mut check: impl FnMut() -> bool) -> bool {
    let deadline = Instant::now() + timeout;
    while Instant::now() < deadline {
        if check() {
            return true;
        }
        std::thread::sleep(Duration::from_millis(10));
    }
    check()
}
