// ABOUTME: Tests for the lighting engine: fades, retargeting, pulse, twinkle and commanded levels.
// ABOUTME: Time is passed in explicitly, so every test is deterministic.

use mqttdmx::command::{Action, Command, PulseSpeed, Target, TwinkleParams};
use mqttdmx::config::{Easing, CHANNEL_COUNT};
use mqttdmx::engine::Engine;

const SEED: u64 = 7;

fn frame(engine: &mut Engine, now_ms: f64) -> [u8; CHANNEL_COUNT] {
    let mut frame = [0u8; CHANNEL_COUNT];
    engine.render(now_ms, &mut frame);
    frame
}

/// Level of 1-based `channel` at `now_ms`.
fn level(engine: &mut Engine, channel: usize, now_ms: f64) -> u8 {
    frame(engine, now_ms)[channel - 1]
}

fn command(channels: &[usize], action: Action) -> Command {
    Command {
        target: Target::Group("test".to_string()),
        channels: channels.iter().map(|c| c - 1).collect(),
        action,
    }
}

fn set(channels: &[usize], level: u8) -> Command {
    command(
        channels,
        Action::Set {
            level,
            fade_ms: 0.0,
            easing: Easing::Linear,
        },
    )
}

fn fade(channels: &[usize], level: u8, fade_ms: f64, easing: Easing) -> Command {
    command(
        channels,
        Action::Set {
            level,
            fade_ms,
            easing,
        },
    )
}

fn pulse(channels: &[usize], speed: PulseSpeed) -> Command {
    command(
        channels,
        Action::Pulse {
            speed,
            easing: Easing::Linear,
        },
    )
}

fn twinkle(channels: &[usize], min_level: u8, max_level: u8) -> Command {
    command(
        channels,
        Action::TwinkleStart(TwinkleParams {
            min_level,
            max_level,
            min_duration_ms: 500.0,
            max_duration_ms: 2000.0,
            easing: Easing::Sine,
        }),
    )
}

fn stop_twinkle(channels: &[usize], fade_ms: f64) -> Command {
    command(
        channels,
        Action::TwinkleStop {
            fade_ms,
            easing: Easing::Linear,
        },
    )
}

#[test]
fn every_channel_starts_at_the_startup_level() {
    let mut engine = Engine::new(255, SEED);
    assert!(frame(&mut engine, 0.0).iter().all(|&v| v == 255));
    assert!(engine.commanded_levels().iter().all(|&v| v == 255));
    let mut dark = Engine::new(0, SEED);
    assert!(frame(&mut dark, 1000.0).iter().all(|&v| v == 0));
}

#[test]
fn an_instant_set_changes_only_its_channels() {
    let mut engine = Engine::new(0, SEED);
    engine.apply(&set(&[1, 512], 200), 100.0);
    let out = frame(&mut engine, 100.0);
    assert_eq!(out[0], 200);
    assert_eq!(out[511], 200);
    assert_eq!(out.iter().filter(|&&v| v != 0).count(), 2);
}

#[test]
fn a_linear_fade_moves_at_a_constant_rate_and_stops_at_its_target() {
    let mut engine = Engine::new(0, SEED);
    engine.apply(&fade(&[1], 200, 1000.0, Easing::Linear), 0.0);
    assert_eq!(level(&mut engine, 1, 0.0), 0);
    assert_eq!(level(&mut engine, 1, 250.0), 50);
    assert_eq!(level(&mut engine, 1, 500.0), 100);
    assert_eq!(level(&mut engine, 1, 999.0), 200, "199.8 rounds to 200");
    assert_eq!(level(&mut engine, 1, 1000.0), 200);
    assert_eq!(level(&mut engine, 1, 60_000.0), 200);
}

#[test]
fn a_sine_fade_starts_and_ends_gently() {
    let mut engine = Engine::new(0, SEED);
    engine.apply(&fade(&[1], 200, 1000.0, Easing::Sine), 0.0);
    // (1 - cos(pi * t)) / 2 at t = 0.1, 0.25, 0.5, 0.75
    assert_eq!(level(&mut engine, 1, 100.0), 5, "200 * 0.0245 = 4.9");
    assert_eq!(level(&mut engine, 1, 250.0), 29, "200 * 0.1464 = 29.3");
    assert_eq!(level(&mut engine, 1, 500.0), 100);
    assert_eq!(level(&mut engine, 1, 750.0), 171, "200 * 0.8536 = 170.7");
    assert_eq!(level(&mut engine, 1, 1000.0), 200);
}

#[test]
fn a_fade_down_reaches_zero() {
    let mut engine = Engine::new(200, SEED);
    engine.apply(&fade(&[1], 0, 3000.0, Easing::Sine), 0.0);
    assert_eq!(level(&mut engine, 1, 1500.0), 100);
    assert_eq!(level(&mut engine, 1, 3000.0), 0);
}

#[test]
fn a_new_fade_starts_from_the_level_showing_without_a_jump() {
    let mut engine = Engine::new(0, SEED);
    engine.apply(&fade(&[1], 255, 1000.0, Easing::Linear), 0.0);
    assert_eq!(level(&mut engine, 1, 500.0), 128, "127.5 showing");
    engine.apply(&fade(&[1], 0, 1000.0, Easing::Linear), 500.0);
    assert_eq!(
        level(&mut engine, 1, 500.0),
        128,
        "still 127.5 at the moment of retargeting"
    );
    assert_eq!(
        level(&mut engine, 1, 1000.0),
        64,
        "halfway from 127.5 to 0 is 63.75"
    );
    assert_eq!(level(&mut engine, 1, 1500.0), 0);
}

#[test]
fn overlapping_fades_leave_the_other_channels_of_the_first_fade_running() {
    let mut engine = Engine::new(0, SEED);
    engine.apply(&fade(&[1, 2], 255, 1000.0, Easing::Linear), 0.0);
    engine.apply(&fade(&[2, 3], 255, 2000.0, Easing::Linear), 0.0);
    let out = frame(&mut engine, 1000.0);
    assert_eq!(out[0], 255, "channel 1 finishes the first fade");
    assert_eq!(out[1], 128, "channel 2 follows the second fade");
    assert_eq!(out[2], 128, "channel 3 follows the second fade");
    let out = frame(&mut engine, 2000.0);
    assert_eq!(&out[..3], &[255, 255, 255]);
}

#[test]
fn setting_one_channel_mid_fade_does_not_freeze_the_rest() {
    let mut engine = Engine::new(255, SEED);
    engine.apply(&fade(&[1, 2, 3, 4], 0, 3000.0, Easing::Linear), 0.0);
    engine.apply(&set(&[1], 255), 1000.0);
    let out = frame(&mut engine, 4500.0);
    assert_eq!(&out[..4], &[255, 0, 0, 0]);
}

#[test]
fn an_instant_set_cancels_a_running_fade() {
    let mut engine = Engine::new(0, SEED);
    engine.apply(&fade(&[1], 255, 1000.0, Easing::Linear), 0.0);
    engine.apply(&set(&[1], 0), 300.0);
    assert_eq!(level(&mut engine, 1, 300.0), 0);
    assert_eq!(level(&mut engine, 1, 2000.0), 0);
}

#[test]
fn commanded_levels_follow_sets_and_fade_targets_immediately() {
    let mut engine = Engine::new(0, SEED);
    engine.apply(&set(&[1], 50), 0.0);
    engine.apply(&fade(&[2], 200, 10_000.0, Easing::Linear), 0.0);
    let commanded = engine.commanded_levels();
    assert_eq!(commanded[0], 50);
    assert_eq!(commanded[1], 200, "a fade reports its destination at once");
    assert_eq!(commanded[2], 0);
}

#[test]
fn a_pulse_rises_holds_and_returns_to_the_commanded_level() {
    let mut engine = Engine::new(100, SEED);
    engine.apply(&pulse(&[1], PulseSpeed::Slow), 0.0);
    assert_eq!(level(&mut engine, 1, 0.0), 100);
    assert_eq!(
        level(&mut engine, 1, 750.0),
        178,
        "halfway up from 100 to 255"
    );
    assert_eq!(level(&mut engine, 1, 1500.0), 255);
    assert_eq!(level(&mut engine, 1, 3499.0), 255, "holding");
    assert_eq!(level(&mut engine, 1, 4250.0), 178, "halfway down");
    assert_eq!(level(&mut engine, 1, 5000.0), 100);
    assert_eq!(level(&mut engine, 1, 9000.0), 100);
    assert_eq!(
        engine.commanded_levels()[0],
        100,
        "pulse never changes the commanded level"
    );
}

#[test]
fn a_fast_pulse_uses_the_fast_timings() {
    let mut engine = Engine::new(0, SEED);
    engine.apply(&pulse(&[1], PulseSpeed::Fast), 0.0);
    assert_eq!(level(&mut engine, 1, 250.0), 128);
    assert_eq!(level(&mut engine, 1, 500.0), 255);
    assert_eq!(level(&mut engine, 1, 1000.0), 255);
    assert_eq!(level(&mut engine, 1, 1250.0), 128);
    assert_eq!(level(&mut engine, 1, 1500.0), 0);
}

#[test]
fn a_repeated_pulse_still_returns_to_the_commanded_level() {
    let mut engine = Engine::new(100, SEED);
    engine.apply(&pulse(&[1], PulseSpeed::Slow), 0.0);
    assert_eq!(level(&mut engine, 1, 4250.0), 178);
    engine.apply(&pulse(&[1], PulseSpeed::Slow), 4250.0);
    assert_eq!(
        level(&mut engine, 1, 4250.0),
        178,
        "restarts from the level showing"
    );
    assert_eq!(level(&mut engine, 1, 4250.0 + 1500.0), 255);
    assert_eq!(
        level(&mut engine, 1, 4250.0 + 5000.0),
        100,
        "not the mid-ramp 178"
    );
}

#[test]
fn a_set_during_a_pulse_cancels_it() {
    let mut engine = Engine::new(100, SEED);
    engine.apply(&pulse(&[1], PulseSpeed::Slow), 0.0);
    engine.apply(&set(&[1], 0), 2000.0);
    assert_eq!(level(&mut engine, 1, 2000.0), 0);
    assert_eq!(level(&mut engine, 1, 6000.0), 0);
}

#[test]
fn a_pulse_returns_to_a_level_commanded_by_an_earlier_fade() {
    let mut engine = Engine::new(0, SEED);
    engine.apply(&fade(&[1], 200, 10_000.0, Easing::Linear), 0.0);
    engine.apply(&pulse(&[1], PulseSpeed::Fast), 1000.0);
    assert_eq!(level(&mut engine, 1, 1000.0), 20, "the fade had reached 20");
    assert_eq!(
        level(&mut engine, 1, 1000.0 + 1500.0),
        200,
        "returns to the fade's target"
    );
}

/// Samples `channels` (1-based) every 25 ms from `from_ms` to `to_ms`, moving
/// forward in time the way the render loop does. Returns one series per channel.
fn sample(engine: &mut Engine, channels: &[usize], from_ms: f64, to_ms: f64) -> Vec<Vec<u8>> {
    let mut series = vec![Vec::new(); channels.len()];
    let mut t = from_ms;
    while t <= to_ms {
        let out = frame(engine, t);
        for (values, &channel) in series.iter_mut().zip(channels) {
            values.push(out[channel - 1]);
        }
        t += 25.0;
    }
    series
}

#[test]
fn twinkle_stays_between_its_levels_and_keeps_moving() {
    let mut engine = Engine::new(0, SEED);
    engine.apply(&twinkle(&[1, 2, 3], 100, 200), 0.0);
    frame(&mut engine, 0.0);
    let series = sample(&mut engine, &[1, 2, 3], 2000.0, 60_000.0);
    for (channel, levels) in (1..=3).zip(&series) {
        assert!(
            levels.iter().all(|&v| (100..=200).contains(&v)),
            "channel {channel} left 100-200"
        );
        let distinct: std::collections::BTreeSet<u8> = levels.iter().copied().collect();
        assert!(
            distinct.len() > 20,
            "channel {channel} barely moved: {distinct:?}"
        );
    }
    assert!(
        frame(&mut engine, 10_000.0)[3..].iter().all(|&v| v == 0),
        "other channels untouched"
    );
}

#[test]
fn twinkling_channels_move_independently() {
    let mut engine = Engine::new(0, SEED);
    engine.apply(&twinkle(&[1, 2], 0, 255), 0.0);
    let series = sample(&mut engine, &[1, 2], 0.0, 20_000.0);
    let differing = series[0]
        .iter()
        .zip(&series[1])
        .filter(|(a, b)| a != b)
        .count();
    assert!(
        differing > series[0].len() / 2,
        "channels moved together: {differing} of {} differ",
        series[0].len()
    );
}

#[test]
fn twinkle_starts_from_the_level_showing() {
    let mut engine = Engine::new(0, SEED);
    engine.apply(&set(&[1], 30), 0.0);
    engine.apply(&twinkle(&[1], 100, 255), 1000.0);
    assert_eq!(level(&mut engine, 1, 1000.0), 30);
    assert!(
        level(&mut engine, 1, 1025.0) <= 40,
        "moves away gently, no pop to the midpoint"
    );
    assert_eq!(
        engine.commanded_levels()[0],
        30,
        "twinkle never changes the commanded level"
    );
}

#[test]
fn stopping_a_twinkle_returns_to_the_commanded_level() {
    let mut engine = Engine::new(200, SEED);
    engine.apply(&twinkle(&[1], 0, 100), 0.0);
    let showing = level(&mut engine, 1, 5000.0);
    engine.apply(&stop_twinkle(&[1], 1000.0), 5000.0);
    assert_eq!(
        level(&mut engine, 1, 5000.0),
        showing,
        "the return fade starts where the twinkle was"
    );
    assert_eq!(level(&mut engine, 1, 6000.0), 200);
    assert_eq!(level(&mut engine, 1, 20_000.0), 200);
}

#[test]
fn stopping_a_twinkle_instantly() {
    let mut engine = Engine::new(50, SEED);
    engine.apply(&twinkle(&[1], 100, 255), 0.0);
    engine.apply(&stop_twinkle(&[1], 0.0), 3000.0);
    assert_eq!(level(&mut engine, 1, 3000.0), 50);
}

#[test]
fn stopping_a_twinkle_leaves_channels_that_are_not_twinkling_alone() {
    let mut engine = Engine::new(0, SEED);
    engine.apply(&twinkle(&[1], 100, 255), 0.0);
    engine.apply(&fade(&[2], 255, 1000.0, Easing::Linear), 0.0);
    engine.apply(&stop_twinkle(&[1, 2], 0.0), 500.0);
    assert_eq!(
        level(&mut engine, 1, 500.0),
        0,
        "channel 1 returns to its commanded 0"
    );
    assert_eq!(
        level(&mut engine, 2, 750.0),
        191,
        "channel 2's fade carries on"
    );
}

#[test]
fn a_blackout_stops_every_effect() {
    let mut engine = Engine::new(255, SEED);
    engine.apply(&twinkle(&[1, 2, 3], 100, 255), 0.0);
    engine.apply(&pulse(&[4], PulseSpeed::Slow), 0.0);
    engine.apply(&fade(&[5], 0, 10_000.0, Easing::Linear), 0.0);
    let everything: Vec<usize> = (1..=CHANNEL_COUNT).collect();
    engine.apply(&set(&everything, 0), 1000.0);
    for t in [1000.0, 2000.0, 30_000.0] {
        assert!(
            frame(&mut engine, t).iter().all(|&v| v == 0),
            "not dark at {t}"
        );
    }
}

#[test]
fn a_long_gap_between_frames_does_not_stall_twinkle() {
    let mut engine = Engine::new(0, SEED);
    engine.apply(&twinkle(&[1], 100, 200), 0.0);
    let started = std::time::Instant::now();
    let value = level(&mut engine, 1, 3_600_000.0);
    assert!(started.elapsed() < std::time::Duration::from_millis(100));
    assert!((100..=200).contains(&value));
}

#[test]
fn the_same_seed_gives_the_same_twinkle() {
    let mut first = Engine::new(0, SEED);
    let mut second = Engine::new(0, SEED);
    first.apply(&twinkle(&[1], 0, 255), 0.0);
    second.apply(&twinkle(&[1], 0, 255), 0.0);
    assert_eq!(
        sample(&mut first, &[1], 0.0, 10_000.0),
        sample(&mut second, &[1], 0.0, 10_000.0)
    );
}

#[test]
fn a_command_with_no_channels_changes_nothing() {
    let mut engine = Engine::new(10, SEED);
    engine.apply(&set(&[], 255), 0.0);
    assert!(frame(&mut engine, 0.0).iter().all(|&v| v == 10));
}
