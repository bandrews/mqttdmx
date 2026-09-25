# Contributing to mqttdmx

Contributions are welcome! This document explains how to set up a development environment, run the
tests and validate a change.

## Development Setup

### Prerequisites

- Rust 1.95 (matches the Linux validation image; the locked dependencies need 1.85 or later)
- mosquitto and its command-line clients, for the broker tests

```bash
# Debian/Ubuntu
sudo apt-get install mosquitto mosquitto-clients

# macOS
brew install mosquitto
```

No other system libraries are needed.

### Building

```bash
cargo build            # Debug build
cargo build --release  # Release build
```

## Running Tests

Validate locally. GitHub Actions is intentionally disabled for this repository to avoid hosted CI
costs and quota usage. Do not add or enable Actions workflows without an explicit maintainer request.

```bash
cargo test                           # Everything except the broker tests
MQTTDMX_BROKER_TESTS=1 cargo test    # Everything, starting mosquitto on free ports as needed
cargo test --test engine_test        # One suite
```

| Suite | Covers |
|---|---|
| `config_test` | Config loading, overrides and every validation rule |
| `command_test` | Topic and payload parsing, and every rejection message |
| `engine_test` | Fades, retargeting, overlapping groups, pulse, twinkle and commanded levels, on a simulated clock |
| `status_test` | The group-level and health JSON documents |
| `output_test` | Enttec Pro framing, device selection and the serial driver, including unplug and replug |
| `render_test` | Frame pacing, command latency, fade smoothness and shutdown of the render loop |
| `daemon_test` | The real binary against a real broker and a simulated Enttec interface (broker tests) |
| `fuzz_test` | Property tests that untrusted input never panics |

The serial tests use a pseudo-terminal whose other end behaves like an Enttec DMX USB Pro
(`tests/support/mod.rs`), so no hardware is needed. They need Linux pseudo-terminals: on other
systems they, and the broker tests built on them, are reported as ignored, so run Lane A there.
Hardware checks are listed in [docs/deployment.md](docs/deployment.md#checking-real-hardware).

## Validating a Change

`scripts/validate.sh` runs every check a change must pass: formatting, clippy with warnings as errors,
a warning-free release build, the fuzz suite, and every test including the broker tests.

```bash
./scripts/validate.sh            # Lane A: inside a Linux container (needs Docker)
./scripts/validate.sh --native   # Lane B: on this host (needs mosquitto installed)
```

## Code Quality

- `cargo fmt` before committing.
- `cargo clippy --all-targets -- -D warnings` must be clean.
- The release build must have no warnings.
- Every source file starts with two `// ABOUTME:` lines saying what it does.
- Parse untrusted input into typed values and reject it with a clear reason; never let it panic.
  Release builds abort on panic.
- New features and bug fixes start with a failing test.

## Project Layout

| Path | Contents |
|---|---|
| `src/config.rs` | Config file model, command-line overrides, validation |
| `src/command.rs` | MQTT topic and payload parsing into commands |
| `src/engine.rs` | Per-channel levels over time: fades, pulse, twinkle |
| `src/render.rs` | The render thread: applies commands and sends frames at the frame rate |
| `src/output/` | Output drivers (Enttec DMX USB Pro, null) and serial device discovery |
| `src/mqtt.rs` | Broker connection, command subscription and retained status publishing |
| `src/status.rs` | Status documents |
| `src/main.rs` | Command line, startup, signal handling and shutdown |
| `legacy/` | The Node.js prototype, for reference only |

## Pull Requests

1. Branch from `main`.
2. Write a failing test, make it pass, and keep the change focused.
3. Run `./scripts/validate.sh` (or `--native`).
4. Update the docs and the changelog for any behaviour change.
