# Configuration

mqttdmx reads an optional JSON file (`--config`) and command-line flags. A flag overrides the same
setting in the file, and anything set in neither place uses its default.

The configuration is checked at startup, and mqttdmx exits with status 1 before touching the output
if anything is wrong. There are two kinds of error:

- Problems reading or parsing the file stop at the first one and print a single line starting
  `cannot read config file` or `invalid config file`. These are: an unreadable file, invalid JSON, an
  unknown key inside a section, or a value of the wrong type or outside its type's range (for example
  `"startup_level": 300`).
- Everything else, such as a missing driver, an out-of-range frame rate or a bad channel number, is
  collected and printed together under `Configuration is invalid:`.

An invalid command-line value is rejected by the argument parser with status 2. `--check-config` runs
the checks, prints a summary of the result and exits.

## Command-line flags

| Flag | Config key | Meaning |
|---|---|---|
| `-c`, `--config <FILE>` | | JSON config file |
| `-s`, `--server <HOST>` | `mqtt.server` | broker host name or IP address |
| `-p`, `--port <PORT>` | `mqtt.port` | broker port |
| `-t`, `--topic <TOPIC>` | `mqtt.topic` | base topic |
| `--mqtt-username <NAME>` | `mqtt.username` | broker user name |
| `--mqtt-password <PASSWORD>` | `mqtt.password` | broker password |
| `--driver <DRIVER>` | `output.driver` | `enttec-usb-dmx-pro` or `null` |
| `-d`, `--device <PATH>` | `output.device` | serial device path, or `auto` |
| `--startup-level <LEVEL>` | `output.startup_level` | level every channel starts at, 0–255 |
| `--log-format <FORMAT>` | `logging.format` | `text` or `json` |
| `-v`, `--verbose` | | log at debug level |
| `--check-config` | | check the configuration, print a summary and exit |
| `--list-devices` | | list serial devices, mark Enttec interfaces, and exit |
| `-V`, `--version` | | print the version |
| `-h`, `--help` | | print help |

## The config file

Every section and every key is optional except `output.driver`, and `output.device` when the driver
is `enttec-usb-dmx-pro`. Both can be given on the command line instead.

Unknown keys at the top level of the file are ignored, so the file can carry settings for other
programs. An unknown key inside a section or a group is an error, so that a misspelt setting can't
quietly fall back to its default.

[`config.example.json`](../config.example.json) lists every key with its default, plus example values
for the driver, device and groups.

### mqtt

| Key | Default | Meaning |
|---|---|---|
| `server` | `"localhost"` | broker host name or IP address |
| `port` | `1883` | broker port |
| `topic` | `"dmx"` | base topic; no `+` or `#`, and no leading, trailing or doubled `/` |
| `client_id` | `mqttdmx-` and 8 random hex digits | set this only if your broker needs a fixed id; two clients with the same id keep disconnecting each other |
| `username`, `password` | none | must be set together |

The connection is plain TCP; TLS is not supported. If the broker can't be reached, mqttdmx retries
after 0.5 s, doubling the wait up to 10 s. The wait only resets once a connection has stayed up for
5 s, so a connection that fails immediately every time (for example because of an oversized retained
message, see [troubleshooting](troubleshooting.md)) backs off instead of reconnecting in a tight
loop. The keep-alive is 10 s.

### output

| Key | Default | Meaning |
|---|---|---|
| `driver` | none, required | `enttec-usb-dmx-pro`, or `null` to send nothing |
| `device` | none, required for `enttec-usb-dmx-pro` | serial device path, or `auto` |
| `frame_rate` | `40` | frames per second, 1–44 (a full universe can't go faster than 44) |
| `startup_level` | `0` | level all 512 channels start at |

**Choosing the device.** Linux numbers USB serial devices (`/dev/ttyUSB0`, `/dev/ttyUSB1`, ...) in the
order they appear, so the number can change after a replug or reboot. Use either the stable name
under `/dev/serial/by-id/` (`--list-devices` shows it) or `auto`. `auto` looks for exactly one serial
device whose USB manufacturer or product name identifies it as an Enttec DMX USB Pro; if there is
none, or more than one, it reports that and keeps looking. Compatible interfaces from other makers
need an explicit path.

**What happens at runtime.**

- A full 512-channel frame is sent every `1/frame_rate` seconds. The first frame after the device
  opens carries the current levels.
- When the device opens, mqttdmx asks the interface for its firmware version and waits up to half a
  second for the answer. If none comes, it logs a warning and uses the device anyway.
- The device is opened exclusively. If another program has it open, the open fails and is retried.
- If the device is missing, or a write fails with an error such as the device being unplugged,
  mqttdmx closes it and tries again after 0.25 s, doubling the wait up to 5 s. Levels and fades carry
  on in memory meanwhile. The first failure and each recovery are logged; repeated identical failures
  are not.
- If the interface is still plugged in but stops taking data, mqttdmx keeps it open and skips frames
  until it takes data again. No frame waits longer than 0.1 s. After a second of this the interface is
  reported as `disconnected` with "the interface has stopped accepting data". mqttdmx doesn't close
  the device in this case because closing a USB serial port with unsent data can block for up to 30
  seconds.

### fade

| Key | Default | Meaning |
|---|---|---|
| `easing` | `"linear"` | easing for fades and pulses that don't name one: `linear` or `sine` |

### twinkle

Defaults for twinkle fields a command leaves out.

| Key | Default | Meaning |
|---|---|---|
| `min_level` | `100` | lowest level, 0–255 |
| `max_level` | `255` | highest level, 0–255, not below `min_level` |
| `min_duration` | `500` | shortest move in ms, 200–600 000 |
| `max_duration` | `2000` | longest move in ms, 200–600 000, not below `min_duration` |
| `easing` | `"sine"` | `linear` or `sine` |

### groups

Maps a group name to its channels:

```json
"groups": {
  "HouseLights": { "channels": [1, 2, 3, 4], "description": "Overhead lights" },
  "Spotlight":   { "channels": [10] }
}
```

`channels` lists DMX channels 1–512 without duplicates and may be empty. `description` is optional and
only shown by `--check-config`. Names appear in MQTT topics, so they can't be empty or contain `/`,
`+` or `#`. Groups may share channels.

### logging

| Key | Default | Meaning |
|---|---|---|
| `level` | `"info"` | `error`, `warn`, `info`, `debug` or `trace` |
| `format` | `"text"` | `text`, or `json` for log collectors |

Logs go to standard output. The `RUST_LOG` environment variable, if set, overrides `level` with a
[tracing filter](https://docs.rs/tracing-subscriber/latest/tracing_subscriber/filter/struct.EnvFilter.html),
for example `RUST_LOG=mqttdmx=debug`.
