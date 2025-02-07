#!/bin/bash
set -euo pipefail

for dir in ./usbpd;
do
    pushd $dir
    cargo test
    popd
done
