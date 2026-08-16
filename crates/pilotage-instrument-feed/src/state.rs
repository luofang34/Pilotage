//! The feed: lanes in, one encoded state frame out.

use indicate_instrument_feeder::avionics::{GroupSnapshot, IngressSnapshot};
use indicate_instrument_state::abi::v7::encode_state;
use indicate_instrument_state::{
    AircraftState, Attitude, EstimateQuality, HeadingReference, HeadingSample, Kinematics,
    SnapshotCoherence, SnapshotMeta, Stamped, ValidFlags,
};
use pilotage_instrument_runtime::feeder::{
    Ingress, IngressParams, NavGuidance, Turn, nav_display_state,
};
use pilotage_protocol::wire;

use crate::sample::avionics_sample;

/// Construction parameters, forwarded to the shared ingress.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FeedParams {
    /// The vehicle whose publications this feed accepts.
    pub vehicle_id: u64,
    /// Accept unseen incarnations under the simulation policy.
    pub sim_accept_unseen: bool,
}

/// One client's telemetry-to-state pipeline: the avionics ingress, the
/// turn derivation, and the navigation-guidance lane, assembled into the
/// state frame exactly the way the browser viewer assembles them.
pub struct InstrumentFeed {
    ingress: Ingress,
    turn: Turn,
    nav: NavGuidance,
}

impl InstrumentFeed {
    /// A feed that has admitted nothing.
    #[must_use]
    pub fn new(params: &FeedParams) -> Self {
        Self {
            ingress: Ingress::new(&IngressParams {
                vehicle_id: params.vehicle_id,
                source_id: None,
                incarnation: None,
                sim_accept_unseen: params.sim_accept_unseen,
                maximum_seen_incarnations: 4,
                maximum_skew_nanos: 50_000_000,
            }),
            turn: Turn::new(),
            nav: NavGuidance::new(),
        }
    }

    /// Ingests one wire telemetry sample; returns whether admitted state
    /// changed. Publications for other vehicles are the ingress's own
    /// refusal to make, not this function's.
    pub fn ingest(&mut self, sample: &wire::TelemetrySample, now_ms: f64) -> bool {
        let Some(avionics) = sample.avionics.as_ref() else {
            return false;
        };
        let vehicle_id = sample.vehicle.as_ref().map_or(0, |v| v.value);
        self.ingress
            .ingest(&avionics_sample(vehicle_id, avionics), now_ms)
    }

    /// Assembles and encodes the current state frame against the caller's
    /// clock. `buf` is the runtime's state buffer capacity; the returned
    /// length is the encoded prefix.
    ///
    /// # Errors
    ///
    /// Returns the ABI error when `buf` cannot hold the frame, which is a
    /// caller sizing fault rather than a data fault.
    pub fn state_frame(
        &mut self,
        now_ms: f64,
        buf: &mut [u8],
    ) -> Result<usize, indicate_instrument_state::abi::v7::AbiError> {
        let snapshot = self.ingress.snapshot(now_ms);
        let state = self.assemble(&snapshot, now_ms);
        encode_state(&state, buf)
    }

    /// The assembly the browser viewer performs per frame, in one place.
    fn assemble(&mut self, snapshot: &IngressSnapshot, now_ms: f64) -> AircraftState {
        let heading = declared_sim_heading(snapshot.attitude.as_ref());
        let dynamics = self.turn.update(
            heading
                .as_ref()
                .map_or(f64::NAN, |h| f64::from(h.data.heading_rad)),
            heading
                .as_ref()
                .and_then(|h| h.age_ms)
                .map_or(f64::NAN, f64::from),
            snapshot.attitude.as_ref().map(|group| &group.stamp),
        );
        let valid_flags = snapshot.valid_flags;
        let attitude_valid = valid_flags & 1 != 0;
        AircraftState {
            attitude: stamped_group(snapshot.attitude.as_ref(), |group| Attitude {
                quat: indicate_instrument_state::Quat {
                    w: group.quat[0],
                    x: group.quat[1],
                    y: group.quat[2],
                    z: group.quat[3],
                },
                rates_rps: group.rates,
            }),
            kinematics: stamped_group(snapshot.kinematics.as_ref(), |group| Kinematics {
                pos_ned_m: group.pos_ned,
                vel_ned_mps: group.vel_ned,
            }),
            nav: nav_display_state(self.nav.snapshot(now_ms).as_ref()).unwrap_or_default(),
            heading: heading.map_or_else(Stamped::default, |h| Stamped {
                data: Some(h.data),
                age_ms: h.age_ms,
            }),
            dynamics: dynamics.map_or_else(Stamped::default, |declaration| Stamped {
                data: Some(indicate_instrument_state::DynSample {
                    turn: Some(indicate_instrument_state::TurnSample {
                        #[allow(clippy::cast_possible_truncation)]
                        rate_rps: declaration.turn_rps as f32,
                        basis: indicate_instrument_state::TurnBasis::from_u8(
                            declaration.turn_basis,
                        ),
                    }),
                    lateral_mps2: None,
                }),
                age_ms: to_age_ms(declaration.age_ms),
            }),
            quality: quality_from_code(snapshot.quality),
            valid: ValidFlags {
                attitude: attitude_valid,
                rates: valid_flags & 2 != 0,
                position: valid_flags & 4 != 0,
                velocity_horizontal: valid_flags & 8 != 0,
                velocity_vertical: valid_flags & 8 != 0,
                heading: attitude_valid && snapshot.attitude.is_some(),
                turn: attitude_valid,
                slip: false,
                variation: false,
            },
            snapshot: SnapshotMeta {
                generation: snapshot.generation,
                coherence: coherence_from_report(snapshot),
            },
            ..AircraftState::default()
        }
    }
}

/// One stamped state group from one admitted feeder group.
fn stamped_group<G, T>(
    group: Option<&GroupSnapshot<G>>,
    convert: impl FnOnce(&G) -> T,
) -> Stamped<T> {
    group.map_or_else(Stamped::default, |snapshot| Stamped {
        data: Some(convert(&snapshot.data)),
        age_ms: to_age_ms(snapshot.age_ms),
    })
}

/// A heading declared from the simulation attitude's yaw, reference
/// TRUE-of-sim, exactly as the viewer declares it. The sim is the only
/// source this feed currently serves; a magnetometer lane would arrive
/// with its own reference.
struct DeclaredHeading {
    data: HeadingSample,
    age_ms: Option<f32>,
}

fn declared_sim_heading(
    attitude: Option<&GroupSnapshot<indicate_instrument_feeder::avionics::AttitudeGroup>>,
) -> Option<DeclaredHeading> {
    let group = attitude?;
    let [w, x, y, z] = group.data.quat;
    if ![w, x, y, z].iter().all(|v| v.is_finite()) {
        return None;
    }
    let yaw = f64::from(2.0 * (w * z + x * y)).atan2(f64::from(1.0 - 2.0 * (y * y + z * z)));
    #[allow(clippy::cast_possible_truncation)]
    Some(DeclaredHeading {
        data: HeadingSample {
            heading_rad: yaw as f32,
            reference: HeadingReference::SimLocalTrue,
        },
        age_ms: to_age_ms(group.age_ms),
    })
}

#[allow(clippy::cast_possible_truncation)]
fn to_age_ms(age_ms: f64) -> Option<f32> {
    age_ms.is_finite().then_some(age_ms as f32)
}

fn quality_from_code(code: u32) -> EstimateQuality {
    match code {
        0 => EstimateQuality::Good,
        1 => EstimateQuality::Degraded,
        2 => EstimateQuality::Unusable,
        _ => EstimateQuality::Unknown,
    }
}

fn coherence_from_report(snapshot: &IngressSnapshot) -> SnapshotCoherence {
    use indicate_instrument_feeder::avionics::Coherence;
    match snapshot.coherence.status {
        Coherence::Coherent => SnapshotCoherence::Coherent,
        Coherence::ExcessiveSkew => SnapshotCoherence::ExcessiveSkew,
        Coherence::Insufficient => SnapshotCoherence::Insufficient,
    }
}
