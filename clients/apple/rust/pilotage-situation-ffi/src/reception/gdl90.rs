//! GDL 90 appliance stream state and domain normalization.

use aero_link::core::{
    Gdl90AddressType, Gdl90DecodeError, Gdl90Direction, Gdl90HeadingReference, Gdl90TrafficReport,
    decode_gdl90_foreflight_ahrs, decode_gdl90_legacy_ahrs, decode_gdl90_ownship_report,
    decode_gdl90_traffic_report, unframe_gdl90_message,
};
use surveillance_core::{
    AddressNamespace, AirGroundState, Callsign, DeliveryPath, EmergencyState, EmitterCategory,
    FieldQuality, HeadingReference, ObservationOrigin, ObservationTime, PositionObservation,
    SourceRef, TrackKey, TrafficObservation, VelocityObservation, VerticalRateSource,
    Wgs84Position,
};

use super::{DomainRecords, ReceptionError, ReceptionPipeline, ReceptionTally};
use crate::{Gdl90HeadingReferenceValue, Gdl90NavigationSnapshot};

#[cfg(test)]
mod tests;

const FRAME_FLAG: u8 = 0x7e;
const MAX_FRAME_BYTES: usize = 1_024;
const MAX_MESSAGE_BYTES: usize = 512;

#[derive(Default)]
struct FrameCollector {
    frame: Vec<u8>,
    inside_frame: bool,
}

impl FrameCollector {
    fn accept(&mut self, byte: u8) -> Option<Vec<u8>> {
        if byte == FRAME_FLAG {
            let complete = (self.inside_frame && self.frame.len() > 1).then(|| {
                let mut frame = core::mem::take(&mut self.frame);
                frame.push(FRAME_FLAG);
                frame
            });
            self.frame.clear();
            self.frame.push(FRAME_FLAG);
            self.inside_frame = true;
            return complete;
        }
        if !self.inside_frame {
            return None;
        }
        if self.frame.len() >= MAX_FRAME_BYTES {
            self.frame.clear();
            self.inside_frame = false;
            return Some(Vec::new());
        }
        self.frame.push(byte);
        None
    }
}

/// Mutable state belongs to the Apple adapter, not to the portable decoder.
#[derive(Default)]
pub(super) struct Gdl90StreamState {
    collector: FrameCollector,
    navigation: Gdl90NavigationSnapshot,
    has_navigation: bool,
    pub(super) bytes_consumed: u64,
    pub(super) valid_frames: u64,
    pub(super) crc_errors: u64,
    pub(super) invalid_frames: u64,
    pub(super) traffic_reports: u64,
    pub(super) deferred_uplink_messages: u64,
    pub(super) unsupported_messages: u64,
}

impl Gdl90StreamState {
    pub(super) fn accept_chunk(
        &mut self,
        data: &[u8],
        source_id: u32,
        source_epoch: u32,
        monotonic_micros: u64,
        pipeline: &mut ReceptionPipeline,
    ) -> Result<(ReceptionTally, DomainRecords), ReceptionError> {
        self.bytes_consumed = self.bytes_consumed.wrapping_add(data.len() as u64);
        let mut tally = ReceptionTally::default();
        let mut records = DomainRecords::default();
        for byte in data.iter().copied() {
            let Some(frame) = self.collector.accept(byte) else {
                continue;
            };
            if frame.is_empty() {
                self.invalid_frames = self.invalid_frames.wrapping_add(1);
                continue;
            }
            let mut message = [0_u8; MAX_MESSAGE_BYTES];
            match unframe_gdl90_message(&frame, &mut message) {
                Ok(length) => {
                    self.valid_frames = self.valid_frames.wrapping_add(1);
                    self.accept_message(
                        &message[..length],
                        source_id,
                        source_epoch,
                        monotonic_micros,
                        pipeline,
                        &mut tally,
                        &mut records,
                    )?;
                }
                Err(Gdl90DecodeError::CrcMismatch) => {
                    self.crc_errors = self.crc_errors.wrapping_add(1);
                }
                Err(_) => {
                    self.invalid_frames = self.invalid_frames.wrapping_add(1);
                }
            }
        }
        Ok((tally, records))
    }

    pub(super) fn navigation_snapshot(&self) -> Option<Gdl90NavigationSnapshot> {
        self.has_navigation.then_some(self.navigation)
    }

    #[allow(clippy::too_many_arguments)]
    fn accept_message(
        &mut self,
        message: &[u8],
        source_id: u32,
        source_epoch: u32,
        monotonic_micros: u64,
        pipeline: &mut ReceptionPipeline,
        tally: &mut ReceptionTally,
        records: &mut DomainRecords,
    ) -> Result<(), ReceptionError> {
        match message.first().copied() {
            Some(20) => self.accept_traffic(
                message,
                source_id,
                source_epoch,
                monotonic_micros,
                pipeline,
                tally,
                records,
            ),
            Some(10) => {
                self.accept_ownship(message, monotonic_micros);
                Ok(())
            }
            Some(0x4c) => {
                self.accept_legacy_ahrs(message, monotonic_micros);
                Ok(())
            }
            Some(0x65) => {
                self.accept_foreflight_ahrs(message, monotonic_micros);
                Ok(())
            }
            Some(7) => {
                self.deferred_uplink_messages = self.deferred_uplink_messages.wrapping_add(1);
                Ok(())
            }
            Some(0 | 11) => Ok(()),
            Some(_) => {
                self.unsupported_messages = self.unsupported_messages.wrapping_add(1);
                Ok(())
            }
            None => {
                self.invalid_frames = self.invalid_frames.wrapping_add(1);
                Ok(())
            }
        }
    }

    #[allow(clippy::too_many_arguments)]
    fn accept_traffic(
        &mut self,
        message: &[u8],
        source_id: u32,
        source_epoch: u32,
        monotonic_micros: u64,
        pipeline: &mut ReceptionPipeline,
        tally: &mut ReceptionTally,
        records: &mut DomainRecords,
    ) -> Result<(), ReceptionError> {
        let report = match decode_gdl90_traffic_report(message) {
            Ok(report) => report,
            Err(_) => {
                self.invalid_frames = self.invalid_frames.wrapping_add(1);
                return Ok(());
            }
        };
        let observation = normalize_traffic(report, source_id, source_epoch, monotonic_micros);
        let mut track = pipeline
            .traffic
            .accept_observation(observation, monotonic_micros)?;
        records.track.append(&mut track);
        tally.traffic_observations = tally.traffic_observations.wrapping_add(1);
        self.traffic_reports = self.traffic_reports.wrapping_add(1);
        Ok(())
    }

    fn accept_ownship(&mut self, message: &[u8], monotonic_micros: u64) {
        let Ok(report) = decode_gdl90_ownship_report(message) else {
            self.invalid_frames = self.invalid_frames.wrapping_add(1);
            return;
        };
        self.navigation.latitude_degrees = report.latitude_degrees;
        self.navigation.longitude_degrees = report.longitude_degrees;
        self.navigation.ownship_monotonic_micros = monotonic_micros;
        match report.direction {
            Some(Gdl90Direction::TrueTrack(value)) => {
                self.navigation.ground_track_degrees_true = Some(f64::from(value));
            }
            Some(Gdl90Direction::MagneticHeading(value)) => {
                self.set_heading(value, Gdl90HeadingReference::Magnetic);
            }
            Some(Gdl90Direction::TrueHeading(value)) => {
                self.set_heading(value, Gdl90HeadingReference::True);
            }
            None => {}
        }
        if let Some(altitude) = report.pressure_altitude_feet {
            self.navigation.pressure_altitude_feet = Some(f64::from(altitude));
            self.navigation.baro_monotonic_micros = monotonic_micros;
        }
        self.has_navigation = true;
    }

    fn accept_legacy_ahrs(&mut self, message: &[u8], monotonic_micros: u64) {
        let Ok(decoded) = decode_gdl90_legacy_ahrs(message) else {
            self.invalid_frames = self.invalid_frames.wrapping_add(1);
            return;
        };
        self.navigation.roll_degrees = decoded.report.roll_degrees.map(f64::from);
        self.navigation.pitch_degrees = decoded.report.pitch_degrees.map(f64::from);
        self.navigation.attitude_monotonic_micros = monotonic_micros;
        self.navigation.pressure_altitude_feet =
            decoded.report.pressure_altitude_feet.map(f64::from);
        self.navigation.vertical_speed_feet_per_minute =
            decoded.report.vertical_speed_feet_per_minute.map(f64::from);
        self.navigation.baro_monotonic_micros = monotonic_micros;
        self.has_navigation = true;
    }

    fn accept_foreflight_ahrs(&mut self, message: &[u8], monotonic_micros: u64) {
        let Ok(report) = decode_gdl90_foreflight_ahrs(message) else {
            self.invalid_frames = self.invalid_frames.wrapping_add(1);
            return;
        };
        self.navigation.roll_degrees = report.roll_degrees.map(f64::from);
        self.navigation.pitch_degrees = report.pitch_degrees.map(f64::from);
        self.navigation.attitude_monotonic_micros = monotonic_micros;
        if let Some(heading) = report.heading {
            self.set_heading(heading.degrees, heading.reference);
        }
        self.has_navigation = true;
    }

    fn set_heading(&mut self, degrees: f32, reference: Gdl90HeadingReference) {
        self.navigation.heading_degrees = Some(f64::from(degrees));
        self.navigation.heading_reference = Some(match reference {
            Gdl90HeadingReference::True => Gdl90HeadingReferenceValue::TrueNorth,
            Gdl90HeadingReference::Magnetic => Gdl90HeadingReferenceValue::MagneticNorth,
        });
    }
}

fn normalize_traffic(
    report: Gdl90TrafficReport,
    source_id: u32,
    source_epoch: u32,
    monotonic_micros: u64,
) -> TrafficObservation {
    let mut observation = TrafficObservation::new(
        ObservationTime::local(monotonic_micros),
        SourceRef::new(source_id, source_epoch, DeliveryPath::InstalledAvionics),
        ObservationOrigin::PanelFused,
        TrackKey::new(
            address_namespace(report.address_type),
            report.participant_address,
        ),
    );
    observation.position = report.latitude_degrees.zip(report.longitude_degrees).map(
        |(latitude_deg, longitude_deg)| {
            PositionObservation::Decoded(Wgs84Position {
                latitude_deg,
                longitude_deg,
            })
        },
    );
    observation.pressure_altitude_ft = report.pressure_altitude_feet.map(|value| value as i32);
    observation.velocity = traffic_velocity(report);
    observation.callsign = decode_callsign(report.callsign).map(Callsign::new);
    observation.air_ground = Some(if report.airborne {
        AirGroundState::Subsonic
    } else {
        AirGroundState::Ground
    });
    observation.emergency = Some(emergency_state(report.emergency_code));
    observation.emitter_category = emitter_category(report.emitter_category);
    observation.quality = FieldQuality::new(
        (report.nic <= 11).then_some(report.nic),
        (report.nac_p <= 11).then_some(report.nac_p),
        None,
        None,
    );
    observation
}

fn traffic_velocity(report: Gdl90TrafficReport) -> Option<VelocityObservation> {
    let mut velocity = VelocityObservation::default();
    velocity.ground_speed_kt = report.ground_speed_knots.map(f64::from);
    velocity.vertical_rate_fpm = report
        .vertical_speed_feet_per_minute
        .map(|value| value as i32);
    velocity.vertical_rate_source = report
        .vertical_speed_feet_per_minute
        .map(|_| VerticalRateSource::Barometric);
    match report.direction {
        Some(Gdl90Direction::TrueTrack(value)) => {
            velocity.track_angle_deg_true = Some(f64::from(value));
        }
        Some(Gdl90Direction::MagneticHeading(value)) => {
            velocity.heading_deg = Some(f64::from(value));
            velocity.heading_reference = Some(HeadingReference::MagneticNorth);
        }
        Some(Gdl90Direction::TrueHeading(value)) => {
            velocity.heading_deg = Some(f64::from(value));
            velocity.heading_reference = Some(HeadingReference::TrueNorth);
        }
        None => {}
    }
    (velocity != VelocityObservation::default()).then_some(velocity)
}

fn address_namespace(address_type: Gdl90AddressType) -> AddressNamespace {
    match address_type {
        Gdl90AddressType::AdsbIcao | Gdl90AddressType::TisbIcao => AddressNamespace::Icao,
        Gdl90AddressType::AdsbSelfAssigned => AddressNamespace::SelfAssigned,
        Gdl90AddressType::TisbTrackFile => AddressNamespace::TisbTrackFile,
        Gdl90AddressType::SurfaceVehicle => AddressNamespace::SurfaceVehicle,
        Gdl90AddressType::GroundStationBeacon => AddressNamespace::FixedBeacon,
        Gdl90AddressType::Reserved(_) => AddressNamespace::AdsbNonIcao,
        _ => AddressNamespace::AdsbNonIcao,
    }
}

fn decode_callsign(bytes: [u8; 8]) -> Option<String> {
    let value = core::str::from_utf8(&bytes).ok()?.trim_end();
    (!value.is_empty()).then(|| value.to_owned())
}

fn emergency_state(code: u8) -> EmergencyState {
    match code {
        0 => EmergencyState::None,
        1 => EmergencyState::General,
        2 => EmergencyState::Medical,
        3 => EmergencyState::MinimumFuel,
        4 => EmergencyState::NoCommunication,
        5 => EmergencyState::UnlawfulInterference,
        6 => EmergencyState::DownedAircraft,
        _ => EmergencyState::Reserved,
    }
}

fn emitter_category(code: u8) -> Option<EmitterCategory> {
    match code {
        1 => Some(EmitterCategory::LightAircraft),
        2 => Some(EmitterCategory::SmallAircraft),
        3 => Some(EmitterCategory::LargeAircraft),
        4 => Some(EmitterCategory::HighVortexAircraft),
        5 => Some(EmitterCategory::HeavyAircraft),
        6 => Some(EmitterCategory::HighPerformanceAircraft),
        7 => Some(EmitterCategory::Rotorcraft),
        9 => Some(EmitterCategory::Glider),
        10 => Some(EmitterCategory::LighterThanAir),
        11 => Some(EmitterCategory::Parachutist),
        12 => Some(EmitterCategory::Ultralight),
        14 => Some(EmitterCategory::UncrewedAircraft),
        15 => Some(EmitterCategory::SpaceVehicle),
        17 => Some(EmitterCategory::SurfaceEmergencyVehicle),
        18 => Some(EmitterCategory::SurfaceServiceVehicle),
        19 => Some(EmitterCategory::PointObstacle),
        20 => Some(EmitterCategory::ClusterObstacle),
        21 => Some(EmitterCategory::LineObstacle),
        _ => None,
    }
}
