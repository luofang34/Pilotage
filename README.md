# Pilotage

Engine-independent platform for low-latency remote control, supervision, simulation,
and training of maritime, aerial, and terrestrial vehicles.

> **⚠️ Work in progress — experimental.** Pilotage is early-stage, experimental
> software under active development, provided **as is** with **no warranty or
> guarantee of any kind**, express or implied — including, without limitation,
> fitness for a particular purpose, correctness, reliability, availability, or
> safety. Interfaces, wire formats, and behavior may change without notice.
>
> **SIM / NOT FOR FLIGHT.** Nothing here is certified, approved, or airworthy.
> Nothing may be used for operational control of a real vehicle or for any
> safety-critical purpose. Use at your own risk.

**Status:** early implementation. The core protocol/authority/input crates, a
WebTransport session host, and a Gazebo adapter are in place; a browser and a native
client drive a real Gazebo vehicle end-to-end (video, telemetry, and control). The
decision records under [docs/adr](docs/adr/README.md) are the authoritative design.

## Working demo

A diff-drive vehicle in Gazebo, driven from a browser or native client: the onboard
camera streams down as MJPEG over WebTransport, odometry streams down as telemetry,
and keyboard/gamepad control streams up — with channel-level authority (scoped leases
and fencing generations) enforced on every frame. A C++ gz-transport sidecar bridges
Gazebo to the Rust adapter; a deterministic headless adapter stands in when Gazebo is
not present. Runs over loopback or LAN.

## Run a PX4 SITL session

One command launches a PX4 + Gazebo software-in-the-loop session and opens the
browser viewer:

```sh
cargo xtask sim --fc px4-gz
```

The launcher builds what a fresh checkout is missing before it starts: the
release session host and the viewer's generated wasm runtime are built
automatically on the first run, and the C++ gz camera sidecar is built
best-effort (the session still flies without it — you just get no video).

You provide the pieces git does not vendor:

- A **PX4-Autopilot** SITL checkout, built once (`make px4_sitl`), at
  `../PX4-Autopilot` or the path in `PX4_DIR`. The launcher fails with the
  exact missing path when it is absent.
- **Gazebo Harmonic** (`gz`) on `PATH`.
- Toolchains for the auto-built artifacts: `wasm-bindgen-cli` 0.2.126 plus the
  `wasm32-unknown-unknown` target (viewer, required), and `protobuf` + the
  Gazebo dev libraries (camera sidecar, optional). Each build script prints its
  own install hint on failure.

For an ArduPilot SITL connection, set `PILOTAGE_ARDUPILOT_PROFILE=simulation`
and start the session host with `--adapter ardupilot`. Set
`PILOTAGE_ARDUPILOT_ADDR` to the local UDP endpoint, or use the default
`127.0.0.1:14550`. To enable MAVLink gimbal discovery, set
`PILOTAGE_ARDUPILOT_GIMBAL=1`. Discovery does not claim gimbal control.
The device reports which axes and actions it supports.
PX4 and ArduPilot also discover MAVLink cameras on the same link.
Each camera has an independent `vehicle.camera.<component>.<device>` scope.
Zoom controls require a current camera capability report. Zoom targets do
not replace the reported zoom setting. A sent command does not confirm
that the camera reached its target.
After a camera restarts or its boot clock wraps, restart the session host
to establish a new connection. Lower camera timestamps do not establish
a new camera identity.
For native takeoff, send `Takeoff` on `vehicle.motion` after the FC reports
armed, GUIDED mode, and on-ground state with valid position data. The target
is 2.5 m above the measured starting altitude. A climb input does not start
takeoff. The `flight_procedure` telemetry field reports progress separately
from action acceptance. A timeout does not confirm that vehicle motion stopped.

## Remote session access

The session host listens on `127.0.0.1` by default for all adapters.
Run the host on the computer connected to the device. To accept remote
clients, set `--bind` to a network interface address and supply
`--join-token-file` with an absolute path. The file must contain a shared
secret with 64 hexadecimal digits. Generate one on the host:

```sh
umask 077
openssl rand -hex 32 > "$HOME/pilotage-join-token"
cargo run --release -p pilotage-session-host -- \
  --adapter reference --bind 0.0.0.0 --port 4433 \
  --join-token-file "$HOME/pilotage-join-token"
```

Provide the secret to each operator through a trusted channel. Keep it out
of the public session file and URL. Clients must also verify the host's
certificate SHA-256 from its `LISTENING` line. In the browser viewer, enter
the secret in **Join code**, then select **Connect**. For the simulator
launcher, use `cargo xtask sim --lan --join-token-file /absolute/token-file`.
Sokoly-App accepts a local token file for each named session. The Apple
client and command-line probe do not accept a token and cannot join a
protected host.

The host checks the first client message before it sends capabilities,
telemetry, or video. A missing or invalid secret closes the connection.
The client stops automatic connection attempts after admission is rejected.
Admission does not grant a control lease. Connection recovery restores
observation only; the operator must request control again. Network delay
does not extend command deadlines.

## DJI USB bench connection on macOS

Select one RC-N1 attachment and a new capture file. The capture directory
must exist. The host refuses to overwrite a capture.
The host uses the session access settings above.

For the upper USB port, set `PILOTAGE_DJI_USB=auto` to select the only matching
DJI C5 device in the macOS I/O Registry. The device must already expose the
iAP and `com.dji.logiclink` interfaces. Discovery only reads registry properties.
The host stops if no device or more than one device matches. It checks the
full USB descriptors when it opens the selected device.
The host does not run the upper-port bootstrap procedure or change the USB
role or device configuration. Connecting the cable alone does not ensure
that the required profile is present.

```sh
PILOTAGE_DJI_USB=auto \
PILOTAGE_DJI_CAPTURE=/tmp/dji-session-01.jsonl \
cargo run --release -p pilotage-session-host --no-default-features -- --adapter dji
```

To select an explicit attachment for diagnostics, use
`PILOTAGE_DJI_USB_REGISTRY_ID` instead of `PILOTAGE_DJI_USB`. Use the current
device registry ID as a nonzero decimal value or a hexadecimal value with
a `0x` prefix. The registry ID can change when the cable is disconnected.

The upper interface supplies telemetry and H.264 video from one connection.
Video decoding requires `ffmpeg` on `PATH`. Set `PILOTAGE_DJI_FFMPEG` to an
absolute executable path to select another installation. A decoder that
cannot start prevents host startup. Camera capture time and calibration
remain unknown.

For the lower USB CDC port, select its callout path instead:

```sh
PILOTAGE_DJI_SERIAL=/dev/cu.usbmodemDEVICE \
PILOTAGE_DJI_CAPTURE=/tmp/dji-session-01.jsonl \
cargo run --release -p pilotage-session-host --no-default-features -- --adapter dji
```

Set only one of `PILOTAGE_DJI_SERIAL`, `PILOTAGE_DJI_USB_REGISTRY_ID`, or
`PILOTAGE_DJI_USB`.
An invalid attachment or an unsupported platform fails startup. The host
does not select another device or transport.

This adapter exposes a DJI Mini 3 experimental object with gimbal control.
Continuous flight and positioning control are not implemented.
Keep the propellers removed for bench tests.
The default `observed` control mode requires fresh gimbal observations before
it sends a pitch command. Pitch targets are limited to -90 through 0 degrees.
The host limits target planning to 30 degrees per second. This is not a
measured device speed limit. Recenter continues while control input and
feedback remain valid. The host does not repeat an unchanged target after
a full write.
The gimbal scope supports pitch only. A client can route camera pan to
vehicle yaw when the separate flight velocity capability is enabled.
An RC version response alone does not establish the aircraft route.

Set `PILOTAGE_DJI_EXPERIMENTAL_FLIGHT=1` to enable Takeoff and Land on the
upper USB interface in `observed` mode. The default disables these actions.
The adapter requires a verified Mini 3 and fresh aircraft reports. Takeoff
also requires stopped motors, no reported flight, and at least 25 percent
battery charge. Land does not require that battery threshold.
The App needs a control lease and a valid command window for each action.
The aircraft checks its other start conditions. Device acceptance and motor
reports do not prove completed takeoff or landing. An uncertain outcome does
not cause an automatic retry. Use the original remote controller for flight
control. Verify remote takeover and disconnect behavior before a flight test.
After a Takeoff write, another Takeoff requires an explicit adapter restart.
A restart does not stop an aircraft procedure.

Set `PILOTAGE_DJI_VIRTUAL_STICK=1` to expose experimental BODY-FRD velocity
control on the upper USB interface. This setting is separate from Takeoff
and Land. A new control lease must supply neutral input. Mode entry requires
fresh airborne GPS-attitude reports and independent mode confirmation.
Fresh input retains the stream at up to 20 writes per second. The limits are
1 m/s horizontal speed, 0.5 m/s vertical speed, and 30 degrees/s yaw rate.
Input loss requests one zero-velocity write and mode release. Native flight
modes and remote mode exit stop the stream without automatic reacquisition.
A new lease requires confirmed release. Release virtual-stick control before
requesting native Takeoff or Land. These checks do not verify aircraft motion
or remote takeover behavior. Keep the original controller available.

The bottom RC-N1 CDC interface can forward pitch commands without aircraft
pose feedback. For this bench connection, set
`PILOTAGE_DJI_CONTROL_MODE=bench-initial-horizontal`. Before you start the
host, confirm that the camera is horizontal. This mode uses that initial
target without reporting it as a measured pose. Pitch targets are limited
to -90 through 0 degrees. Target changes are limited to 5 degrees per second.
Recenter moves the target toward zero at the same limit. It does not send
an immediate zero target. A pending command expires after 100 milliseconds.

The one-way mode cannot confirm physical motion or a physical stop. Neutral
input stops changes to the target. It cannot cancel a command that was sent.
A control-session loss prevents further commands until the host is restarted.
The adapter reports that a physical link-loss action is not available.
Before each restart, confirm that the camera is horizontal again.

The capture contains received and transmitted bytes with host monotonic times.
A lost connection or a capture failure stops control. Restart the host to open
a new connection and capture. The adapter does not reconnect automatically.

## What the architecture protects

- **Portable sans-IO core.** All protocol, authority, input, timing, and replay logic
  lives in portable crates shared by the browser and native clients. Platform code
  stays thin; domain logic is never duplicated.
- **Engine independence.** Gazebo is the first adapter, not the platform boundary.
  Unreal, Unity, deterministic headless trainers, and real-vehicle gateways are peer
  implementations of the same adapter contract.
- **No mandatory center.** Central services may provide identity, rendezvous, or relay,
  but a session must never require a centrally operated simulator or vehicle fleet.
- **Authority correctness.** Explicit state machines govern handover, emergency
  override, and link loss; every control frame carries a scope and a fencing
  generation, so stale controllers are invalidated immediately.

## Repository layout

| Path | Contents |
|---|---|
| `docs/adr/` | Architecture decision records (authoritative) |
| `docs/` | Architecture overview and supporting design docs |
| `schemas/` | Protobuf wire-schema sources — the protocol's source of truth |
| `crates/` | Portable sans-IO core crates |
| `hosts/` | Session-host binary |
| `adapters/` | Vehicle adapters: Gazebo (with its C++ bridge) and the deterministic reference |
| `sim/` | Gazebo demo worlds |
| `clients/` | Browser demo viewer (native viewer lives under `tools/`) |
| `tools/` | Native viewer/probe and the HID device probe |
| `services/` | Identity, rendezvous/signaling, and other optional central services |

Start with [docs/architecture.md](docs/architecture.md), then the
[ADR index](docs/adr/README.md).
