#!/usr/bin/env bash
# ABOUTME: The validation steps shared by both lanes: fmt, clippy, release build, fuzz and all tests.
# ABOUTME: Broker tests start their own mosquitto on free ports, so no broker needs to be running.
set -euo pipefail
cd "$(dirname "${BASH_SOURCE[0]}")/.."

echo "==> cargo fmt --check"
cargo fmt --check

echo "==> cargo clippy --all-targets -- -D warnings"
cargo clippy --all-targets -- -D warnings

echo "==> cargo build --release (RUSTFLAGS=-D warnings)"
RUSTFLAGS="-D warnings" cargo build --release

# Property tests for the untrusted inputs, surfaced on their own so a
# panic-on-malformed-input regression is easy to spot.
echo "==> cargo test --test fuzz_test (proptest)"
cargo test --test fuzz_test

echo "==> cargo test (MQTTDMX_BROKER_TESTS=1)"
MQTTDMX_BROKER_TESTS=1 cargo test

echo "VALIDATION OK"
