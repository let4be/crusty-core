#!/bin/bash
set -e

rustup component add rustfmt --toolchain nightly
rustup component add clippy --toolchain nightly
cargo install cargo-release
pre-commit install
echo "Ready to go..."
