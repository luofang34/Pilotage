//! Telemetry in the shapes the DJI adapter publishes, plus the simulator
//! truth group.
//!
//! The estimate is the simulated state without error. Attitude,
//! kinematics, and the geodetic fix share one estimate stamp, as on the
//! DJI airframe report. The fix height is relative to the launch point.

use pilotage_adapter_api::{
    AvionicsAttitudeSample, AvionicsKinematicsSample, AvionicsSample, BatterySample, FcStateSample,
    GeodeticFixSample, GimbalAttitudeSample, LandedStateSample, MeasurementClock, MeasurementStamp,
    MotorStateSample, SimTruthSample, SourceIntegrity, SourceRole, TelemetrySample,
};
use pilotage_geo::{
    BaroSettingId, DatumRealizationId, GeodeticPosition, GeoidModelId, HorizontalDatum,
    PositionQuality, TerrainRefId, VerticalDatum, VerticalPosition,
};
use pilotage_protocol::{GimbalAxisLocks, GimbalReference, GimbalYawFrame};
use pilotage_timing::SimTick;

use super::{SimMulticopterAdapter, TICK_NS};
use crate::geodesy::{quaternion_from_euler, to_f32};

/// The airframe estimate publishes at 20 Hz.
const AIRFRAME_EVERY_TICKS: u64 = 5;
/// The gimbal attitude publishes at 10 Hz.
const GIMBAL_EVERY_TICKS: u64 = 10;
/// The battery publishes at 1 Hz.
const BATTERY_EVERY_TICKS: u64 = 100;
/// Estimator authorization: attitude, rates, position, and velocity.
const VALID_ALL: u32 = 0b1111;
/// Estimate quality: good.
const QUALITY_GOOD: u32 = 0;
/// Source identities of the published groups.
const AIRFRAME_SOURCE: u64 = 0x51_4d_01;
const GIMBAL_SOURCE: u64 = 0x51_4d_02;
const BATTERY_SOURCE: u64 = 0x51_4d_03;
const TRUTH_SOURCE: u64 = 0x51_4d_04;

/// The wrapping publication counters of each group.
#[derive(Debug, Default)]
pub(super) struct Sequences {
    airframe: u32,
    gimbal: u32,
    battery: u32,
    last_sampled_tick: Option<u64>,
}

impl SimMulticopterAdapter {
    fn stamp(&self, role: SourceRole, source_id: u64, sequence: u32) -> MeasurementStamp {
        MeasurementStamp {
            role,
            // The simulated groups carry the integrity class the DJI
            // reports carry, so consumers treat both the same way.
            integrity: if role == SourceRole::SimulationTruth {
                SourceIntegrity::Unprotected
            } else {
                SourceIntegrity::ChecksummedOnly
            },
            source_id,
            source_incarnation: self.incarnation,
            source_epoch: 0,
            sequence,
            acquired_at_ns: self.tick.wrapping_mul(TICK_NS),
            clock: MeasurementClock::Simulation,
        }
    }

    /// The sample of this tick, or `None` when no group is due or the
    /// tick was already sampled.
    pub(super) fn sample(&mut self) -> Option<TelemetrySample> {
        if self.sequences.last_sampled_tick == Some(self.tick) {
            return None;
        }
        let airframe_due = self.tick.is_multiple_of(AIRFRAME_EVERY_TICKS);
        let gimbal_due = self.tick.is_multiple_of(GIMBAL_EVERY_TICKS);
        let battery_due = self.tick.is_multiple_of(BATTERY_EVERY_TICKS);
        if !airframe_due && !gimbal_due && !battery_due {
            return None;
        }
        self.sequences.last_sampled_tick = Some(self.tick);
        let mut sample = TelemetrySample {
            flight_procedure: None,
            payload: None,
            vehicle: self.vehicle,
            tick: SimTick::new(self.tick),
            pose: None,
            speed: None,
            avionics: None,
            sim_truth: None,
            fc_state: None,
            gimbal: None,
            battery: None,
        };
        if airframe_due {
            self.sequences.airframe = self.sequences.airframe.wrapping_add(1);
            let estimate = self.stamp(
                SourceRole::OperationalEstimate,
                AIRFRAME_SOURCE,
                self.sequences.airframe,
            );
            sample.speed = Some(self.body.ground_speed_mps());
            sample.avionics = self.avionics(estimate);
            sample.sim_truth = self.truth();
            sample.fc_state = Some(self.fc_state(estimate));
        }
        if gimbal_due {
            self.sequences.gimbal = self.sequences.gimbal.wrapping_add(1);
            sample.gimbal = Some(self.gimbal_attitude());
        }
        if battery_due {
            self.sequences.battery = self.sequences.battery.wrapping_add(1);
            sample.battery = Some(self.battery_sample());
        }
        Some(sample)
    }

    fn attitude_quaternion(&self) -> [f32; 4] {
        let (roll, pitch) = self.body.tilt_rad();
        to_f32(quaternion_from_euler(roll, pitch, self.body.yaw_rad))
    }

    fn fix(&self, stamp: MeasurementStamp) -> Option<GeodeticFixSample> {
        let [lat, lon] = self
            .frame
            .lat_lon(self.body.pos_ned_m[0], self.body.pos_ned_m[1]);
        let vertical = VerticalPosition::new(
            self.body.height_m(),
            VerticalDatum::LocalRelative,
            GeoidModelId::UNDECLARED,
            TerrainRefId::UNDECLARED,
            BaroSettingId::UNDECLARED,
            self.origin,
        )
        .inspect_err(|error| tracing::warn!(%error, "simulated height refused"))
        .ok()?;
        let position = GeodeticPosition::new(
            lat,
            lon,
            HorizontalDatum::Wgs84,
            DatumRealizationId::UNDECLARED,
            vertical,
        )
        .inspect_err(|error| tracing::warn!(%error, "simulated fix refused"))
        .ok()?;
        Some(GeodeticFixSample {
            position,
            quality: PositionQuality {
                horizontal_mm: 0,
                vertical_mm: 0,
            },
            stamp,
        })
    }

    fn kinematics_group(&self, stamp: MeasurementStamp) -> AvionicsKinematicsSample {
        AvionicsKinematicsSample {
            pos_ned_m: self.body.pos_ned_m.map(|value| value as f32),
            vel_ned_mps: self.body.vel_ned_mps.map(|value| value as f32),
            stamp,
        }
    }

    fn avionics(&self, stamp: MeasurementStamp) -> Option<AvionicsSample> {
        Some(AvionicsSample {
            attitude: Some(AvionicsAttitudeSample::Canonical {
                quat_wxyz: self.attitude_quaternion(),
                rates_rps: Some([0.0, 0.0, self.body.yaw_rate_rps as f32]),
                stamp,
            }),
            kinematics: Some(self.kinematics_group(stamp)),
            baro: None,
            geodetic: Some(self.fix(stamp)?),
            magnetic_variation: None,
            estimator_status_stamp: Some(stamp),
            valid_flags: VALID_ALL,
            quality: QUALITY_GOOD,
        })
    }

    fn truth(&self) -> Option<SimTruthSample> {
        let stamp = self.stamp(
            SourceRole::SimulationTruth,
            TRUTH_SOURCE,
            self.sequences.airframe,
        );
        let kinematics = self.kinematics_group(stamp);
        Some(SimTruthSample {
            quat_wxyz: self.attitude_quaternion(),
            pos_ned_m: kinematics.pos_ned_m,
            vel_ned_mps: kinematics.vel_ned_mps,
            valid_flags: 0b1101,
            geodetic: Some(self.fix(stamp)?),
            stamp,
        })
    }

    fn fc_state(&self, estimate: MeasurementStamp) -> FcStateSample {
        let stamp = MeasurementStamp {
            role: SourceRole::FcState,
            ..estimate
        };
        FcStateSample {
            arm_state: if self.phase.armed() { 2 } else { 1 },
            last_command: None,
            landed_state: Some(LandedStateSample {
                state: self.phase.landed_state(),
                stamp,
            }),
            motor_state: Some(MotorStateSample {
                state: self.phase.motor_state(),
                stamp,
            }),
            landing_approach: None,
            stamp: Some(stamp),
        }
    }

    fn gimbal_attitude(&self) -> GimbalAttitudeSample {
        GimbalAttitudeSample {
            quat_wxyz: to_f32(quaternion_from_euler(
                0.0,
                self.gimbal.pitch_rad(),
                self.body.yaw_rad,
            )),
            rates_rps: [f32::NAN; 3],
            reference: GimbalReference {
                yaw_frame: GimbalYawFrame::Earth,
                locks: GimbalAxisLocks {
                    roll: true,
                    pitch: true,
                    yaw: false,
                },
            },
            failure_flags: 0,
            stamp: self.stamp(
                SourceRole::PayloadDevice,
                GIMBAL_SOURCE,
                self.sequences.gimbal,
            ),
        }
    }

    fn battery_sample(&self) -> BatterySample {
        BatterySample {
            instance: 0,
            remaining_fraction: Some(self.battery.remaining_fraction() as f32),
            voltage_v: Some(self.battery.voltage_v() as f32),
            current_a: Some(self.battery.current_a() as f32),
            consumed_energy_j: Some(self.battery.consumed_j() as f32),
            time_remaining_s: self.battery.time_remaining_s(),
            stamp: self.stamp(SourceRole::FcState, BATTERY_SOURCE, self.sequences.battery),
        }
    }
}
