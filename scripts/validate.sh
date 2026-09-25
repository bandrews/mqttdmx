#!/usr/bin/env bash
# ABOUTME: One-command validation gate for mqttdmx: formatting, lints, release build and every test.
# ABOUTME: Default runs Lane A (Docker/Linux); --native runs Lane B on this host.
#
# Usage:
#   ./scripts/validate.sh            Lane A: everything inside a Linux container with mosquitto.
#   ./scripts/validate.sh --native   Lane B: the same steps on this host. Needs mosquitto and
#                                    mosquitto-clients installed for the broker tests.
#
# Exits non-zero on the first failing step.
set -euo pipefail

REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$REPO_ROOT"

run_native() {
    echo "== Lane B (native host) =="
    for tool in mosquitto mosquitto_pub mosquitto_sub; do
        if ! command -v "$tool" >/dev/null 2>&1; then
            echo "ERROR: $tool is not installed; the broker tests need it." >&2
            echo "       Install mosquitto and mosquitto-clients, or run Lane A." >&2
            exit 1
        fi
    done
    docker/run-validation.sh
    echo "== Lane B OK =="
}

run_docker() {
    echo "== Lane A (Docker, Linux) =="
    if ! command -v docker >/dev/null 2>&1; then
        echo "ERROR: docker is not installed or not on PATH." >&2
        echo "       Start Docker and re-run, or use --native for the host lane." >&2
        exit 1
    fi
    docker build -t mqttdmx-validate -f docker/validate.Dockerfile docker
    # The repo is mounted at /build; artifacts go to a named volume so a host
    # (macOS) target/ is never mixed with Linux build output.
    docker run --rm \
        -v "$REPO_ROOT":/build \
        -v mqttdmx_validate_target:/target \
        -v mqttdmx_validate_cargo:/usr/local/cargo/registry \
        -e CARGO_TARGET_DIR=/target \
        mqttdmx-validate
    echo "== Lane A OK =="
}

case "${1:-}" in
    --native) run_native ;;
    "")       run_docker ;;
    *)
        echo "usage: $0 [--native]" >&2
        exit 2
        ;;
esac
