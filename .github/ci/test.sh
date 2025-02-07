#!/bin/bash
set -euo pipefail

for dir in .;
do
    pushd $dir
    cargo test
    popd
done
