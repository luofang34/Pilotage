#![allow(clippy::expect_used, clippy::panic)]

use aero_link::{
    Adsb1090Message, CorrectedUatFrame, FisBFlags, FisBMessage, FisBTime, GeoPosition,
    InformationFrame, ReceiverSource, ReceptionEvent, SourceTransport, UatFrameClass, UatMessage,
    UatReception, UatUplink,
};

use crate::{PresentationSession, RadioDomainSession, WeatherStationPosition};

const ODD_POSITION: &str = "8D40621D58C386435CC412692AD6";
const EVEN_POSITION: &str = "8D40621D58C382D690C8AC2863A7";
const METAR_PAYLOAD: &[u8] = &[
    52, 85, 1, 74, 2, 202, 24, 184, 48, 219, 29, 181, 197, 168, 49, 227, 12, 53, 45, 72, 53, 76,
    216, 2, 44, 236, 51, 194, 12, 176, 191, 28, 32, 7, 46, 121, 200,
];

#[test]
fn serialized_receptions_drive_surveillance_and_clear_on_suspend() {
    let domain = RadioDomainSession::new().expect("default radio state must be valid");
    let session = PresentationSession::new();
    let source = ReceiverSource::new(1, SourceTransport::Usb);
    let events = [
        ReceptionEvent::adsb1090(1_000, decode_adsb(ODD_POSITION)).with_source(source),
        ReceptionEvent::adsb1090(2_000, decode_adsb(EVEN_POSITION)).with_source(source),
    ];
    let mut observations = 0;
    for event in events {
        let batch = domain
            .accept_reception_event(encode_event(event), 7, 0, 2_000)
            .expect("traffic event must be accepted");
        observations += batch.traffic_observations;
        for record in batch.track_records {
            session
                .accept_track_record(record, 1_000_000)
                .expect("track record must be accepted");
        }
    }

    assert_eq!(observations, 2);
    assert_eq!(
        session
            .current_display(1_000_000)
            .expect("display state must be available")
            .points
            .len(),
        1
    );

    let suspended = session
        .clear_radio_records()
        .expect("suspension must clear radio state");
    assert!(suspended.points.is_empty());
}

#[test]
fn fis_b_reception_reaches_typed_airmass_presentation() {
    let domain = RadioDomainSession::new().expect("default radio state must be valid");
    let session = PresentationSession::new();
    session
        .replace_weather_station_positions(vec![WeatherStationPosition {
            station_id: "KJFK".into(),
            latitude_deg: 40.6413,
            longitude_deg: -73.7781,
        }])
        .expect("station position must be valid");

    let batch = domain
        .accept_reception_event(encode_event(metar_event()), 3, 0, 100)
        .expect("FIS-B event must be accepted");

    assert_eq!(batch.events_consumed, 1);
    assert_eq!(batch.traffic_observations, 0);
    assert_eq!(batch.weather_products, 1);
    assert_eq!(batch.weather_records.len(), 1);
    let display = session
        .accept_weather_record(batch.weather_records[0].clone(), 1_000_000)
        .expect("weather record must be accepted");
    assert_eq!(display.points.len(), 1);
    assert_eq!(display.points[0].label.as_deref(), Some("KJFK"));
    assert_eq!(display.points[0].style_id, "weather-mvfr");
}

#[test]
fn invalid_event_line_fails_before_domain_ingestion() {
    let domain = RadioDomainSession::new().expect("default radio state must be valid");
    let error = domain
        .accept_reception_event("{\"band\":\"invented\"}".into(), 1, 0, 1)
        .expect_err("an invalid event must fail");

    assert!(
        error
            .to_string()
            .contains("serialized AeroLink ReceptionEvent")
    );
    let next = domain
        .accept_reception_event(encode_event(metar_event()), 1, 0, 100)
        .expect("a later valid event must remain usable");
    assert_eq!(next.events_consumed, 1);
}

fn decode_adsb(hex: &str) -> Adsb1090Message {
    let bytes = hex
        .as_bytes()
        .chunks_exact(2)
        .map(|pair| {
            let text = core::str::from_utf8(pair).expect("frame must contain ASCII hex");
            u8::from_str_radix(text, 16).expect("frame must contain valid hex")
        })
        .collect::<Vec<_>>();
    Adsb1090Message::decode(&bytes, None).expect("published ADS-B frame must decode")
}

fn encode_event(event: ReceptionEvent) -> String {
    serde_json::to_string(&event).expect("typed reception event must encode")
}

fn metar_event() -> ReceptionEvent {
    let corrected_frame = CorrectedUatFrame::try_from_boxed_bytes(
        UatFrameClass::GroundUplink,
        vec![0; UatFrameClass::GroundUplink.corrected_bytes()].into_boxed_slice(),
    )
    .expect("ground uplink frame length must be valid");
    let reception = UatReception {
        rssi_raw: -30,
        receiver_timestamp: 1,
        corrected_symbols: 0,
        encoded_frame: None,
        corrected_frame,
        message: UatMessage::Uplink(UatUplink {
            station_position: GeoPosition {
                latitude: 40.0,
                longitude: -73.0,
            },
            position_valid: true,
            utc_coupled: true,
            application_data_valid: true,
            slot_id: 1,
            tisb_site_id: 1,
            information_frames: vec![InformationFrame {
                frame_type: 0,
                payload: METAR_PAYLOAD.to_vec(),
                fis_b: Some(metar_message()),
            }],
        }),
    };
    ReceptionEvent::uat978(100, reception).with_source(ReceiverSource::new(2, SourceTransport::Usb))
}

fn metar_message() -> FisBMessage {
    FisBMessage {
        flags: FisBFlags {
            a: false,
            g: false,
            p: false,
            segmented: false,
        },
        product: serde_json::from_value(serde_json::json!({
            "id": 413,
            "name": "Generic textual product type 2",
            "format": "dlac_text",
            "status": "standard"
        }))
        .expect("product metadata must match AeroLink"),
        time: FisBTime {
            month: None,
            day: None,
            hour: 12,
            minute: 30,
            second: None,
        },
        segmentation: None,
        payload: METAR_PAYLOAD.to_vec(),
        text_reports: Vec::new(),
    }
}
