#![allow(clippy::expect_used, clippy::panic)]

use super::{RawReportDecoder, RawReportError};
use crate::{
    DeviceInfo, NeutralPosition, RawReportAxisField, RawReportLayout, SourceAxisContract,
    SourceAxisRange,
};

fn signed_contract() -> SourceAxisContract {
    SourceAxisContract {
        schema_version: 1,
        device: DeviceInfo {
            vendor_id: 1,
            product_id: 2,
            product: Some("Test HID".to_owned()),
        },
        raw_report_layout: Some(RawReportLayout {
            report_byte_count: 2,
            report_id: None,
            axes: vec![RawReportAxisField {
                source_index: 0,
                bit_offset: 3,
                bit_width: 5,
                signed: true,
            }],
        }),
        axes: vec![SourceAxisRange {
            source_index: 0,
            minimum: -16.0,
            maximum: 15.0,
            neutral_position: NeutralPosition::Centered,
        }],
    }
}

#[test]
fn decodes_a_signed_field_from_the_lsb0_bit_stream() {
    let decoder = RawReportDecoder::new(&signed_contract()).expect("valid signed layout");
    assert_eq!(decoder.decode(&[0xc8, 0]).expect("report decodes"), [-7.0]);
}

#[test]
fn rejects_a_field_that_crosses_the_report_boundary() {
    let mut contract = signed_contract();
    let layout = contract.raw_report_layout.as_mut().expect("layout");
    layout.report_byte_count = 1;
    layout.axes[0].bit_offset = 4;
    assert!(matches!(
        RawReportDecoder::new(&contract),
        Err(RawReportError::InvalidAxisField { .. })
    ));
}

#[test]
fn rejects_a_source_range_outside_the_raw_integer_field() {
    let mut contract = signed_contract();
    contract.axes[0].minimum = -17.0;
    assert!(matches!(
        RawReportDecoder::new(&contract),
        Err(RawReportError::InvalidSourceRange { .. })
    ));
}
