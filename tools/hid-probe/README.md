# HID characterization

`hid-probe` measures device error. It does not tune control feel or vehicle
limits.

The tool uses one portable capture schema for native Apple HID, other native
HID ports, and the browser Gamepad API. Each capture records the sampling
source. Each capture also records the selected timestamp clock.

## Native flow

List the HID devices that the operating system can read:

```text
cargo run -p hid-probe -- list
```

Record an idle segment and one segment for each named control:

```text
cargo run -p hid-probe -- capture \
  --idle-seconds 5 \
  --movement-seconds 5 \
  --axes roll,pitch,throttle,yaw \
  --out capture.json
```

Keep all controls at rest during the idle segment. For each movement segment,
move only the named control. Move the positive direction first. Use the full
range. Move a centered control significantly on both sides of its center.
Return the control to its rest position.

Create a candidate from the exact capture bytes and the exact baseline profile
bytes:

```text
cargo run -p hid-probe -- characterize \
  --capture capture.json \
  --profile device.json \
  --out candidate.json
```

The `characterize` command prints the canonical candidate digest. Review the
candidate and its source capture. Copy the printed candidate digest and the
`source_capture_digest` only after this review. Promotion rejects any candidate
change after this review. Promote the candidate into a new profile file:

```text
cargo run -p hid-probe -- promote \
  --candidate candidate.json \
  --profile device.json \
  --confirm-source-digest <reviewed-sha256> \
  --confirm-candidate-digest <reviewed-candidate-sha256> \
  --out accepted-device.json
```

Promotion changes only source-axis mapping, direction, raw calibration, and a
measured device-noise dead zone. Promotion preserves the response curve and
all other profile content. The candidate schema has no vehicle-limit field.
Promotion rejects a centered axis when its evidence does not cover both sides
of center. A baseline endpoint center identifies a one-sided control such as a
throttle.

## Timing evidence

The Apple native bridge records raw HID report arrival time. The browser bridge
records a sample only when `Gamepad.timestamp` changes. Thus, a browser render
frame does not become a device report.

The result contains the median report period, median absolute jitter, and an
estimated dropped-report count. It also contains the sample count and
confidence.

## Platform dead zone

A raw HID capture records that the platform dead zone is not in the raw report.
A browser capture records `unknown` unless paired native and browser samples
measure the platform dead zone. A candidate with `unknown` or `observed`
platform dead-zone evidence cannot add a device dead zone.

## RadioMaster evidence

Do not qualify a RadioMaster axis order from an idle capture. Record a unique
movement for each named control. Reject a capture when two names select one
source axis. Reject a capture when cross-axis movement is more than the
declared coupling limit.

SIM / NOT FOR FLIGHT.
