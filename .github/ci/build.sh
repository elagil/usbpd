#!/bin/bash
# All-in-one local entry point, mirroring the CI jobs: workspace members
# (host.sh) plus every firmware matrix target from .github/workflows/ci.yml.
set -euo pipefail

bash .github/ci/host.sh

for dir in examples/embassy-nucleo-h563zi examples/embassy-nucleo-g474re-sink examples/embassy-nucleo-g474re-sink-epr examples/embassy-nucleo-g474re-source;
do
    bash .github/ci/firmware.sh "$dir"
done

bash .github/ci/firmware.sh examples/embassy-nucleo-g474re-drp role-initiator usbpd-g474re-drp usbpd-g474re-drp-initiator
bash .github/ci/firmware.sh examples/embassy-nucleo-g474re-drp role-acceptor usbpd-g474re-drp usbpd-g474re-drp-acceptor