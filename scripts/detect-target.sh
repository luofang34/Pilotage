#!/usr/bin/env bash
# Detect a connected render target over USB CDC — detect, never ask.
#
# Prints any CDC serial devices present (macOS and Linux names). Detection is
# informational: the timing gate itself is the deterministic cargo test
# `timing::tests::budget_wcet_meets_the_frame_deadline`, and the detection
# outcome is recorded in
# docs/instruments/evidence-artifacts/timing/target-timing.txt. When a target
# IS connected, measure per-operation cycles on it and replace the
# conservative-bound model with a measured-usb-cdc one.
set -u

found=0
for dev in /dev/ttyACM* /dev/ttyUSB* /dev/cu.usbmodem* /dev/cu.usbserial*; do
  [ -e "$dev" ] || continue
  found=1
  echo "usb-cdc: $dev"
done

if [ "$found" -eq 0 ]; then
  echo "usb-cdc: no target connected; the conservative-bound timing model applies"
else
  echo "usb-cdc: target present — measure per-operation cycles on it and update"
  echo "         docs/instruments/evidence-artifacts/timing/target-timing.txt"
  echo "         (provenance: measured-usb-cdc, with the firmware identity)"
fi
exit 0
