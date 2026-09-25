# Deployment

mqttdmx is a single binary with no runtime dependencies beyond the C library. You can run it under
systemd or in Docker.

## systemd

[`packaging/mqttdmx.service`](../packaging/mqttdmx.service) runs the binary as an unprivileged user
in the `dialout` group, with a read-only view of the filesystem.

```bash
cargo build --release
sudo install -Dm755 target/release/mqttdmx /usr/local/bin/mqttdmx
sudo useradd --system --no-create-home --shell /usr/sbin/nologin --groups dialout mqttdmx
sudo install -Dm644 your-config.json /etc/mqttdmx/config.json
mqttdmx --config /etc/mqttdmx/config.json --check-config
sudo install -Dm644 packaging/mqttdmx.service /etc/systemd/system/mqttdmx.service
sudo systemctl daemon-reload
sudo systemctl enable --now mqttdmx
```

The unit only passes `--config`, so the config file must set `output.driver` and, for the Enttec
driver, `output.device`.

The unit has no `DeviceAllow=` list. systemd resolves a device class like `char-ttyUSB` only when the
service starts, and if the USB serial driver hasn't been loaded by then (nothing plugged in since
boot), an interface plugged in later would be blocked until the service restarts. Access is left to
the `dialout` group instead.

`Restart=on-failure` restarts mqttdmx if it exits with an error. In practice that means a bad config
(which will fail again until fixed) or an internal fault; a missing interface or broker doesn't make
it exit.

## Docker

```bash
docker build -t mqttdmx .
docker run -d --restart always --name mqttdmx \
  -v /dev:/dev:ro \
  --device-cgroup-rule='c 188:* rmw' \
  -v /path/to/lights.json:/config/mqttdmx.json:ro \
  mqttdmx \
  mqttdmx --config /config/mqttdmx.json --server 192.168.1.10 \
    --driver enttec-usb-dmx-pro --device auto
```

A container normally only sees the devices that existed when it started, and `--device
/dev/ttyUSB0` can't follow an interface that comes back as `/dev/ttyUSB1` after a replug. Sharing the
host's `/dev` and allowing all USB serial devices (character major 188) lets mqttdmx find it again.
The mount is read-only, but that doesn't stop writing to the device itself.

The image runs as user `mqttdmx` in group `dialout`, which is group id 20 on Debian and Ubuntu. If
your host uses a different id for `dialout`, add `--group-add <id>`.

There is no Docker health check because mqttdmx has no HTTP endpoint; watch `dmx/status/online` and
`dmx/status/health` instead.

## Stopping and restarting

On SIGTERM (which `systemctl stop` and `docker stop` send) mqttdmx sends a final frame, publishes
`offline`, and exits. If the broker doesn't respond, it gives up waiting after about two seconds. The
interface keeps showing the last frame while mqttdmx is stopped.

The config file is only read at startup, so changing groups needs a restart. After a restart every
channel is back at `startup_level`, so restarting while the lights are in use will change what's on.

## Checking real hardware

The tests use a simulated interface. When installing or replacing an interface, check these on the
real thing:

1. `mqttdmx --list-devices` marks the interface as an Enttec DMX USB Pro. If it doesn't (a compatible
   interface from another maker), use its `/dev/serial/by-id/` path rather than `auto`.
2. After starting, `dmx/status/health` shows `connected` and a firmware version. No firmware version,
   plus a warning in the log, means the interface didn't answer the query; it still gets frames.
3. Setting a group to 255 and back to 0 changes the lights you expect.
4. Unplugging the USB cable shows `disconnected` within a second or so. Note what the lights do; with
   no signal, the dimmers fall back to their own loss-of-signal behaviour.
5. Plugging it back in returns to `connected` within about five seconds, with the lights at the levels
   mqttdmx was tracking, including anything sent while it was unplugged.
6. Booting with the interface unplugged and plugging it in afterwards reaches `connected` without
   restarting mqttdmx.
7. Stopping and restarting the broker leaves the lights alone, and commands work again once
   `online` is back.
8. A 10-second fade from 0 to 255, once with `"easing": "linear"` and once with `"easing": "sine"`,
   looks acceptable at the low end. Set `fade.easing` to whichever suits your dimmers.
