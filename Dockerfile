FROM rust:1.95-bookworm AS builder

WORKDIR /build

# Copy manifests first for dependency caching
COPY Cargo.toml Cargo.lock ./

# Create a dummy source to pre-compile dependencies
RUN mkdir -p src \
    && echo 'fn main() {}' > src/main.rs \
    && touch src/lib.rs \
    && cargo build --release \
    && rm -rf src

# Copy the real source and rebuild (only the crate itself recompiles)
COPY src ./src
RUN touch src/main.rs src/lib.rs && cargo build --release

FROM debian:bookworm-slim

# An unprivileged user in the dialout group, which owns USB serial devices.
RUN useradd --system --no-create-home --shell /usr/sbin/nologin --groups dialout mqttdmx
COPY --from=builder /build/target/release/mqttdmx /usr/local/bin/mqttdmx
USER mqttdmx

# Give the container the DMX interface in a way that survives replugging:
#   -v /dev:/dev:ro --device-cgroup-rule='c 188:* rmw'
# (188 is the USB serial device major number) and --device auto or a
# /dev/serial/by-id path. See docs/deployment.md.
CMD ["mqttdmx", "--config", "/config/mqttdmx.json"]
