use std::cell::Cell;

use crate::{
    ComposingSituationViewV1, DomainSelectionV1, MissingDataReasonV1, MonotonicStampV1,
    QueryAxisV1, QueryRequirementsV1, SITUATION_VIEW_SCHEMA_VERSION, SituationViewError,
    SituationViewQueryV1, SituationViewRequestV1, SituationViewV1, SnapshotCaptureV1,
    SnapshotSourceV1, TimeQueryV1, UtcInstantV1,
};

struct CountingSource {
    captures: Cell<u32>,
}

impl SnapshotSourceV1 for CountingSource {
    fn capture(&self, _selection: &DomainSelectionV1, _time: &TimeQueryV1) -> SnapshotCaptureV1 {
        self.captures.set(self.captures.get().wrapping_add(1));
        SnapshotCaptureV1::Missing {
            reason: MissingDataReasonV1::DomainUnavailable,
        }
    }
}

fn request() -> SituationViewRequestV1 {
    SituationViewRequestV1::attach(
        SituationViewQueryV1 {
            time: TimeQueryV1 {
                axis: QueryAxisV1::ValidTime,
                evaluation_utc: UtcInstantV1 {
                    unix_seconds: 1,
                    subsecond_nanoseconds: 0,
                },
            },
            domains: vec![
                DomainSelectionV1 {
                    domain: "surveillance".to_string(),
                    subject: "all_tracks".to_string(),
                },
                DomainSelectionV1 {
                    domain: "airmass".to_string(),
                    subject: "regional_weather".to_string(),
                },
            ],
            requirements: QueryRequirementsV1::default(),
        },
        MonotonicStampV1 {
            clock_id: "host".to_string(),
            nanoseconds: 1,
        },
    )
}

#[test]
fn composer_captures_each_selected_domain_once() {
    let view = ComposingSituationViewV1::new(CountingSource {
        captures: Cell::new(0),
    });
    let result = view.query(&request()).expect("request must be valid");
    assert_eq!(result.domains.len(), 2);
    assert_eq!(view.source().captures.get(), 2);
}

#[test]
fn composer_rejects_an_unknown_request_version() {
    let view = ComposingSituationViewV1::new(CountingSource {
        captures: Cell::new(0),
    });
    let mut request = request();
    request.schema_version = SITUATION_VIEW_SCHEMA_VERSION.wrapping_add(1);
    assert!(matches!(
        view.query(&request),
        Err(SituationViewError::UnsupportedSchemaVersion { .. })
    ));
    assert_eq!(view.source().captures.get(), 0);
}
