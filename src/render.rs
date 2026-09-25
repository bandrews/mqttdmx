// ABOUTME: The render loop: a dedicated thread that owns the engine and the output device.
// ABOUTME: Applies commands as they arrive and sends one frame per tick at the configured rate.

use std::sync::mpsc::{Receiver, RecvTimeoutError, TryRecvError};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

use tokio::sync::watch;
use tracing::debug;

use crate::command::Command;
use crate::config::CHANNEL_COUNT;
use crate::engine::Engine;
use crate::output::Output;
use crate::status::OutputStatus;

pub enum RenderMessage {
    Command(Command),
    /// Send one last frame with everything received so far, then stop.
    Shutdown,
}

/// Starts the render loop on its own thread. It runs until it receives
/// `Shutdown` or every sender is dropped. Commanded levels and output status
/// are published on the watch channels whenever they change; the loop never
/// waits on their receivers.
pub fn spawn(
    engine: Engine,
    output: Box<dyn Output>,
    frame_rate: u32,
    messages: Receiver<RenderMessage>,
    levels: watch::Sender<[u8; CHANNEL_COUNT]>,
    output_status: watch::Sender<OutputStatus>,
) -> std::io::Result<JoinHandle<()>> {
    let render_loop = RenderLoop {
        engine,
        output,
        period: Duration::from_secs(1) / frame_rate.max(1),
        messages,
        levels,
        output_status,
    };
    std::thread::Builder::new()
        .name("render".to_string())
        .spawn(move || render_loop.run())
}

struct RenderLoop {
    engine: Engine,
    output: Box<dyn Output>,
    period: Duration,
    messages: Receiver<RenderMessage>,
    levels: watch::Sender<[u8; CHANNEL_COUNT]>,
    output_status: watch::Sender<OutputStatus>,
}

impl RenderLoop {
    fn run(mut self) {
        let epoch = Instant::now();
        let elapsed_ms = |at: Instant| at.duration_since(epoch).as_secs_f64() * 1000.0;
        let mut frame = [0u8; CHANNEL_COUNT];
        let mut next_frame = epoch;
        let mut stopping = false;
        self.publish_levels();

        loop {
            // Until the next frame is due, apply commands the moment they arrive.
            // When the frame is already due, still take everything queued, so a
            // slow output can delay commands by a frame but never starve them.
            while !stopping {
                let wait = next_frame.saturating_duration_since(Instant::now());
                // Err(true): every sender is gone. Err(false): nothing more for now.
                let received = if wait.is_zero() {
                    self.messages
                        .try_recv()
                        .map_err(|e| e == TryRecvError::Disconnected)
                } else {
                    self.messages
                        .recv_timeout(wait)
                        .map_err(|e| e == RecvTimeoutError::Disconnected)
                };
                match received {
                    Ok(RenderMessage::Command(command)) => {
                        self.engine.apply(&command, elapsed_ms(Instant::now()));
                        self.publish_levels();
                    }
                    Ok(RenderMessage::Shutdown) | Err(true) => stopping = true,
                    Err(false) => break,
                }
            }

            self.engine.render(elapsed_ms(Instant::now()), &mut frame);
            self.output.send(&frame);
            let status = self.output.status();
            self.output_status.send_if_modified(|current| {
                let changed = *current != status;
                if changed {
                    *current = status;
                }
                changed
            });

            if stopping {
                debug!("render loop stopped after sending its last frame");
                return;
            }

            next_frame += self.period;
            let now = Instant::now();
            if next_frame + self.period < now {
                // Far behind, for example after the machine was suspended:
                // start afresh rather than sending a burst of late frames.
                next_frame = now;
            }
        }
    }

    fn publish_levels(&self) {
        let commanded = self.engine.commanded_levels();
        self.levels.send_if_modified(|current| {
            let changed = *current != commanded;
            if changed {
                *current = commanded;
            }
            changed
        });
    }
}
