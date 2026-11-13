#!/bin/bash
# Host CI: format-check, clippy, build and test for the workspace members
# (usbpd, usbpd-traits). Runs without the examples/embassy submodule.
set -euo pipefail

# rustfmt unstable options (rustfmt.toml: group_imports, imports_granularity)
# only exist on nightly.
cargo +nightly fmt --check

cargo clippy --workspace
# Test-code lints need --all-targets; the plain clippy pass covers lib only.
RUSTFLAGS="-D warnings" cargo clippy --workspace --all-targets
# Also compiles everything a separate `cargo build` would.
cargo test --workspace

# Test building some feature combinations.
pushd usbpd
cargo build --features serde,log
cargo build --features serde,defmt
# The epr feature is default-on; keep the disabled build green.
cargo build --no-default-features
cargo test --no-default-features
popd