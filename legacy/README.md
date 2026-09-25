> The original Node.js prototype, kept for reference. It is not maintained; see the
> [top-level README](../README.md) for the current version.

# MQTT DMX Controller

A lightweight Node.js application for controlling DMX lighting fixtures over MQTT. Supports direct channel control, smooth fades, named groups, and twinkle effects.

## Features

- Control 512 DMX channels via MQTT
- **Named Groups**: Define groups of channels for easy control
- **Twinkle Effect**: Animated glow effect with configurable parameters
- Simple subtopic format for quick channel updates
- JSON message format for complex commands
- Smooth fade transitions with configurable duration
- Tracks all channel states in memory
- Graceful shutdown (turns off all lights)

## Installation

```bash
npm install
```

## Usage

Basic usage with default settings:

```bash
node index.js
```

With custom configuration:

```bash
node index.js --broker mqtt://192.168.1.25 --topic dmx --device /dev/ttyUSB0 --config ./config.json
```

### Command Line Options

- `--broker <url>`: MQTT broker URL (default: `mqtt://192.168.1.25`)
- `--topic <topic>`: MQTT topic prefix (default: `dmx`)
- `--device <path>`: DMX device path (default: `/dev/ttyUSB0`)
- `--driver <name>`: DMX driver (default: `enttec-open-usb-dmx`)
- `--config <path>`: Path to JSON config file for groups and settings
- `--help, -h`: Show help

### Available DMX Drivers

- `enttec-open-usb-dmx`: For Enttec Open DMX USB and compatible dongles (default)
- `enttec-usb-dmx-pro`: For Enttec USB DMX Pro
- `dmx4all`: For DMX4ALL devices
- `null`: For testing without hardware

## Configuration File

Create a JSON config file to define light groups and twinkle settings:

```json
{
  "groups": {
    "village": {
      "channels": [1, 2, 3, 4],
      "description": "Village houses"
    },
    "tree": {
      "channels": [10, 11, 12],
      "description": "Christmas tree lights"
    },
    "allLights": {
      "channels": [1, 2, 3, 4, 10, 11, 12],
      "description": "All lights"
    }
  },
  "twinkle": {
    "minBrightness": 100,
    "maxBrightness": 255,
    "minDuration": 500,
    "maxDuration": 2000,
    "variance": 0.3,
    "easing": "sine"
  }
}
```

### Twinkle Configuration Options

| Option | Default | Description |
|--------|---------|-------------|
| `minBrightness` | 100 | Minimum brightness during twinkle (0-255) |
| `maxBrightness` | 255 | Maximum brightness during twinkle (0-255) |
| `minDuration` | 500 | Minimum time for a twinkle cycle in ms |
| `maxDuration` | 2000 | Maximum time for a twinkle cycle in ms |
| `variance` | 0.3 | How much individual lights vary from each other (0-1) |
| `easing` | "sine" | Easing function: "linear", "sine", or "ease-in-out" |

## Message Formats

### Simple Subtopic Format

Set a single channel by publishing to `<topic>/<channel>`:

```bash
# Set channel 1 to 255
mosquitto_pub -h 192.168.1.25 -t dmx/1 -m 255

# Set channel 100 to 128
mosquitto_pub -h 192.168.1.25 -t dmx/100 -m 128
```

### Group Control

Control groups by publishing to `<topic>/group/<groupname>`:

```bash
# Set village group to 255
mosquitto_pub -h 192.168.1.25 -t dmx/group/village -m '{"value": 255}'

# Fade village group to 0 over 3 seconds
mosquitto_pub -h 192.168.1.25 -t dmx/group/village -m '{"value": 0, "fade": 3000}'

# Start twinkle on village group
mosquitto_pub -h 192.168.1.25 -t dmx/group/village -m '{"twinkle": true}'

# Start twinkle with custom settings
mosquitto_pub -h 192.168.1.25 -t dmx/group/village -m '{"twinkle": true, "minBrightness": 150, "maxBrightness": 255, "minDuration": 800, "maxDuration": 2500}'

# Stop twinkle on village group
mosquitto_pub -h 192.168.1.25 -t dmx/group/village -m '{"twinkle": false}'
```

### Query Topics

Request information from the controller:

```bash
# Get list of configured groups (publishes to dmx/status/groups)
mosquitto_pub -h 192.168.1.25 -t dmx/get/groups -m ''

# Get current state (publishes to dmx/status/state)
mosquitto_pub -h 192.168.1.25 -t dmx/get/state -m ''
```

### JSON Format

Publish JSON messages to the main topic for advanced control:

#### Set Single Channel

```json
{"channel": 1, "value": 255}
```

#### Set Multiple Channels

```json
{"channels": {"1": 255, "2": 128, "3": 64}}
```

#### Fade Single Channel

```json
{"channel": 1, "value": 255, "fade": 5000}
```

Fades channel 1 from current value to 255 over 5 seconds (5000ms).

#### Fade Multiple Channels

```json
{"channels": {"1": 255, "2": 0, "3": 128}, "fade": 3000}
```

Fades multiple channels simultaneously over 3 seconds.

#### Control Group via JSON

```json
{"group": "village", "value": 255}
{"group": "village", "value": 0, "fade": 3000}
{"group": "village", "twinkle": true}
{"group": "village", "twinkle": false}
```

#### Set Range to Same Value

```json
{"range": {"start": 1, "end": 17}, "value": 255}
```

Sets channels 1 through 10 to value 255.

#### Set Range with Individual Values

```json
{"range": {"start": 1, "end": 3}, "values": [255, 128, 64]}
```

Sets channel 1 to 255, channel 2 to 128, and channel 3 to 64. The values array must match the range size.

#### Fade Range

```json
{"range": {"start": 1, "end": 10}, "value": 0, "fade": 2000}
```

Fades channels 1 through 10 to 0 over 2 seconds.

### Example MQTT Commands

```bash
# Simple channel set
mosquitto_pub -h 192.168.1.25 -t dmx/1 -m 255

# JSON single channel
mosquitto_pub -h 192.168.1.25 -t dmx -m '{"channel": 1, "value": 255}'

# JSON fade
mosquitto_pub -h 192.168.1.25 -t dmx -m '{"channel": 1, "value": 255, "fade": 5000}'

# JSON multiple channels
mosquitto_pub -h 192.168.1.25 -t dmx -m '{"channels": {"1": 255, "2": 128, "3": 64}}'

# JSON multiple channels with fade
mosquitto_pub -h 192.168.1.25 -t dmx -m '{"channels": {"1": 255, "2": 0}, "fade": 2000}'

# JSON range - set channels 1-10 to 255
mosquitto_pub -h 192.168.1.25 -t dmx -m '{"range": {"start": 1, "end": 10}, "value": 255}'

# JSON range with values - set channels 5-7 to different values
mosquitto_pub -h 192.168.1.25 -t dmx -m '{"range": {"start": 5, "end": 7}, "values": [255, 128, 64]}'

# JSON range with fade - fade channels 1-20 to 0 over 3 seconds
mosquitto_pub -h 192.168.1.25 -t dmx -m '{"range": {"start": 1, "end": 20}, "value": 0, "fade": 3000}'

# Group control - set all village lights to 255
mosquitto_pub -h 192.168.1.25 -t dmx/group/village -m '{"value": 255}'

# Group control - start twinkle
mosquitto_pub -h 192.168.1.25 -t dmx/group/tree -m '{"twinkle": true}'

# Get group list
mosquitto_pub -h 192.168.1.25 -t dmx/get/groups -m ''
mosquitto_sub -h 192.168.1.25 -t dmx/status/groups
```

## Running as a Service

To run as a systemd service, create `/etc/systemd/system/mqttdmx.service`:

```ini
[Unit]
Description=MQTT DMX Controller
After=network.target

[Service]
Type=simple
User=enigma
WorkingDirectory=/repos/eleventhhourenigma/christmas/utilities/mqttdmx
ExecStart=/usr/bin/node /repos/eleventhhourenigma/christmas/utilities/mqttdmx/index.js --config /repos/eleventhhourenigma/christmas/utilities/mqttdmx/config.json
Restart=on-failure
RestartSec=5s

[Install]
WantedBy=multi-user.target
```

Then enable and start:

```bash
sudo systemctl enable mqttdmx
sudo systemctl start mqttdmx
sudo systemctl status mqttdmx
```

## Device Permissions

If you get permission errors accessing `/dev/ttyUSB0`, add your user to the `dialout` group:

```bash
sudo usermod -a -G dialout $USER
```

Then log out and log back in.

## Architecture

- **Channel State**: All 512 DMX channel values are tracked in memory
- **Fade Engine**: Updates at 60fps for smooth transitions
- **Twinkle Engine**: Updates at 30fps for efficient animation
- **Concurrent Fades**: Multiple channels can fade independently
- **Concurrent Twinkles**: Multiple groups can twinkle independently
- **Fade Cancellation**: New commands cancel any active fade on affected channels
- **Twinkle Cancellation**: Setting a static value on a twinkling group stops the twinkle

## Testing Without Hardware

For testing without DMX hardware:

```bash
node index.js --driver null --config ./config.json
```

This will run the application without sending data to a physical device.
