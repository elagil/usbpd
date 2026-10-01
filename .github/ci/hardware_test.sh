#!/bin/bash
set -euo pipefail

CHIP="STM32G474RETx"
PROBE_A="${PROBE_SERIAL_A:?Set PROBE_SERIAL_A secret}"
PROBE_B="${PROBE_SERIAL_B:?Set PROBE_SERIAL_B secret}"
PROBE_VID_PID="${PROBE_VID_PID:-0483:374e}"
TIMEOUT_SECS="${TIMEOUT_SECS:-30}"

INITIATOR_ELF="usbpd-g474re-drp-initiator"
ACCEPTOR_ELF="usbpd-g474re-drp-acceptor"

# Ensure firmware binaries are executable.
chmod +x "$INITIATOR_ELF" "$ACCEPTOR_ELF"

# ST-LINK probes are exclusively opened. `probe-rs attach` flashes the given
# firmware and starts it, capturing RTT output from boot. Therefore attach to
# both boards first, each capturing to its own log file, and let them run.
# Probe stderr goes to separate files: flashing/probe failures would otherwise
# surface only as unspecific missing-marker failures below.
LOG_A=$(mktemp)
LOG_B=$(mktemp)
ERR_A=$(mktemp)
ERR_B=$(mktemp)
trap 'kill -9 "$PID_A" "$PID_B" 2>/dev/null; rm -f "$LOG_A" "$LOG_B" "$ERR_A" "$ERR_B"' EXIT

# Flash & run the initiator on board A.
probe-rs attach --chip "$CHIP" --probe "$PROBE_VID_PID:$PROBE_A" "$INITIATOR_ELF" \
  --connect-under-reset --target-output-file "defmt=$LOG_A" > /dev/null 2>"$ERR_A" &
PID_A=$!

# Flash & run the acceptor on board B.
probe-rs attach --chip "$CHIP" --probe "$PROBE_VID_PID:$PROBE_B" "$ACCEPTOR_ELF" \
  --connect-under-reset --target-output-file "defmt=$LOG_B" > /dev/null 2>"$ERR_B" &
PID_B=$!

# A probe-rs process that vanished before the kill below failed to flash or
# attach; do not mistake its empty log for a firmware failure.
fail_dead_process() {
  local name="$1" pid="$2" errfile="$3"
  if ! kill -0 "$pid" 2>/dev/null; then
    echo "probe-rs for $name exited before the end of the test:"
    cat "$errfile"
    exit 1
  fi
}

# Poll until both logs show the completed swap flow, or the deadline expires.
# A timeout falls through to the marker checks below, which then fail with
# their specific reasons. Note: unlike a plain `sleep $TIMEOUT_SECS`, this
# bounds the whole flash+run window, not just the capture after flashing.
deadline=$((SECONDS + TIMEOUT_SECS))
while [ "$SECONDS" -lt "$deadline" ]; do
  fail_dead_process "initiator" "$PID_A" "$ERR_A"
  fail_dead_process "acceptor" "$PID_B" "$ERR_B"
  if grep -q "CI:SWAP" "$LOG_A" && grep -q "CI:SWAP" "$LOG_B"; then
    break
  fi
  sleep 1
done

# Release the probes. SIGKILL: probe-rs attach does not exit on SIGTERM quickly
# enough, and lingering processes would block subsequent invocations.
kill -9 "$PID_A" "$PID_B" 2>/dev/null || true
sleep 2

# Verify board identities, so swapped probes/devices can not fake a pass, then
# verify success markers.
PASS=true
fail() {
  echo "$1"
  PASS=false
}

grep -q "initiator: true" "$LOG_A" || fail "Initiator board did not boot the initiator firmware"
grep -q "initiator: false" "$LOG_B" || fail "Acceptor board did not boot the acceptor firmware"

grep -q "CI:PASS" "$LOG_A" || fail "Initiator log missing CI:PASS"
grep -q "CI:PASS" "$LOG_B" || fail "Acceptor log missing CI:PASS"

# The power role swap must have completed on both sides (contract
# re-established after the swap: CI:SWAP).
grep -q "CI:SWAP" "$LOG_A" || fail "Initiator log missing CI:SWAP"
grep -q "CI:SWAP" "$LOG_B" || fail "Acceptor log missing CI:SWAP"

# A CI:FAIL only voids a board when the board never completed the full flow
# (CI:PASS + CI:SWAP): the ucpd_task loop restarts after an unrecoverable error
# and may recover, so a transient FAIL alongside a completed cycle is tolerated.
complete() {
  grep -q "CI:PASS" "$1" && grep -q "CI:SWAP" "$1"
}

grep -q "CI:FAIL" "$LOG_A" && ! complete "$LOG_A" && fail "Initiator reported CI:FAIL without completing the flow"
grep -q "CI:FAIL" "$LOG_B" && ! complete "$LOG_B" && fail "Acceptor reported CI:FAIL without completing the flow"

grep -qi "panic" "$LOG_A" "$LOG_B" && fail "Panic detected in logs"

if [ "$PASS" = true ]; then
  echo "Hardware test PASSED"
  echo "--- Initiator log ---"
  cat "$LOG_A"
  echo "--- Acceptor log ---"
  cat "$LOG_B"
  exit 0
else
  echo "Hardware test FAILED"
  echo "--- RTT log initiator ---"
  cat "$LOG_A"
  echo "--- RTT log acceptor ---"
  cat "$LOG_B"
  echo "--- probe-rs stderr ---"
  cat "$ERR_A" "$ERR_B"
  exit 1
fi