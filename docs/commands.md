# Commands and status topics

All examples use the default base topic `dmx`. Change it with `mqtt.topic` or `--topic`.

## Topics

| Topic | Direction | Payload |
|---|---|---|
| `dmx/group/<name>` | in | an action for a group from the config file |
| `dmx/channel/<n>` | in | an action for channel `n` (1–512) |
| `dmx` | in | a JSON object naming a target and an action |
| `dmx/status/online` | out, retained | `online` or `offline` |
| `dmx/status/health` | out, retained | JSON: version and interface state |
| `dmx/status/groups` | out, retained | JSON: each group's set level |

mqttdmx subscribes to `dmx`, `dmx/channel/+` and `dmx/group/+` only, so it never sees its own status
messages.

## Targets

On `dmx/group/<name>` and `dmx/channel/<n>` the target is in the topic. Group names are
case-sensitive. A group with no channels is allowed; commands to it are accepted and do nothing.

On the base topic `dmx`, the payload names the target with exactly one of:

| Key | Example |
|---|---|
| `group` | `"group": "HouseLights"` |
| `channel` | `"channel": 12` |
| `range` | `"range": {"start": 1, "end": 512}` (inclusive) |

## Actions

Each message carries one action. Levels are whole numbers 0–255; times are milliseconds.

### Set or fade

```json
{"value": 255}
{"value": 0, "fade": 3000}
{"value": 128, "fade": 10000, "easing": "sine"}
```

`fade` can be 0 (the same as leaving it out) up to 86 400 000 (24 hours). `easing` is `linear` or
`sine`; if you leave it out, the config's `fade.easing` is used, which is `linear` unless you change
it. The fade starts from the level the channel is showing when the command arrives.

On group and channel topics a bare number is the same as `{"value": n}`:
`mosquitto_pub -t dmx/channel/12 -m 255`.

### Pulse

```json
{"pulse": "slow"}
{"pulse": "fast"}
```

The channels rise to 255, hold, and fall back to their set level. `slow` is 1.5 s up, 2 s hold,
1.5 s down; `fast` is 0.5 s each. The rise and fall use `fade.easing`. Pulsing again part-way through
starts from the current level and still ends at the set level.

### Twinkle

```json
{"twinkle": true}
{"twinkle": true, "min_level": 150, "max_level": 255, "min_duration": 800, "max_duration": 2500}
{"twinkle": false}
{"twinkle": false, "fade": 1000}
```

Each channel moves to a random level between `min_level` and `max_level`, taking a random time
between `min_duration` and `max_duration`, then picks another. Channels move independently. Fields
you leave out come from the config's `twinkle` section, and the combined values must still have min
no greater than max. Durations must be between 200 ms and 10 minutes; the lower limit stops twinkle
from becoming a strobe. `easing` sets the shape of each move.

`{"twinkle": false}` returns twinkling channels to their set level, instantly or over `fade` ms (with
an optional `easing`). Channels in the target that aren't twinkling are not touched.

## How commands combine

Each channel has a set level (the last value it was set or faded to, starting at `startup_level`) and
is doing one thing at a time: holding, fading, pulsing or twinkling. A command replaces what the
channels it names are doing and leaves every other channel alone. So:

- If you fade a group and then fade a second, overlapping group, the shared channels follow the second
  fade and the rest finish the first.
- Setting a level cancels any fade, pulse or twinkle on those channels. A range 1–512 set to 0 turns
  everything off.
- Pulse and twinkle don't change the set level.

Commands are applied as they arrive and show up in the next output frame (every 25 ms at the default
frame rate). mqttdmx has no cue lists or delays; sequencing is up to whatever sends the commands.

## What gets rejected

A message is checked in full before anything changes. If any part is wrong, nothing happens and a
WARN line is logged with the reason and the payload. That covers:

- invalid JSON, `null`, or a string or array where a number or object was expected
- a level outside 0–255 or with a fraction
- a `fade` that is negative, over 24 hours, or not a number
- unknown fields (so a typo like `"fdae"` can't silently turn a fade into a snap)
- no action, or more than one (`value` together with `pulse`, for example)
- fields that don't belong to the action, such as `fade` with `pulse`
- no target or several targets on the base topic
- an unknown group, a channel outside 1–512, or a range with start after end
- twinkle settings outside the limits above

### Retained commands

Don't publish commands with the retain flag. The broker delivers such a message normally, so it
takes effect, but it also stores it and sends it again every time mqttdmx reconnects. mqttdmx ignores
those replays and logs a warning, because re-running an old cue after a network blip would be wrong.
Clear a stored message with `mosquitto_pub -r -n -t <topic>`.

## Status topics

All three are retained and republished whenever mqttdmx reconnects, so a client that subscribes
later, or a broker that restarts, gets the current state.

### `dmx/status/online`

`online` while connected. mqttdmx publishes `offline` when it stops. If it crashes, the broker
publishes `offline` on its behalf (the MQTT Last Will) as soon as the connection closes; if the
network drops instead, the broker notices within about 15 seconds.

### `dmx/status/health`

Published on connect and whenever the interface's state changes.

```json
{
  "version": "2.0.0",
  "output": {
    "driver": "enttec-usb-dmx-pro",
    "device": "/dev/ttyUSB0",
    "state": "connected",
    "firmware": "1.44",
    "error": null
  }
}
```

`output.state` is one of:

- `connected`: frames are reaching the interface.
- `disconnected`: they aren't. Either the device is closed and mqttdmx is retrying, or it is open but
  has stopped accepting data. `output.error` says which.
- `disabled`: the `null` driver is in use.

`output.device` is the device path while it is open, otherwise `null`. `output.firmware` is `null` if
the interface didn't answer the firmware query.

### `dmx/status/groups`

Each group's set level:

```json
{"HouseLights": 255, "Spotlight": 0, "AllLights": null, "Unused": null}
```

A group is `null` when its channels are at different levels, or when it has no channels. During a
fade the value is already the target, and pulse and twinkle don't change it. The document is sent at
most four times a second; the last change is always sent.
