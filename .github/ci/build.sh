#!/bin/bash
set -euo pipefail

for dir in . examples/embassy-nucleo-h563zi examples/embassy-stm32-g431cb;
do
    pushd $dir
    cargo fmt
    cargo clippy -- -D warnings
    cargo clippy --features defmt -- -D warnings
    cargo build --release
    popd
done
