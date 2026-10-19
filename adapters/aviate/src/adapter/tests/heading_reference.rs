//! The FC's geomagnetic reference: the stamped variation on the estimate
//! lane and the heading reference the capability advertises from it.

use std::time::{Duration, Instant};

use pilotage_adapter_api::{MagneticVariationSource, VehicleAdapter};
use pilotage_mavlink::link::GeomagneticReferenceUpdate;
use pilotage_protocol::{HeadingReference, VehicleId};

use super::state_with;
use crate::adapter::AviateAdapter;

fn adapter_reporting(source: Option<u8>, age: Duration) -> AviateAdapter {
    let state = state_with(Duration::ZERO, Duration::ZERO);
    if let Some(source) = source {
        let mut latest = state.lock().expect("link state");
        latest.geomagnetic_reference = Some(GeomagneticReferenceUpdate {
            declination_rad: if source == 0 { f32::NAN } else { -0.2113 },
            source,
            sequence: 3,
            received_since_start_ns: 4_000_000_000,
            received_at: Instant::now().checked_sub(age).expect("monotonic clock"),
        });
    }
    AviateAdapter::from_state(VehicleId::new(1), state)
}

fn advertised(adapter: &AviateAdapter) -> Option<HeadingReference> {
    adapter.capabilities().vehicles[0].heading_reference
}

#[test]
fn a_model_reference_rides_the_estimate_lane_as_fc_model() {
    let mut adapter = adapter_reporting(Some(1), Duration::ZERO);
    let batch = adapter.sample_telemetry();
    let avionics = batch.samples[0].avionics.expect("estimate lane");
    let variation = avionics.magnetic_variation.expect("variation");
    assert_eq!(variation.source, MagneticVariationSource::FcModel);
    assert_eq!(variation.declination_rad, -0.2113);
    assert_eq!(variation.stamp.sequence, 3);
    assert_eq!(variation.stamp.acquired_at_ns, 4_000_000_000);
}

#[test]
fn a_stale_reference_is_withheld_from_the_estimate_lane() {
    let mut adapter = adapter_reporting(Some(1), Duration::from_secs(10));
    let batch = adapter.sample_telemetry();
    let avionics = batch.samples[0].avionics.expect("estimate lane");
    assert!(avionics.magnetic_variation.is_none());
}

#[test]
fn the_advertised_heading_reference_follows_the_fc_source() {
    assert_eq!(
        advertised(&adapter_reporting(Some(1), Duration::ZERO)),
        Some(HeadingReference::True)
    );
    assert_eq!(
        advertised(&adapter_reporting(Some(2), Duration::ZERO)),
        Some(HeadingReference::True)
    );
    assert_eq!(
        advertised(&adapter_reporting(Some(0), Duration::ZERO)),
        Some(HeadingReference::Magnetic)
    );
    // An aged report still states the FC's configuration.
    assert_eq!(
        advertised(&adapter_reporting(Some(1), Duration::from_secs(10))),
        Some(HeadingReference::True)
    );
}

#[test]
fn no_or_unknown_reference_leaves_the_heading_reference_unstated() {
    assert_eq!(advertised(&adapter_reporting(None, Duration::ZERO)), None);
    assert_eq!(
        advertised(&adapter_reporting(Some(9), Duration::ZERO)),
        None
    );
}
