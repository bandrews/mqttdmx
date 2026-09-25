# Troubleshooting

Start with the status topics and the log:

```bash
mosquitto_sub -h <broker> -v -t 'dmx/status/#'
journalctl -u mqttdmx -n 100          # or: docker logs --tail 100 mqttdmx
```

## mqttdmx isn't online

If `dmx/status/online` has never appeared, mqttdmx has never reached this broker, or it is using a
different base topic. Check `--server`, `--port` and `mqtt.topic`.

If it says `offline`, mqttdmx has stopped or lost its connection. The log will say which:

- `MQTT connection to <host>:<port> failed: ...` means it can't reach the broker. It keeps retrying,
  at most every 10 s, and repeats the warning every tenth attempt. The lights keep their current state
  meanwhile.
- `MQTT connection to <host>:<port> failed: ... payload size limit exceeded`, straight after each
  "connected" line, means a retained message over 1 MiB is sitting on one of the command topics, and
  the broker sends it on every connection. Find it with
  `mosquitto_sub -v -t 'dmx/#' --retained-only -W 2` and clear it with
  `mosquitto_pub -r -n -t '<topic>'`.
- `cannot read config file ...`, `invalid config file: ...` or `Configuration is invalid:` at startup
  means mqttdmx exited without starting. The first two show one problem at a time; the last lists
  all the remaining ones. `mqttdmx --config <file> --check-config` runs the same checks.

## The interface shows as disconnected

`output.error` in `dmx/status/health`, and the matching log line, give the reason:

| Error | Meaning |
|---|---|
| `no Enttec DMX USB Pro found` | `--device auto` found no matching device. Check the cable and `mqttdmx --list-devices`. In Docker, `/dev` must be shared as described in [deployment.md](deployment.md#docker). Interfaces from other makers need an explicit path. |
| `found several Enttec DMX USB Pro interfaces` | Set `--device` to one of the paths listed. |
| `cannot open <path>: No such file or directory` | The path doesn't exist. `/dev/serial/by-id/...` names survive replugging; `/dev/ttyUSBn` numbers may not. |
| `cannot open <path>: Permission denied` | The user isn't in the `dialout` group, or in Docker the group id doesn't match the host's. |
| `cannot open <path>: Unable to acquire exclusive lock on serial port` | Another program has the device open, such as a Node.js serial application. Stop it. |
| `cannot open <path>: Device or resource busy` | Another mqttdmx has the device open. |
| `Broken pipe` or `I/O error` | The interface was unplugged. mqttdmx reopens it when it returns. |
| `the interface has stopped accepting data` | The interface is plugged in but not taking frames, probably hung. mqttdmx resumes if it recovers; otherwise unplug it and plug it back in. |

A warning that the interface "did not answer the widget parameters request" is not an error: frames
are still sent. It usually means the device isn't a genuine DMX USB Pro, which is worth confirming.

## A command does nothing

- Look for `rejected command` in the log. The line gives the reason and the payload.
  [commands.md](commands.md#what-gets-rejected) lists what is rejected.
- `ignoring retained command` means the broker replayed a command that was once published with the
  retain flag. It was applied when first sent. Clear the stored copy with `mosquitto_pub -r -n -t
  '<topic>'` and stop the sender using the retain flag.
- No log line at all means the message went to a topic mqttdmx doesn't subscribe to. Single channels
  are `dmx/channel/<n>`, not `dmx/<n>`.
- If the command was accepted but nothing changed, check the group's channels with `--check-config`
  and check that `output.state` is `connected`.

## Group levels look wrong

`dmx/status/groups` shows the level each group was last set to, not the level showing right now, so
it jumps to a fade's target when the fade starts. A group whose channels are at different levels,
which happens when an overlapping group or a single channel was changed, shows `null`.

## Slow fades look steppy at low levels

DMX has 256 steps, and at the bottom of the range each step is a large relative change in brightness,
especially on LED dimmers. A long fade near zero can show individual steps. `"easing": "sine"` spends
less time at the extremes, which helps a little. Some dimmers have their own smoothing or dimming-curve
setting, which usually helps more.
