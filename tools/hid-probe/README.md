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

Create a candidate from the exact source-axis contract, capture, and baseline
profile:

```text
cargo run -p hid-probe -- characterize \
  --contract source-contract.json \
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
  --contract source-contract.json \
  --capture capture.json \
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
of center. The source-axis contract identifies a one-sided control such as a
throttle. The contract also gives the trusted minimum and maximum values.

## Timing evidence

The native `hid-probe` port records raw HID report callbacks. The browser
bridge records a state update only when `Gamepad.timestamp` changes. It does
not call a browser state update a raw HID report.

A native source-axis contract contains one raw report layout. The layout gives
the exact report size and one integer bit field for each axis. The contract
digest binds this layout. The native producer uses the layout to decode each
report. The analyzer decodes each report again. It rejects a sample when the
decoded axis bits differ from the recorded axis bits.

The Apple fixture sampler accepts injected samples for schema tests. It marks
these samples as `synthetic` and `injected_samples`. Promotion refuses this
source. An Apple producer can use `apple_hid` and `report_callbacks` only when
one IOHID open handle supplies the identity, connection state, timestamp, and
report bytes.

The result identifies the timing observation. It contains the median event
period, median absolute jitter, sample count, and confidence. A raw report
callback capture can contain an estimated dropped-report count. A browser
capture sets this count to `null`.

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
