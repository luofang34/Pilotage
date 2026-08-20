//! Simulation flight recorder: truth beside estimate, one JSONL line
//! per telemetry sample.
//!
//! A simulated flight exists to be judged, and judging needs the
//! simulator's ground truth NEXT TO what the flight controller believed
//! — the barometric altitude, the GNSS-fed kinematics, and eventually a
//! vision estimate — in one durable record a tuning pass can regress
//! against. The recorder is simulation-only (ADR-0040): a flight build
//! carries neither the truth fields nor this module, and a client-side
//! recording (the iPad's) can extend itself with the same optional wire
//! fields without a schema of its own.
//!
//! `PILOTAGE_RECORD_DIR` names the directory; unset means no recording.
//! The recorder is a rolling FDR: each host run writes numbered
//! segments (`flight-<pid>-<seq>.jsonl`), rolls to a new segment at a
//! fixed size, and deletes the oldest beyond a fixed count — the
//! recording holds the RECENT window, bounded, the way a flight data
//! recorder does. An unbounded recording once filled the disk and took
//! the whole session down with it. Writes are best-effort and buffered
//! — a full disk degrades to a warning, never to a telemetry stall.

use std::io::Write as _;
use std::path::PathBuf;

use pilotage_adapter_api::TelemetrySample;
use pilotage_timing::MonoTimestamp;

/// One segment's size ceiling. At the nominal ~50 KB/s sample stream a
/// segment holds roughly ten minutes.
const SEGMENT_BYTES: u64 = 32 * 1024 * 1024;
/// Segments retained, current one included: the recording's whole
/// footprint stays under `KEEP_SEGMENTS x SEGMENT_BYTES` (~128 MB,
/// covering the last forty-ish minutes).
const KEEP_SEGMENTS: u32 = 4;

/// The per-run recorder, or `None` when recording is off.
pub(super) struct Recorder {
    dir: PathBuf,
    file: std::io::BufWriter<std::fs::File>,
    /// Bytes written into the CURRENT segment.
    segment_bytes: u64,
    /// The current segment's sequence number.
    sequence: u32,
    /// One warning per run, so a failing disk does not storm the log.
    warned: bool,
    /// A recorder that could not roll: writing stopped for good, so
    /// the footprint guarantee outlives the disk fault.
    dead: bool,
}

impl Recorder {
    /// Opens the recorder when `PILOTAGE_RECORD_DIR` names a directory.
    pub(super) fn from_env() -> Option<Self> {
        let dir = std::env::var_os("PILOTAGE_RECORD_DIR").map(PathBuf::from)?;
        let path = dir.join(Self::segment_name(0));
        let open = || -> std::io::Result<std::fs::File> {
            std::fs::create_dir_all(&dir)?;
            std::fs::File::create(&path)
        };
        match open() {
            Ok(file) => {
                tracing::info!(path = %path.display(), "flight recorder on (rolling FDR)");
                Some(Self {
                    dir,
                    file: std::io::BufWriter::new(file),
                    segment_bytes: 0,
                    sequence: 0,
                    warned: false,
                    dead: false,
                })
            }
            Err(error) => {
                tracing::warn!(%error, "flight recorder could not open; recording off");
                None
            }
        }
    }

    fn segment_name(sequence: u32) -> String {
        format!("flight-{}-{sequence}.jsonl", std::process::id())
    }

    /// Rolls to the next segment and drops the one falling out of the
    /// retained window. A roll that cannot create its next segment
    /// STOPS the recorder: the sequence must not advance past files
    /// that never existed (the retained-window deletion would drift
    /// off the real files), and retrying into the old segment would
    /// grow it without bound — the one guarantee this recorder makes
    /// is its footprint.
    fn roll(&mut self) {
        use std::io::Write as _;
        self.file.flush().ok();
        let next = self.sequence.wrapping_add(1);
        let path = self.dir.join(Self::segment_name(next));
        match std::fs::File::create(&path) {
            Ok(file) => {
                self.sequence = next;
                self.file = std::io::BufWriter::new(file);
                self.segment_bytes = 0;
            }
            Err(error) => {
                self.dead = true;
                tracing::warn!(%error, "FDR segment roll failed; recording stopped");
                return;
            }
        }
        if let Some(expired) = self.sequence.checked_sub(KEEP_SEGMENTS) {
            std::fs::remove_file(self.dir.join(Self::segment_name(expired))).ok();
        }
    }

    /// Appends one sample. Truth and estimate ride the same line so a
    /// consumer never has to re-associate them by time.
    pub(super) fn record(&mut self, sample: &TelemetrySample, now: MonoTimestamp) {
        if self.dead {
            return;
        }
        let mut line = String::with_capacity(512);
        line.push_str(&format!(
            "{{\"t_ns\":{},\"tick\":{}",
            now.as_nanos(),
            sample.tick.as_u64()
        ));
        if let Some(avionics) = &sample.avionics {
            if let Some(kinematics) = &avionics.kinematics {
                line.push_str(&format!(
                    ",\"est_pos_ned\":[{},{},{}],\"est_vel_ned\":[{},{},{}]",
                    kinematics.pos_ned_m[0],
                    kinematics.pos_ned_m[1],
                    kinematics.pos_ned_m[2],
                    kinematics.vel_ned_mps[0],
                    kinematics.vel_ned_mps[1],
                    kinematics.vel_ned_mps[2],
                ));
            }
            if let Some(attitude) = &avionics.attitude {
                line.push_str(&format!(
                    ",\"est_quat\":[{},{},{},{}]",
                    attitude.quat_wxyz[0],
                    attitude.quat_wxyz[1],
                    attitude.quat_wxyz[2],
                    attitude.quat_wxyz[3],
                ));
            }
            if let Some(baro) = &avionics.baro {
                line.push_str(&format!(",\"baro_alt_m\":{}", baro.pressure_alt_m));
            }
        }
        if let Some(truth) = &sample.sim_truth {
            line.push_str(&format!(
                ",\"truth_pos_ned\":[{},{},{}],\"truth_vel_ned\":[{},{},{}],\"truth_quat\":[{},{},{},{}]",
                truth.pos_ned_m[0],
                truth.pos_ned_m[1],
                truth.pos_ned_m[2],
                truth.vel_ned_mps[0],
                truth.vel_ned_mps[1],
                truth.vel_ned_mps[2],
                truth.quat_wxyz[0],
                truth.quat_wxyz[1],
                truth.quat_wxyz[2],
                truth.quat_wxyz[3],
            ));
        }
        line.push_str("}\n");
        if self.file.write_all(line.as_bytes()).is_err() {
            if !self.warned {
                self.warned = true;
                tracing::warn!("flight recorder write failed; further failures are silent");
            }
            return;
        }
        self.segment_bytes = self.segment_bytes.saturating_add(line.len() as u64);
        if self.segment_bytes >= SEGMENT_BYTES {
            self.roll();
        }
    }
}
