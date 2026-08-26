//! Build-variant seam for the simulation-only attachments: a simulation
//! build holds live camera-sidecar and truth-oracle handles; a flight
//! build holds uninhabited types so both fields are structurally `None`.

/// The live sidecar client handle in a simulation build.
#[cfg(feature = "sim")]
pub(super) type CameraBridge = pilotage_sim_video::BridgeClient;
/// A flight build has no video producer.
#[cfg(not(feature = "sim"))]
pub(super) type CameraBridge = std::convert::Infallible;

/// The commanded pointing of a producer-rendered payload view. A flight
/// vehicle's gimbal is a real device on its own link, not a rendered
/// view, so this attachment is simulation-only and the type is
/// uninhabited in a flight build.
#[cfg(feature = "sim")]
pub(super) type Pointing = super::pointing::PointingState;
/// A flight build renders no view to aim.
#[cfg(not(feature = "sim"))]
pub(super) type Pointing = std::convert::Infallible;

/// The simulation-truth oracle handle in a simulation build.
#[cfg(feature = "sim")]
pub(super) type TruthOracle = super::shm_sampling::ShmSource;
/// A flight build has no simulator to borrow truth from.
#[cfg(not(feature = "sim"))]
pub(super) type TruthOracle = std::convert::Infallible;

/// The no-video triple of a flight build.
#[cfg(not(feature = "sim"))]
pub(super) fn no_camera() -> (
    Option<tokio::sync::mpsc::Receiver<pilotage_adapter_api::RawVideoFrame>>,
    Option<CameraBridge>,
    Option<tokio::task::JoinHandle<()>>,
) {
    (None, None, None)
}

impl super::AviateAdapter {
    /// The typed fault that has fail-closed the simulation-truth source,
    /// if any. A faulted source publishes no telemetry and does not
    /// re-attach. Only the shared-memory truth source carries a
    /// fail-closed fault state; the MAVLink estimate link reports `None`
    /// here.
    #[must_use]
    pub fn shm_fault(&self) -> Option<&super::AviateAdapterError> {
        #[cfg(feature = "sim")]
        {
            self.truth.as_ref().and_then(|source| source.fault())
        }
        #[cfg(not(feature = "sim"))]
        {
            None
        }
    }

    /// The truth oracle's sample for this tick, `None` in a flight build
    /// (the oracle type is uninhabited there).
    pub(super) fn take_truth_sample(&mut self) -> Option<pilotage_adapter_api::SimTruthSample> {
        #[cfg(feature = "sim")]
        {
            let mut sample = self
                .truth
                .as_mut()
                .and_then(|source| source.truth_sample())?;
            sample.geodetic = self.paired_fix(sample.stamp);
            self.report_join(sample.geodetic.is_some());
            Some(sample)
        }
        #[cfg(not(feature = "sim"))]
        {
            self.truth
                .as_ref()
                .map(|never| -> pilotage_adapter_api::SimTruthSample { match **never {} })
        }
    }

    /// Reports a run of truth samples published with no fix joined to them.
    ///
    /// A sensor that never speaks, a topic nobody publishes, and two clocks
    /// that do not share an origin are the same from here: no position,
    /// for the rest of the session, in silence. One report at the start of
    /// a run says which session is affected without a line per sample; the
    /// count says how long the run has lasted when it ends.
    #[cfg(feature = "sim")]
    fn report_join(&mut self, joined: bool) {
        /// Enough samples that a single late fix does not raise a report,
        /// and few enough that an operator learns inside a second at
        /// 30 Hz.
        const RUN_BEFORE_REPORT: u64 = 30;

        if joined {
            if self.navsat_join_failures >= RUN_BEFORE_REPORT {
                tracing::info!(
                    samples = self.navsat_join_failures,
                    "simulator position joined again after a run with none",
                );
            }
            self.navsat_join_failures = 0;
            return;
        }
        self.navsat_join_failures = self.navsat_join_failures.wrapping_add(1);
        if self.navsat_join_failures == RUN_BEFORE_REPORT {
            tracing::warn!(
                samples = RUN_BEFORE_REPORT,
                sensor_bound = self.camera_bridge.is_some(),
                "no simulator position joined to the truth samples; the map states none",
            );
        }
    }

    /// The simulator's own satellite-navigation fix for this observation.
    #[cfg(feature = "sim")]
    fn paired_fix(
        &self,
        stamp: pilotage_adapter_api::MeasurementStamp,
    ) -> Option<pilotage_adapter_api::GeodeticFixSample> {
        fix_for_moment(self.camera_bridge.as_ref()?.latest_cached().navsat?, stamp)
    }

    /// The truth oracle's session tick in nanoseconds, `None` when no
    /// oracle is bound.
    pub(super) fn truth_tick_ns(&self) -> Option<u64> {
        #[cfg(feature = "sim")]
        {
            self.truth.as_ref().map(|source| source.tick())
        }
        #[cfg(not(feature = "sim"))]
        {
            self.truth.as_ref().map(|never| -> u64 { match **never {} })
        }
    }
}

/// Joins a satellite-navigation fix to the truth observation it belongs
/// with, or refuses it.
///
/// The oracle reads the model's local frame over shared memory and the
/// sensor reports its position over the sidecar link, so the two arrive
/// apart and are NOT one observation until something says they are. Both
/// carry the simulation clock, so that is what says it: a fix is attached
/// only while it names the same moment as the sample. A fix carried from an
/// older moment would put the vehicle where it used to be while every other
/// value on the sample says where it is now.
#[cfg(feature = "sim")]
fn fix_for_moment(
    fix: pilotage_sim_video::BridgeNavSat,
    stamp: pilotage_adapter_api::MeasurementStamp,
) -> Option<pilotage_adapter_api::GeodeticFixSample> {
    use pilotage_geo::{
        BaroSettingId, DatumRealizationId, GeodeticPosition, HorizontalDatum, LocalOriginId,
        PositionQuality, SIMULATOR_GEOID_MODEL_ID, TerrainRefId, VerticalDatum, VerticalPosition,
    };

    /// How far apart on the simulation clock a fix and a truth sample may be
    /// and still describe the same moment. The sensor publishes at 30 Hz, so
    /// two of its periods is the widest gap an aligned pair can show.
    const SAME_MOMENT_NS: u64 = 67_000_000;

    if stamp.acquired_at_ns.abs_diff(fix.sim_time_ns) > SAME_MOMENT_NS {
        return None;
    }
    // Zero latitude AND zero longitude is a world that declared no datum,
    // not a vehicle at Null Island. A world with no `spherical_coordinates`
    // block leaves the sensor's origin at 0,0, and a vehicle standing on
    // the ground there reports a small non-zero altitude — so the altitude
    // says nothing about whether a datum exists, and requiring it to be
    // zero too let the whole case through. Exact zero on both angles is
    // not a place a simulator flies to; it is the default nobody set.
    if fix.latitude_deg == 0.0 && fix.longitude_deg == 0.0 {
        return None;
    }
    // The sensor states an altitude above mean sea level and names no geoid,
    // so the height declares the simulator's own separation and stays
    // traceable to a simulator.
    let vertical = VerticalPosition::new(
        fix.altitude_m,
        VerticalDatum::Msl,
        SIMULATOR_GEOID_MODEL_ID,
        TerrainRefId::UNDECLARED,
        BaroSettingId::UNDECLARED,
        LocalOriginId::UNDECLARED,
    )
    .ok()?;
    let position = GeodeticPosition::new(
        fix.latitude_deg,
        fix.longitude_deg,
        HorizontalDatum::Wgs84,
        DatumRealizationId::UNDECLARED,
        vertical,
    )
    .ok()?;
    Some(pilotage_adapter_api::GeodeticFixSample {
        position,
        // The simulator states no accuracy, and an unstated accuracy is
        // unstated: zero is what the wire reads as "not said".
        quality: PositionQuality {
            horizontal_mm: 0,
            vertical_mm: 0,
        },
        stamp,
    })
}

#[cfg(all(test, feature = "sim"))]
mod tests;
