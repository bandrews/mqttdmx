# Changelog

All notable changes to mqttdmx are recorded here. The format follows
[Keep a Changelog](https://keepachangelog.com/en/1.0.0/) and versions follow
[Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [2.0.0] - 2026-09-25

A rewrite in Rust. The Node.js version is kept in `legacy/` for reference. The reasoning behind the
changes is in [docs/decisions.md](docs/decisions.md).

### Added

- Retained status topics: `dmx/status/online` (with an MQTT Last Will), `dmx/status/health` (version
  and interface state) and `dmx/status/groups` (each group's set level). They are republished on every
  reconnect.
- Recovery from interface problems without restarting: a missing or unplugged interface is retried
  with backoff, and one that stops taking data is kept open until it recovers. The current levels are
  sent as soon as it's back.
- `--device auto`, which finds a single Enttec DMX USB Pro by its USB name, and `--list-devices`.
- `--check-config`.
- Pulse (`{"pulse": "slow"}` or `"fast"`).
- Fade easing (`"easing": "linear"` or `"sine"`, default set by `fade.easing`).
- `{"twinkle": false, "fade": ms}` to return from a twinkle gradually.
- `--startup-level` / `output.startup_level`.
- A systemd unit, a Dockerfile, and local validation scripts that run the tests against a real
  mosquitto and a simulated interface.

### Changed

- **Breaking:** single-channel commands are on `dmx/channel/<n>` instead of `dmx/<n>`.
- **Breaking:** twinkle fields are `min_level`, `max_level`, `min_duration` and `max_duration`
  (previously camelCase with "Brightness"); `variance` is gone. Durations must be at least 200 ms.
- **Breaking:** `--broker mqtt://host` is replaced by `--server host` and `--port`. `--driver` has no
  default, and the only drivers are `enttec-usb-dmx-pro` and `null`.
- **Breaking:** unknown keys inside a config section are an error. Top-level keys are still ignored.
- Commands are validated in full; unknown fields, out-of-range or fractional levels, and conflicting
  actions reject the whole message with a logged reason. Previously values like `"abc"` or `300` were
  coerced and sent.
- A bad config file stops mqttdmx at startup. Previously it ran with no groups.
- Each channel follows the last command that touched it. Setting a level cancels fades and twinkles
  on those channels.

### Fixed

- Changing one channel of a multi-channel fade no longer freezes the other channels.
- A running fade no longer overwrites a later instant set.
- SIGTERM is handled: a final frame is sent and `offline` is published.
- A malformed message can no longer crash the process.
- A USB unplug no longer crashes the process, and a missing interface at startup is picked up when it
  appears.
- Frames come from a single fixed-rate loop instead of several timers, which gave uneven fade steps.

### Removed

- The `channels` map and `values` array forms on the base topic.
- The `dmx/get/groups` and `dmx/get/state` topics, replaced by the retained status topics.
- The Open DMX USB, DMX4ALL and other drivers of the `dmx` npm package.

## [1.0.0] - 2025-12-03

The Node.js version, now in `legacy/`.
