//! The usage text `cargo xtask help` prints; its tests in `cli/tests.rs`
//! hold it to the parser.

/// The `help` text.
pub const USAGE: &str = "\
cargo xtask <command> [options]

commands:
  sim [options]
      Launch a full SITL session, print its ready URL, supervise it, and
      tear it down on ctrl-c.

      --fc <name>          FC backend: aviate-gz (alias aviate, default),
                           aviate-jsbsim, px4-gz (alias px4), px4-jsbsim,
                           px4-xplane, aviate-xplane, ardupilot-sitl (alias
                           ardupilot), or betaflight-sitl (alias
                           betaflight); px4-gz takes a vehicle model
                           as px4-gz:<model>: x500_gimbal (default) or
                           rc_cessna; px4-jsbsim as px4-jsbsim:<model>:
                           quad (default) or plane
      --profile <profile>  physical | simulation (default) | oracle-only
      --port <port>        host WebTransport port (default: 4433)
      --viewer-port <port> static viewer port (default: 8080)
      --instance <n>       simulator instance (default: 0); another
                           instance shifts the simulator and FC ports and
                           runs beside instance 0 on other ports
      --open               open the ready URL (default)
      --no-open            do not open the ready URL
      --lan                serve the session to the local network; requires a join code
      --join-token-file <path> absolute path to the host's local join-code file
      --ready-timeout <s>  how long each simulator and flight-controller
                           stage may take to come up (default: 180)
      --camera <camera>    none (default) | maplibre: render the vehicle's
                           cameras from the map world model at its
                           simulator truth, on port 46000 + instance,
                           instead of the backend's own camera

  fleet --vehicle <backend>[:<model>] [--vehicle ...] [options]
      Launch one session per vehicle, each with its own simulator,
      host, and viewer, spawned on one airfield: a fixed-wing on the
      runway threshold facing down the runway, every other vehicle on
      the grass beside it, 40 m apart. Writes
      target/xtask-fleet/fleet.json and serves the first vehicle's
      clients/web/session.json; supervises every session and tears all
      of them down on ctrl-c.

      --vehicle <spec>     a backend with an optional model, as for
                           sim --fc (repeat once per vehicle)
      --profile <profile>  physical | simulation (default) | oracle-only
      --port <port>        first host port (default: 4433); vehicle k
                           gets port + k
      --viewer-port <port> first viewer port (default: 8080); vehicle k
                           gets viewer-port + k
      --instance-base <n>  first simulator instance (default: 0);
                           vehicle k gets instance-base + k
      --site <site>        the airfield: n51 (default, Solberg-Hunterdon,
                           runway 04), zurich (PX4's default home), or
                           latitude_deg,longitude_deg,elevation_m,heading_deg
                           for a threshold, its ground elevation (m
                           MSL), and the runway's true course
      --ready-timeout <s>  as for sim, per vehicle (default: 180)
      --camera <camera>    as for sim, for every vehicle (default: none)
      --open               open every vehicle's viewer URL
      --no-open            open none (default)

  mission [options]
      Launch a mission fleet from a fleet file: one simulated multicopter
      host per aircraft, one coordinator per section over its aircraft,
      and a command coordinator over the sections. Every host admits a
      mission approver only with target/xtask-mission/approver.token.
      Places the file's camera fault on the aircraft's planned line,
      writes target/xtask-mission/fleet.json for the App, prints the App
      and agent commands, and tears every host down on ctrl-c.

      --plan <path>        fleet file (default:
                           sim/missions/princeton-landmarks.json)
      --decision <spec>    decision provider the aircraft judge their
                           pictures with, as PILOTAGE_DECISION takes it
                           (default: none: pictures are counted, not
                           judged)
      --decision-every <s> seconds between picture assessments
      --no-fault           leave the fleet file's camera fault out
      --ready-timeout <s>  how long each host may take to listen
                           (default: 60)

  reset [options]
      Reset the running simulation world and restart its FC.

      --fc <name>          FC backend (default: aviate)
      --instance <n>       simulator instance (default: 0); px4-gz
                           resets only that instance, and only while an
                           xtask supervisor runs it

  handshake [options]
      Produce the Alia X-Plane runtime handshake document and print its
      path, without launching a session. For harnesses that run the
      flight controller themselves; X-Plane must be running with the
      trial plugin loaded.

      --out-dir <dir>      directory to write into (default:
                           target/xtask-sim)

  affected --base <ref>
      Classify which CI matrix halves the diff base...HEAD can reach,
      printing one key=value line per answer with reason lines above.
      Fails open: an unclassifiable change answers everything=true.

  guards
      Discover every scripts/check-<name>.sh with its
      scripts/test-check-<name>.sh self-test, run each pair, and name
      every failing guard.

  serve-viewer --dir <dir> --port <port> [--bind <addr>]
      Serve <dir> statically to the browser viewer with caching disabled,
      on 127.0.0.1 unless --bind says otherwise; port 0 picks a free port
      and prints it.

  build-jsbsim
      Clone JSBSim v1.3.3 into $JSBSIM_DIR (default: ~/jsbsim) when it
      is absent, then build and install its static library into
      $JSBSIM_DIR/install with CMake. The JSBSim sidecar links it.

  build-sim-terrain
      Fetch AWS Terrain Tiles around N51 into target/sim-terrain/n51.mbtiles,
      the ground px4-jsbsim and aviate-jsbsim sessions stand their vehicles on.

  help
      Print this help text.";
