#!/bin/bash
set -euo pipefail

CHIP="STM32G474RETx"
PROBE_A="${PROBE_SERIAL_A:?Set PROBE_SERIAL_A secret}"
PROBE_B="${PROBE_SERIAL_B:?Set PROBE_SERIAL_B secret}"
TIMEOUT_SECS=30

SINK_ELF="usbpd-g474re-sink"
SOURCE_ELF="usbpd-g474re-source"

# Sanity check: verify probes are connected
probe-rs info --probe "$PROBE_A"
probe-rs info --probe "$PROBE_B"

# Flash sink to board A
probe-rs download --chip "$CHIP" --probe "$PROBE_A" "$SINK_ELF"

# Flash source to board B
probe-rs download --chip "$CHIP" --probe "$PROBE_B" "$SOURCE_ELF"

# Reset both boards simultaneously
probe-rs reset --chip "$CHIP" --probe "$PROBE_A" &
probe-rs reset --chip "$CHIP" --probe "$PROBE_B" &
wait

# Capture RTT from source board, timeout after TIMEOUT_SECS
LOG_FILE=$(mktemp)
timeout "$TIMEOUT_SECS" probe-rs attach --chip "$CHIP" --probe "$PROBE_B" \
  "$SOURCE_ELF" --target-output-file defmt="$LOG_FILE" || true

# Verify success markers
PASS=false
if grep -q "CI:PASS" "$LOG_FILE"; then
  echo "CI:PASS found - contract established"
  PASS=true
fi

if grep -q "CI:FAIL" "$LOG_FILE"; then
  echo "CI:FAIL found in log"
  PASS=false
fi

if grep -q "panic" "$LOG_FILE"; then
  echo "Panic detected in log"
  PASS=false
fi

if [ "$PASS" = true ]; then
  echo "Hardware test PASSED"
  exit 0
else
  echo "Hardware test FAILED"
  echo "--- RTT log ---"
  cat "$LOG_FILE"
  echo "--- end log ---"
  exit 1
fi
