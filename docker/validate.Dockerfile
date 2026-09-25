# Linux validation image for mqttdmx (Lane A).
# Bundles the Rust toolchain and mosquitto with its command-line clients. The
# repo is mounted at /build at run time; CMD runs the validation script.
# Pinned to match mqttaudio's toolchain (1.95) so both projects run the same
# clippy lint set.
FROM rust:1.95-bookworm

RUN apt-get update && apt-get install -y --no-install-recommends \
        mosquitto mosquitto-clients \
    && rm -rf /var/lib/apt/lists/*

RUN rustup component add clippy rustfmt

WORKDIR /build
CMD ["/build/docker/run-validation.sh"]
