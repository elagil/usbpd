#!/bin/bash
set -euo pipefail

CHIP="STM32G474RETx"
TIMEOUT_SECS="${TIMEOUT_SECS:-30}"

# Probe serial numbers:
#   A = initiator board
#   B = acceptor board
PROBE_A="0483:374e:002100223235511937333439"
PROBE_B="0483:374e:004A002C3235511837333439"

INITIATOR_ELF="usbpd-g474re-drp-initiator"
ACCEPTOR_ELF="usbpd-g474re-drp-acceptor"

# Ensure firmware binaries are executable.
chmod +x "$INITIATOR_ELF" "$ACCEPTOR_ELF"

# ST-LINK probes are exclusively opened.
# Attach to both boards first, each capturing RTT output to its own log, and let them run.
LOG_A=$(mktemp)
LOG_B=$(mktemp)
ERR_A=$(mktemp)
ERR_B=$(mktemp)
trap 'kill -9 "$PID_A" "$PID_B" 2>/dev/null || true; rm -f "$LOG_A" "$LOG_B" "$ERR_A" "$ERR_B"' EXIT

# Flash & run the initiator on probe A.
probe-rs attach --chip "$CHIP" --probe "$PROBE_A" "$INITIATOR_ELF" \
  --connect-under-reset --target-output-file "defmt=$LOG_A" > /dev/null 2>"$ERR_A" &
PID_A=$!

# Flash & run the acceptor on probe B.
probe-rs attach --chip "$CHIP" --probe "$PROBE_B" "$ACCEPTOR_ELF" \
  --connect-under-reset --target-output-file "defmt=$LOG_B" > /dev/null 2>"$ERR_B" &
PID_B=$!

# Block until the CI:SWAP marker is written to the log: returns within
# milliseconds of the marker appearing. Returns 1 when the probe-rs process
# dies first (tail --pid then ends the stream) or the deadline expires.
wait_marker() {
  grep -q "CI:SWAP" <(timeout "$TIMEOUT_SECS" tail -n +1 -f --pid="$2" "$1")
}
wait_marker "$LOG_A" "$PID_A" & W_A=$!
wait_marker "$LOG_B" "$PID_B" & W_B=$!
OK_A=true OK_B=true
wait "$W_A" || OK_A=false
wait "$W_B" || OK_B=false

# A probe-rs process that vanished before its markers appeared failed to flash
# or attach. Do not mistake its empty log for a firmware failure.
check_dead_process() {
  if [ "$4" = false ] && ! kill -0 "$2" 2>/dev/null; then
    echo "probe-rs for $1 exited before the end of the test:"
    cat "$3"
    exit 1
  fi
}
check_dead_process "initiator" "$PID_A" "$ERR_A" "$OK_A"
check_dead_process "acceptor" "$PID_B" "$ERR_B" "$OK_B"

# Release the probes: SIGTERM triggers probe-rs' graceful shutdown.
# SIGKILL as fallback so a hung process cannot block subsequent invocations.
kill "$PID_A" "$PID_B" 2>/dev/null || true
for _ in $(seq 1 50); do
  kill -0 "$PID_A" 2>/dev/null || kill -0 "$PID_B" 2>/dev/null || break
  sleep 0.1
done
kill -9 "$PID_A" "$PID_B" 2>/dev/null || true

# Verify board identities and success markers.
PASS=true
fail() {
  echo "$1"
  PASS=false
}

grep -q "initiator: true" "$LOG_A" || fail "Initiator board did not boot the initiator firmware"
grep -q "initiator: false" "$LOG_B" || fail "Acceptor board did not boot the acceptor firmware"

# The full swap flow must have completed on both sides. A transient CI:FAIL
# alongside a completed cycle is tolerated (the ucpd_task loop may restart and
# recover after an unrecoverable error), so CI:FAIL is not checked here.
markers() {
  grep -q "CI:PASS" "$2" || fail "$1 log missing CI:PASS"
  grep -q "CI:SWAP" "$2" || fail "$1 log missing CI:SWAP"
}
markers "initiator" "$LOG_A"
markers "acceptor" "$LOG_B"

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
