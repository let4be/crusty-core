#!/bin/bash
set -e

cargo release --manifest-path core/Cargo.toml "$@"
