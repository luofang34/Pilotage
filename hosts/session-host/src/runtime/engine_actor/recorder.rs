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
//! Each host run appends to `flight-<pid>.jsonl` there. Writes are
//! best-effort and buffered — a full disk degrades to a warning, never
//! to a telemetry stall.

use std::io::Write as _;
use std::path::PathBuf;

use pilotage_adapter_api::TelemetrySample;
use pilotage_timing::MonoTimestamp;

/// The per-run recorder, or `None` when recording is off.
pub(super) struct Recorder {
    file: std::io::BufWriter<std::fs::File>,
    /// One warning per run, so a failing disk does not storm the log.
    warned: bool,
}

impl Recorder {
    /// Opens the recorder when `PILOTAGE_RECORD_DIR` names a directory.
    pub(super) fn from_env() -> Option<Self> {
        let dir = std::env::var_os("PILOTAGE_RECORD_DIR").map(PathBuf::from)?;
        let path = dir.join(format!("flight-{}.jsonl", std::process::id()));
        let open = || -> std::io::Result<std::fs::File> {
            std::fs::create_dir_all(&dir)?;
            std::fs::OpenOptions::new().create(true).append(true).open(&path)
        };
        match open() {
            Ok(file) => {
                tracing::info!(path = %path.display(), "flight recorder on");
                Some(Self {
                    file: std::io::BufWriter::new(file),
                    warned: false,
                })
            }
            Err(error) => {
                tracing::warn!(%error, "flight recorder could not open; recording off");
                None
            }
        }
    }

    /// Appends one sample. Truth and estimate ride the same line so a
    /// consumer never has to re-associate them by time.
    pub(super) fn record(&mut self, sample: &TelemetrySample, now: MonoTimestamp) {
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
        if self.file.write_all(line.as_bytes()).is_err() && !self.warned {
            self.warned = true;
            tracing::warn!("flight recorder write failed; further failures are silent");
        }
    }
}
