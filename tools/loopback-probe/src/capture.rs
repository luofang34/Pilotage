//! Structured session capture: every decoded telemetry observation —
//! vehicle identity, pose, and all stamped role lanes with full
//! provenance — serialized as one JSON line, so a recorded run is
//! runtime evidence rather than a printed summary.
//!
//! The file begins with a header record naming the capture schema; each
//! subsequent line is one observation. Write failures are counted and
//! reported once — a broken capture must be visible, but must not kill
//! the measurement run it was recording.

use std::fs::File;
use std::io::{BufWriter, Write};
use std::path::Path;

use serde::Serialize;
use tracing::warn;

use crate::telemetry::{CapturedEstimate, CapturedFcState, CapturedTruth, TelemetryObservation};

/// Capture schema identity written as the file's first record.
const CAPTURE_SCHEMA: &str = "pilotage-loopback-capture/1";

#[derive(Serialize)]
struct HeaderRecord<'a> {
    schema: &'a str,
}

/// One serialized observation. `received_at_ns` is the probe-local
/// monotonic receive time; everything else is retained verbatim from the
/// wire sample.
#[derive(Serialize)]
struct ObservationRecord<'a> {
    received_at_ns: u64,
    vehicle: Option<u64>,
    pose: Option<(f32, f32, f32)>,
    estimate: Option<&'a CapturedEstimate>,
    sim_truth: Option<&'a CapturedTruth>,
    fc_state: Option<&'a CapturedFcState>,
}

/// Line-oriented JSON capture sink.
#[derive(Debug)]
pub struct CaptureWriter {
    writer: BufWriter<File>,
    records: u64,
    write_failures: u64,
}

impl CaptureWriter {
    /// Creates the capture file and writes the schema header.
    ///
    /// # Errors
    ///
    /// Returns the file-creation or header-write error.
    pub fn create(path: &Path) -> std::io::Result<Self> {
        let mut writer = BufWriter::new(File::create(path)?);
        let header = serde_json::to_string(&HeaderRecord {
            schema: CAPTURE_SCHEMA,
        })?;
        writeln!(writer, "{header}")?;
        Ok(Self {
            writer,
            records: 0,
            write_failures: 0,
        })
    }

    /// Appends one observation as a JSON line.
    pub fn record(&mut self, observation: &TelemetryObservation) {
        let record = ObservationRecord {
            received_at_ns: observation.received_at.as_nanos(),
            vehicle: observation.vehicle,
            pose: observation.pose,
            estimate: observation.estimate.as_ref(),
            sim_truth: observation.sim_truth.as_ref(),
            fc_state: observation.fc_state.as_ref(),
        };
        let outcome = serde_json::to_string(&record)
            .map_err(std::io::Error::other)
            .and_then(|line| writeln!(self.writer, "{line}"));
        match outcome {
            Ok(()) => self.records = self.records.wrapping_add(1),
            Err(error) => {
                self.write_failures = self.write_failures.wrapping_add(1);
                if self.write_failures == 1 {
                    warn!(%error, "capture write failed; further failures counted silently");
                }
            }
        }
    }

    /// Flushes the sink and returns `(records written, write failures)`.
    pub fn finish(mut self) -> (u64, u64) {
        if let Err(error) = self.writer.flush() {
            self.write_failures = self.write_failures.wrapping_add(1);
            warn!(%error, "capture flush failed");
        }
        (self.records, self.write_failures)
    }
}

#[cfg(test)]
#[allow(clippy::expect_used, clippy::panic)]
mod tests {
    use pilotage_protocol::wire;
    use pilotage_timing::MonoTimestamp;

    use super::CaptureWriter;
    use crate::telemetry::observation_from_sample;

    fn stamp(role: wire::SourceRole, integrity: wire::SourceIntegrity) -> wire::MeasurementStamp {
        wire::MeasurementStamp {
            role: role as i32,
            integrity: integrity as i32,
            source_id: 0x01be,
            source_epoch: 2,
            sequence: 40,
            acquired_at_ns: 1_000_000,
            clock: wire::MeasurementClock::Simulation as i32,
            source_incarnation: vec![0x11; 16],
        }
    }

    #[test]
    fn capture_round_trips_every_provenance_field() {
        let sample = wire::TelemetrySample {
            vehicle: Some(wire::VehicleId { value: 1 }),
            tick: Some(wire::SimTick { value: 1_000_000 }),
            observed_at: Some(wire::MonoTimestamp { nanos: 5 }),
            pose: None,
            velocity: None,
            avionics: Some(wire::AvionicsState {
                valid_flags: 0b1111,
                quality: 0,
                attitude_stamp: Some(stamp(
                    wire::SourceRole::OperationalEstimate,
                    wire::SourceIntegrity::ChecksummedOnly,
                )),
                ..Default::default()
            }),
            sim_truth: Some(Box::new(wire::SimTruthState {
                pos_n_m: 2.0,
                valid_flags: 0b1101,
                stamp: Some(stamp(
                    wire::SourceRole::SimulationTruth,
                    wire::SourceIntegrity::Unprotected,
                )),
                ..Default::default()
            })),
            fc_state: Some(Box::new(wire::FcState {
                arm_state: 2,
                stamp: Some(wire::MeasurementStamp {
                    role: wire::SourceRole::FcState as i32,
                    clock: wire::MeasurementClock::HostMonotonic as i32,
                    ..stamp(
                        wire::SourceRole::FcState,
                        wire::SourceIntegrity::ChecksummedOnly,
                    )
                }),
            })),
            gimbal: None,
        };
        let observation = observation_from_sample(&sample, MonoTimestamp::from_nanos(9));

        let path = std::env::temp_dir().join(format!("plt_capture_{}.jsonl", std::process::id()));
        let mut writer = CaptureWriter::create(&path).expect("create capture");
        writer.record(&observation);
        let (records, failures) = writer.finish();
        assert_eq!((records, failures), (1, 0));

        let content = std::fs::read_to_string(&path).expect("read capture back");
        std::fs::remove_file(&path).ok();
        let mut lines = content.lines();
        assert!(
            lines
                .next()
                .expect("header line")
                .contains("pilotage-loopback-capture/1")
        );
        let record: serde_json::Value =
            serde_json::from_str(lines.next().expect("observation line")).expect("valid json");
        assert_eq!(record["received_at_ns"], 9);
        assert_eq!(record["vehicle"], 1);
        let truth = &record["sim_truth"]["provenance"];
        assert_eq!(truth["role"], 2);
        assert_eq!(truth["source_incarnation"], "11".repeat(16));
        assert_eq!(truth["acquired_at_ns"], 1_000_000);
        assert_eq!(truth["source_epoch"], 2);
        assert_eq!(record["fc_state"]["provenance"]["clock"], 3);
        let estimate = &record["estimate"];
        assert_eq!(estimate["valid_flags"], 0b1111);
        assert_eq!(estimate["attitude_stamp"]["integrity"], 2);
    }
}
