#!/bin/bash
set -euo pipefail

export RUSTFLAGS="-D warnings"

for dir in usbpd usbpd-traits examples/embassy-nucleo-h563zi examples/embassy-stm32-g431cb;
do
    pushd $dir
    cargo +nightly fmt --check
    cargo clippy
    cargo build --release
    popd
done

for dir in usbpd usbpd-traits
do
    pushd $dir
    cargo clippy --features defmt
    popd
done

# Test building some feature combinations.
pushd usbpd
cargo build --features serde,log
cargo build --features serde,defmt
popd
