// ABOUTME: The lighting engine: every channel's level over time, from fades, pulses and twinkles.
// ABOUTME: Pure and deterministic; the caller passes the time and a seed for twinkle randomness.

use rand::rngs::StdRng;
use rand::{Rng, SeedableRng};

use crate::command::{Action, Command, PulseSpeed, TwinkleParams};
use crate::config::{Easing, CHANNEL_COUNT};

const FULL: f64 = 255.0;

/// A move from one level to another over a span of time.
#[derive(Debug, Clone, Copy)]
struct Ramp {
    from: f64,
    to: f64,
    start_ms: f64,
    duration_ms: f64,
    easing: Easing,
}

impl Ramp {
    fn end_ms(&self) -> f64 {
        self.start_ms + self.duration_ms
    }

    fn level_at(&self, now_ms: f64) -> f64 {
        if self.duration_ms <= 0.0 || now_ms >= self.end_ms() {
            return self.to;
        }
        let progress = ((now_ms - self.start_ms) / self.duration_ms).clamp(0.0, 1.0);
        self.from + (self.to - self.from) * ease(self.easing, progress)
    }
}

fn ease(easing: Easing, progress: f64) -> f64 {
    match easing {
        Easing::Linear => progress,
        Easing::Sine => (1.0 - (progress * std::f64::consts::PI).cos()) / 2.0,
    }
}

/// What a channel is following right now.
#[derive(Debug, Clone)]
enum Motion {
    /// Showing its commanded level.
    Steady,
    /// Moving to its commanded level.
    Fade(Ramp),
    /// Rising to full, holding, then falling back to its commanded level.
    Pulse {
        rise: Ramp,
        hold_ms: f64,
        fall_ms: f64,
        easing: Easing,
    },
    /// Moving between random levels until told otherwise.
    Twinkle { params: TwinkleParams, ramp: Ramp },
}

#[derive(Debug, Clone)]
struct Channel {
    commanded: u8,
    motion: Motion,
}

impl Channel {
    fn level_at(&self, now_ms: f64) -> f64 {
        match &self.motion {
            Motion::Steady => f64::from(self.commanded),
            Motion::Fade(ramp) | Motion::Twinkle { ramp, .. } => ramp.level_at(now_ms),
            Motion::Pulse {
                rise,
                hold_ms,
                fall_ms,
                easing,
            } => {
                let fall_start = rise.end_ms() + hold_ms;
                if now_ms < fall_start {
                    rise.level_at(now_ms)
                } else {
                    Ramp {
                        from: FULL,
                        to: f64::from(self.commanded),
                        start_ms: fall_start,
                        duration_ms: *fall_ms,
                        easing: *easing,
                    }
                    .level_at(now_ms)
                }
            }
        }
    }

    /// Finishes motions that are over and starts each new twinkle move that is due.
    fn advance(&mut self, now_ms: f64, rng: &mut StdRng) {
        match &mut self.motion {
            Motion::Steady => {}
            Motion::Fade(ramp) => {
                if now_ms >= ramp.end_ms() {
                    self.motion = Motion::Steady;
                }
            }
            Motion::Pulse {
                rise,
                hold_ms,
                fall_ms,
                ..
            } => {
                if now_ms >= rise.end_ms() + *hold_ms + *fall_ms {
                    self.motion = Motion::Steady;
                }
            }
            Motion::Twinkle { params, ramp } => {
                if now_ms - ramp.end_ms() > params.max_duration_ms {
                    // Frames stopped for a long time; carry on from now rather
                    // than replaying every move that was missed.
                    *ramp = twinkle_ramp(params, ramp.to, now_ms, rng);
                }
                while now_ms >= ramp.end_ms() {
                    *ramp = twinkle_ramp(params, ramp.to, ramp.end_ms(), rng);
                }
            }
        }
    }
}

fn twinkle_ramp(params: &TwinkleParams, from: f64, start_ms: f64, rng: &mut StdRng) -> Ramp {
    Ramp {
        from,
        to: f64::from(rng.gen_range(params.min_level..=params.max_level)),
        start_ms,
        duration_ms: rng.gen_range(params.min_duration_ms..=params.max_duration_ms),
        easing: params.easing,
    }
}

/// Every channel's commanded level and motion. Each channel follows exactly one
/// motion at a time; a command replaces the motion of the channels it names and
/// starts it from the level showing at that moment.
pub struct Engine {
    channels: Vec<Channel>,
    rng: StdRng,
}

impl Engine {
    pub fn new(startup_level: u8, seed: u64) -> Self {
        Engine {
            channels: vec![
                Channel {
                    commanded: startup_level,
                    motion: Motion::Steady,
                };
                CHANNEL_COUNT
            ],
            rng: StdRng::seed_from_u64(seed),
        }
    }

    pub fn apply(&mut self, command: &Command, now_ms: f64) {
        for &index in &command.channels {
            let Some(channel) = self.channels.get_mut(index) else {
                continue;
            };
            channel.advance(now_ms, &mut self.rng);
            let showing = channel.level_at(now_ms);
            match &command.action {
                Action::Set {
                    level,
                    fade_ms,
                    easing,
                } => {
                    channel.commanded = *level;
                    channel.motion = if *fade_ms > 0.0 {
                        Motion::Fade(Ramp {
                            from: showing,
                            to: f64::from(*level),
                            start_ms: now_ms,
                            duration_ms: *fade_ms,
                            easing: *easing,
                        })
                    } else {
                        Motion::Steady
                    };
                }
                Action::Pulse { speed, easing } => {
                    channel.motion = pulse(*speed, *easing, showing, now_ms);
                }
                Action::TwinkleStart(params) => {
                    let ramp = twinkle_ramp(params, showing, now_ms, &mut self.rng);
                    channel.motion = Motion::Twinkle {
                        params: params.clone(),
                        ramp,
                    };
                }
                Action::TwinkleStop { fade_ms, easing } => {
                    if matches!(channel.motion, Motion::Twinkle { .. }) {
                        channel.motion = if *fade_ms > 0.0 {
                            Motion::Fade(Ramp {
                                from: showing,
                                to: f64::from(channel.commanded),
                                start_ms: now_ms,
                                duration_ms: *fade_ms,
                                easing: *easing,
                            })
                        } else {
                            Motion::Steady
                        };
                    }
                }
            }
        }
    }

    /// Writes every channel's level at `now_ms`, rounded to 8 bits, into `frame`.
    pub fn render(&mut self, now_ms: f64, frame: &mut [u8; CHANNEL_COUNT]) {
        for (channel, slot) in self.channels.iter_mut().zip(frame.iter_mut()) {
            channel.advance(now_ms, &mut self.rng);
            *slot = channel.level_at(now_ms).round().clamp(0.0, FULL) as u8;
        }
    }

    /// The level each channel was last set or faded to.
    pub fn commanded_levels(&self) -> [u8; CHANNEL_COUNT] {
        let mut levels = [0u8; CHANNEL_COUNT];
        for (channel, level) in self.channels.iter().zip(levels.iter_mut()) {
            *level = channel.commanded;
        }
        levels
    }
}

fn pulse(speed: PulseSpeed, easing: Easing, showing: f64, now_ms: f64) -> Motion {
    let (rise_ms, hold_ms, fall_ms) = speed.timings_ms();
    Motion::Pulse {
        rise: Ramp {
            from: showing,
            to: FULL,
            start_ms: now_ms,
            duration_ms: rise_ms,
            easing,
        },
        hold_ms,
        fall_ms,
        easing,
    }
}
