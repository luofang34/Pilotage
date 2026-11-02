# JSBSim Rust engine review

Review date: 2026-11-02.

## Scope

This review starts from Pilotage PR #886 at `bd2a78713163448a1d89b1e37b387f95a9c0ff69`.
The Aviate source is main at `971fdd4cadd14789a974957b3b096146225fd337`.
The Rust dependency is `jsbsim 0.2.0`.
The compiler is Rust 1.99.1 on Linux x86-64.
The C++ reference is JSBSim 1.3.3 at `66f7fafc07fe13e7ede9db20873030fed2881713`.

## Changes from this review

- Set the Rust dependency and lockfile to `jsbsim 0.2.0`.
- State its Rust 1.99 requirement in the sidecar manifest.
- Refuse a zero, negative, or non-finite engine time step before model load.
- Make the flight comparison fail on each non-finite value.
  The old `f64::max` reduction could discard a NaN.
- Compare time, specific force, angular rate, dynamic pressure, and both airspeeds.
- Compare angles across the zero-heading boundary.
- Require a positive Rascal climb. An absolute height difference could count a descent.
- Test malformed XML, missing engine files, shared engine and system paths, and paths with spaces.
- Test landing and disarm at 1, 2, and 4 ms sensor periods at three sites.
  The sites include both hemispheres, high latitude, both sides of the date line,
  negative terrain height, and terrain at 3000 m.
- Compare the emitted HIL_SENSOR and HIL_GPS fields, timestamps, and sensor flags.
- Test the C++ data-root error and retain the build-without-C++ error test.

## Independent results

These checks were run for this review. They are not copied from the PR report.

| Check | Result |
| --- | --- |
| Default sidecar tests, with no C++ feature | 53 passed |
| Sidecar tests with `cpp`, including both 5000-step flights | 58 passed |
| Default and `cpp` Clippy, all targets, warnings denied | Passed |
| Sidecar format and documentation checks | Passed |
| Release build, default Rust engine | Passed |
| Release build with the C++ engine | Passed |
| Aviate main mission harness, locked release build | Passed |
| Aviate blocking missions against Rust 0.2.0 | 6 passed; 2 runs each; 84 phases passed |
| Aviate blocking missions against C++ 1.3.3 | 6 passed; 2 runs each; 84 phases passed |

The missions are `basic_flight`, `command_timeout`, `gnss_dropout`,
`hover_stability`, `numeric_fault_inject`, and `position_hold`.
The mission manifest requires two passes from at most three runs.
The harness stops after two passes. Each engine passed on the first two runs.

The default release binary was built without the `cpp` feature.
The Rust mission run used no `--jsbsim-root` option.
The C++ mission run used `--engine cpp --jsbsim-root DIR`.

The original position, velocity, and attitude limits remain 1 mm,
1 mm/s, and 0.1 mrad at each step.
A diagnostic run measured the largest specific-force difference at about
0.010251 m/s2 during the landing cases.
The largest angular-rate difference was about 0.000078484 rad/s.
The largest Rascal specific-force difference was about 0.003173 m/s2.
The new specific-force limit is 0.02 m/s2.
This is one default accelerometer noise standard deviation.
The angular-rate limit is 0.0001 rad/s.
The temporary diagnostic code is not part of the change.
These measurements do not establish agreement for all models or conditions.

The local `cargo test -p xtask --locked` could not fetch the private Airmass
revision. The failure occurred before compilation.
This result does not establish a workspace test pass.
Repository CI must check the workspace with its deploy credentials.

## Existing reports and separate failures

The original PR author reported six Aviate missions and 96 phases per engine.
That report used the earlier Rust dependency and a different Aviate revision.
This review does not treat the reported 96 phases as independently confirmed.
The local mission logs from this review contain 84 passed phases per engine.
The original PX4 mission claims were not rerun for this review.

Aviate PR #377 run `41377667765` failed in Release Scripts.
The action requested Rust 1.120.0, which the distribution server could not find.
The download returned HTTP 404 before compilation.
Its JSBSim mission job passed against the old Pilotage pin.
This toolchain failure is separate from a simulation regression.
The dependency update does not need to merge PR #377.

## Limits

The sidecar supports the bundled X500 and Rascal airframes.
The tests do not qualify arbitrary JSBSim aircraft, non-Earth planet files,
ice, moving ground, extreme flight envelopes, or other host platforms.
The comparison checks agreement with the C++ reference.
It does not validate the models against measured flight data.
