# Design decisions

Notes on why mqttdmx 2.0 works the way it does. If you change one of these, update this file too.

## D1. Rust, on the same stack as mqttaudio

mqttdmx uses the same crates as its sister project [mqttaudio](https://github.com/bandrews/mqttaudio)
(tokio, rumqttc, serde, clap, tracing), so the two are configured, logged and packaged the same way.

The Node.js prototype in `legacy/` had problems that were hard to fix in place. The `dmx` npm package
it used crashed the process when the USB interface was unplugged and silently sent nothing if the
interface was missing at startup, and loose type coercion let values like `"abc"` or `300` reach the
hardware as something else.

## D2. Commands addressed by topic

Commands keep the prototype's style: the target is in the topic (`dmx/group/<name>`) and the payload
is small (`{"value": 255, "fade": 1000}`). This reads well in cue lists and scripts that store topic
and payload pairs. mqttaudio's `{"command": ...}` envelope wasn't adopted.

Changes from the prototype:

- Single channels moved from `dmx/<n>` to `dmx/channel/<n>`, to match groups and keep the base
  topic's first level free for `status`.
- Twinkle fields are snake_case (`min_level` rather than `minBrightness`), as in mqttaudio.
- The `channels` map and `values` array forms and the `dmx/get/*` query topics were removed. The
  retained status topics replace the queries.

The old spellings aren't accepted alongside the new ones.

## D3. Reject bad commands whole

Every command is parsed into a typed value before anything happens. An unknown field, an out-of-range
or fractional level, or conflicting actions reject the whole message with a WARN log line. This is
stricter than mqttaudio, which ignores unknown fields. With lighting, ignoring a misspelt `fade`
turns a fade into a snap, and clamping an out-of-range level would hide a mistake rather than report
it.

## D4. Ignore replayed retained commands

A retained command is stored by the broker and delivered again every time mqttdmx subscribes, which,
with a persistent broker, includes every reboot. mqttdmx skips any command delivered with the retain
flag set, which is how the broker marks those stored copies. A retained command published while
mqttdmx is connected arrives without the flag and is applied normally.

## D5. One render thread, one motion per channel

A single thread owns the lighting engine and the interface. It applies commands as they arrive and,
once per frame (25 ms at 40 fps), works out every channel's level as a floating-point value and sends
a frame, rounded to 8 bits. If the output is slow and a frame is late, commands already waiting are
still applied before it.

Each channel is doing exactly one thing: holding, fading, pulsing or twinkling. A command replaces
what its channels were doing, starting from the level they are showing. This avoids three problems
the prototype had: a timer shared by all the channels in a fade, which froze the others when one was
changed; instant sets being overwritten by a running fade; and uneven fade steps from several timers
running at different rates.

## D6. Set level

Each channel remembers the level it was last set or faded to. Pulse and twinkle don't change it and
return to it when they end, so repeating a pulse part-way through doesn't leave the channel stuck at
an in-between level. `dmx/status/groups` reports these set levels, so a fade's target is visible as
soon as it starts, and a group whose channels differ reports `null`.

## D7. Linear fades by default

Fades are linear unless a command or `fade.easing` asks for `sine`. Which looks better depends on the
dimmers, so it's a setting. Host-side dimming curves and temporal dithering were left out: a
square-law curve makes the low end of an 8-bit channel coarser, and dithering at 40 Hz or less shows
as flicker.

## D8. Status in the daemon's own namespace

mqttdmx publishes three retained topics under `<topic>/status/` and nothing tied to a particular
application. The groups document is one message rather than one per group, and is limited to four
updates a second (the last is always sent), because a consumer may do real work for every message it
receives.

## D9. Recover at runtime, fail at startup

A bad config file stops mqttdmx before it touches the output. After that, nothing is fatal:

- A missing or unplugged interface is retried with backoff (0.25 s to 5 s) while levels and fades
  carry on in memory. An interface that stays plugged in but stops taking data is kept open and
  waited out, because closing a USB serial port with unsent data can block for up to 30 seconds.
- A lost broker is retried with backoff (0.5 s to 10 s). The backoff only resets after a connection
  has lasted 5 s, so a connection that fails immediately doesn't turn into a reconnect loop.

Exiting when the interface disappears would lose the current state, and Docker's `--device` option
can't follow an interface that comes back under a different `ttyUSB` number, so mqttdmx finds it
again itself.

Release builds use `panic = "abort"`. The parsers are fuzz-tested not to panic, but if something
does, the process exits and the service manager restarts it, rather than leaving a daemon that looks
alive with a dead thread.

## D10. Start at a known level, hold on stop

The first frame is `startup_level` on all 512 channels, with no blackout while the interface opens.
Setting it to 255, for example, means house lights come on when the system starts. On shutdown
mqttdmx sends one last frame and stops. The Enttec DMX USB Pro repeats its last frame while powered,
so the lights stay as they were through a restart; blacking out on every restart would be a bad
default in a room with people in it.

Levels aren't saved across restarts. Since interface and broker failures are handled without
restarting, the case that would need it is rare.

## D11. Only the Enttec DMX USB Pro and null drivers

The DMX USB Pro protocol is simple (label 6, "Output Only Send DMX Packet") and the interface keeps
sending the last frame on its own. The Open DMX USB needs the host to generate DMX timing and was
left out until it's needed. Art-Net and sACN would be the obvious next drivers.

## D12. Status publishing never blocks output

The render thread passes snapshots to the MQTT side through watch channels and never waits on the
network. The MQTT side only ever queues messages without blocking, and republishes the latest
snapshots each time it reconnects, so a broker outage can't hold up a fade.
