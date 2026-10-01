#!/bin/bash
# Firmware CI: format-check, clippy and release build for one example crate.
# Invoked per matrix entry from ci.yml with:
#   firmware.sh <dir> [<features>] [<source-binary>] [<artifact-binary>]
# Empty positional args (or their absence) skip the corresponding step.
set -euo pipefail

dir="${1:?usage: firmware.sh <dir> [features] [source-binary] [artifact-binary]}"
features="${2:-}"
source_binary="${3:-}"
artifact_binary="${4:-}"

pushd "$dir"

# rustfmt unstable options (rustfmt.toml: group_imports, imports_granularity)
# only exist on nightly.
cargo +nightly fmt --check

# Both feature variants const-evaluate cfg!(feature = "role-initiator"), so
# clippy runs on default and each role feature set.
cargo clippy
if [ -n "$features" ]; then
    cargo clippy --features "$features"
fi

cargo build --release --features "$features"

if [ -n "$artifact_binary" ]; then
    cp "target/thumbv7em-none-eabihf/release/$source_binary" \
        "target/thumbv7em-none-eabihf/release/$artifact_binary"
fi

popd