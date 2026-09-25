# mqttdmx

mqttdmx is a small daemon that drives one DMX universe from MQTT messages. It talks to an Enttec DMX
USB Pro (or a compatible interface), lets you name groups of channels, and handles fades, pulses and
a random "twinkle" effect itself, so the sender only has to say what it wants and how long it should
take.

It is written in Rust. An earlier Node.js prototype is kept in [`legacy/`](legacy/) for reference.

## Quick start

You need Rust 1.85 or later, an MQTT broker such as Mosquitto, and either an Enttec DMX USB Pro or
nothing at all (the `null` driver sends nothing and is useful for trying things out).

```bash
git clone https://github.com/bandrews/mqttdmx.git
cd mqttdmx
cargo build --release
```

Write a config file that names your channels:

```json
{
  "groups": {
    "HouseLights": { "channels": [1, 2, 3, 4] },
    "Spotlight":   { "channels": [10] }
  }
}
```

Find the interface and start the daemon:

```bash
./target/release/mqttdmx --list-devices
./target/release/mqttdmx --config lights.json --server localhost \
    --driver enttec-usb-dmx-pro --device auto
```

On Linux the user running mqttdmx needs to be in the `dialout` group to open the interface.

Then send it something:

```bash
mosquitto_pub -t dmx/group/HouseLights -m '{"value": 255, "fade": 2000}'
```

## Commands

Commands are addressed by topic. Levels are 0–255 and times are in milliseconds.

```bash
# Set a group, or fade it
mosquitto_pub -t dmx/group/HouseLights -m '{"value": 255}'
mosquitto_pub -t dmx/group/HouseLights -m '{"value": 0, "fade": 10000}'

# A bare number also works on group and channel topics
mosquitto_pub -t dmx/channel/10 -m 255

# Everything off, cancelling any running effect
mosquitto_pub -t dmx -m '{"range": {"start": 1, "end": 512}, "value": 0}'

# Effects
mosquitto_pub -t dmx/group/Spotlight -m '{"pulse": "fast"}'
mosquitto_pub -t dmx/group/HouseLights -m '{"twinkle": true}'
mosquitto_pub -t dmx/group/HouseLights -m '{"twinkle": false}'
```

[docs/commands.md](docs/commands.md) has the full list of fields and the rules for what gets
rejected.

## How it behaves

**Overlapping commands.** Each channel follows the last command that touched it. If you fade a group
and then set one of its channels, that channel stops fading and the rest carry on. Setting a level
always cancels a fade, pulse or twinkle on those channels, so a blackout really is a blackout.

**Fades.** A fade starts from whatever level the channel is showing at that moment, so interrupting
one fade with another doesn't cause a jump. Fades are linear by default; `"easing": "sine"` gives an
S-curve that eases in and out. Output is sent at 40 frames per second, rounded to the 256 steps DMX
allows, so very slow fades near zero will show individual steps on some dimmers.

**Pulse and twinkle.** Both leave the channel's set level alone and return to it when they finish or
are stopped.

**Bad input.** A message that doesn't parse, has an unknown field, or has a value out of range is
rejected whole and logged with the reason. Nothing is partly applied. Commands published with the
MQTT retain flag are applied when they arrive, but the stored copy the broker replays after a
reconnect is ignored, so an old cue is never re-run.

**Startup and shutdown.** Every channel starts at `startup_level` (0 unless you set it), and that is
the first frame sent. On SIGTERM or SIGINT mqttdmx sends one last frame and exits. The Enttec Pro
keeps repeating the last frame it received while it has power, so the lights hold their state while
mqttdmx is stopped or restarting. Levels are not saved; after a restart everything is back at
`startup_level`.

**When things fail.** If the interface is missing or unplugged, mqttdmx keeps running, keeps track of
levels and fades, and reopens the interface when it comes back, sending the current state straight
away. If the broker is unreachable, it keeps retrying and the lights stay as they are. Neither
condition makes it exit; only a bad config file does.

## Monitoring

mqttdmx publishes three retained topics:

| Topic | Contents |
|---|---|
| `dmx/status/online` | `online`, or `offline` when mqttdmx stops or its connection drops (MQTT Last Will) |
| `dmx/status/health` | version, and the interface's state, device path, firmware version and last error |
| `dmx/status/groups` | the level each group was last set to, or `null` if its channels differ |

Output is working when `online` is `online` and `output.state` in `health` is `connected`.

Logs go to standard output. Each accepted command is logged at INFO and each rejected one at WARN:

```text
INFO mqttdmx::mqtt: group HouseLights: fade to 0 over 3000 ms (linear)
WARN mqttdmx::mqtt: rejected command on dmx/group/HouseLights: unknown field "fdae"; payload: "{\"value\":0,\"fdae\":3000}"
```

## Configuration and deployment

Settings come from a JSON file and command-line flags; flags win. Only `output.driver` (and
`output.device` for the Enttec driver) has no default. See
[docs/configuration.md](docs/configuration.md) and [`config.example.json`](config.example.json).

A systemd unit is in [`packaging/mqttdmx.service`](packaging/mqttdmx.service) and a Dockerfile is in
the repository root. [docs/deployment.md](docs/deployment.md) covers both, including how to give a
container access to an interface that may be unplugged and replugged, and a list of things to check
on real hardware.

## Documentation

- [docs/commands.md](docs/commands.md): topics, payloads, validation and status topics
- [docs/configuration.md](docs/configuration.md): config file and command-line flags
- [docs/deployment.md](docs/deployment.md): systemd, Docker and hardware checks
- [docs/troubleshooting.md](docs/troubleshooting.md): error messages and what they mean
- [docs/decisions.md](docs/decisions.md): why it works the way it does
- [CONTRIBUTING.md](CONTRIBUTING.md): building and testing
- [CHANGELOG.md](CHANGELOG.md)

## Limitations

- One universe, output through an Enttec DMX USB Pro or a compatible interface. No Open DMX USB,
  Art-Net or sACN.
- No TLS connection to the broker.
- 8-bit channels only.
- The config file is read once at startup; changing groups needs a restart.

## License

[MIT](LICENSE)

## AI statement

The design, review and testing are human; most of the Rust implementation was written by Claude,
Anthropic's AI model.

Copyright (c) 2025-2026 Mo Fang Heavy Industries LLC.
