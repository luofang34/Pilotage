# ADR-0007: Canonical input model and versioned device-profile registry

- Status: Accepted
- Date: 2026-07-05

## Context

Browser Gamepad API representations vary by device, platform, firmware, and browser.
Serious control arrangements combine a joystick, throttle quadrant, pedals, and a
gamepad. Binding raw browser axis/button indexes directly to simulator commands makes
profiles fragile, prevents reuse across devices, and couples the client to one
simulator's vocabulary.

## Decision

- Separate the pipeline stages: physical sampling → device identification →
  normalization → calibration → deadzone/saturation → response curve → logical
  binding → scoped simulator command → control frame.
- Define a **canonical logical input model** independent of Gamepad API indexes and
  independent of any vehicle type's control vocabulary.
- Maintain a **versioned registry of device profiles** with layered precedence:

  ```text
  built-in registry < organization registry < user profile
      < vehicle-specific profile < current-session override
  ```

- Support multi-device composition, dead zones, saturation, inversion, response
  curves, chords, mode shifts, and button-edge semantics.
- Perform latency-sensitive normalization and profile evaluation client-side (in the
  browser for v1), inside the portable input crate (ADR-0002), so native clients
  reuse it unchanged.
- Bind canonical logical inputs to a host-published control scope (ADR-0006); the
  control frame carries the profile revision in effect (ADR-0009).
- Emergency actions (deadman release, emergency stop) are protocol-level semantics
  with dedicated messages — never merely one more device binding.

## Consequences

- Device profiles need schema versions and migration support from day one.
- Calibration MUST be visible and testable before control is enabled; the client
  SHOULD display raw and normalized values side by side for diagnosis.
- Profile evaluation is deterministic and sans-IO, so recorded raw samples plus a
  profile revision reproduce the exact command stream in tests.
- The registry is data, not code: new devices ship as registry entries, not client
  releases (signed online registry updates are a backlog item).
