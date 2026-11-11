//! Each way the readers of a fix can fall out of step is refused.

use std::path::{Path, PathBuf};

use super::{Sources, refusals};

fn root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../..")
}

fn sources() -> Sources {
    Sources::read(&root()).expect("the geodetic files read")
}

/// Applies `edit` to one source, proves it changed, and checks the guard
/// refuses the result.
fn refused(what: &str, edit: impl FnOnce(&mut Sources) -> bool) {
    let mut edited = sources();
    assert!(edit(&mut edited), "the case did not apply: {what}");
    assert!(!refusals(&edited).is_empty(), "the guard accepted {what}");
}

fn replace(text: &mut String, from: &str, to: &str) -> bool {
    let before = text.clone();
    *text = text.replacen(from, to, 1);
    *text != before
}

#[test]
fn the_repository_passes() {
    assert_eq!(refusals(&sources()), Vec::<String>::new());
}

#[test]
fn a_datum_the_proto_does_not_name_is_refused() {
    refused("a horizontal datum the proto does not name", |s| {
        replace(
            &mut s.datum,
            "            3 => Some(Self::Itrf2014),",
            "            3 => Some(Self::Itrf2014),\n            4 => Some(Self::Etrs89),",
        )
    });
}

#[test]
fn a_browser_that_accepts_an_unknown_code_is_refused() {
    refused(
        "a browser that accepts a datum code the crate cannot produce",
        |s| {
            replace(
                &mut s.decoder,
                "horizontalDatum > 3",
                "horizontalDatum > 99",
            )
        },
    );
}

#[test]
fn a_cast_datum_is_refused() {
    refused("a datum cast onto the wire instead of converted", |s| {
        replace(
            &mut s.mapper,
            "horizontal_datum: u32::from(position.horizontal_datum.to_u8()),",
            "horizontal_datum: position.horizontal_datum as u32,",
        )
    });
}

#[test]
fn a_commented_out_ci_test_is_refused() {
    refused("a CI workflow whose geodetic test is commented out", |s| {
        replace(
            &mut s.ci,
            "run: node clients/web/geodetic-fix.test.mjs",
            "# run: node clients/web/geodetic-fix.test.mjs",
        )
    });
}

#[test]
fn a_client_decoder_that_drops_the_fix_is_refused() {
    refused(
        "a client decoder that does not surface the estimator's fix",
        |s| {
            replace(
                &mut s.wasm_decoder,
                "        geodetic,\n        geodetic_stamp,\n",
                "",
            )
        },
    );
    refused(
        "a client decoder that wraps an unnormalized longitude",
        |s| {
            replace(
                &mut s.wasm_decoder,
                "fix.longitude_deg < 180.0",
                "fix.longitude_deg < 1e9",
            )
        },
    );
}

#[test]
fn a_producer_without_its_gates_is_refused() {
    refused(
        "a mapping that re-checks a position and sends its own copy",
        |s| {
            replace(
                &mut s.mapper,
                "position.normalized()",
                "position.validate()",
            )
        },
    );
    refused("a producer that does not gate the estimate lane", |s| {
        replace(
            &mut s.mapper,
            "fix.stamp.role == SourceRole::OperationalEstimate",
            "fix.stamp.role == SourceRole::FcState",
        )
    });
    refused("a producer that does not gate the truth lane", |s| {
        replace(
            &mut s.mapper,
            "fix.stamp.role == SourceRole::SimulationTruth",
            "true",
        )
    });
    refused("a lane that reaches the wire without the mapping", |s| {
        replace(&mut s.mapper, ".and_then(geodetic_to_wire);", ";")
    });
}

#[test]
fn a_broken_sensor_join_is_refused() {
    refused("a truth sample that drops the paired fix", |s| {
        s.attachments.as_mut().is_some_and(|text| {
            replace(text, "sample.geodetic = self.paired_fix(sample.stamp);", "")
        })
    });
    refused(
        "a sensor topic that names no sensor the model publishes",
        |s| {
            s.camera.as_mut().is_some_and(|text| {
                replace(text, "sensor/navsat_sensor/navsat", "sensor/typo/navsat")
            })
        },
    );
}
