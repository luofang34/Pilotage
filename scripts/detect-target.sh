#!/usr/bin/env bash
# Detect a connected render target over USB CDC — detect, never ask.
#
# Scans the CDC serial device names (macOS and Linux) and, for each device,
# attempts an identity handshake: the port is configured raw at 115200 and any
# identity banner the firmware emits is read with a short timeout. Detection
# is informational: the envelope gate itself is the deterministic cargo test
# `timing::tests::budget_envelope_fits_the_display_derived_deadline`, and the
# detection outcome is recorded in
# docs/instruments/evidence-artifacts/timing/target-timing.txt.
#
# A measured timing model (provenance measured-usb-cdc) requires the firmware
# to report, and the artifact to record: firmware identity + build hash, MCU
# part, configured core clock, compiler and flags, cache/flash/memory state,
# and the committed raw measurement output.
set -u

found=0
for dev in /dev/ttyACM* /dev/ttyUSB* /dev/cu.usbmodem* /dev/cu.usbserial*; do
  [ -e "$dev" ] || continue
  found=1
  echo "usb-cdc: $dev"
  # Configure the port (either stty dialect) and read an identity banner.
  stty -f "$dev" 115200 raw -echo 2>/dev/null \
    || stty -F "$dev" 115200 raw -echo 2>/dev/null \
    || true
  banner=""
  if IFS= read -r -t 2 banner < "$dev" 2>/dev/null && [ -n "$banner" ]; then
    echo "  identity banner: $banner"
    echo "  -> measure per-operation cycles on this target and record a"
    echo "     measured-usb-cdc model (firmware/build hash, MCU, clock,"
    echo "     compiler flags, memory state, raw output) in the timing artifact"
  else
    echo "  no identity banner within 2s — unidentified device; it cannot"
    echo "  ground a measured timing model until its firmware reports an"
    echo "  identity over CDC"
  fi
done

if [ "$found" -eq 0 ]; then
  echo "usb-cdc: no target connected; the conservative-bound model and its"
  echo "         provisional cost envelope apply (not a WCET claim)"
fi
exit 0
