//! `--site` parsing: names and stated sites, and refusals that name the
//! choices.

#![allow(clippy::expect_used, clippy::panic)]

use super::{NAMED_SITES, ZURICH};
use crate::error::XtaskError;

#[test]
fn named_and_stated_sites_parse() {
    for site in NAMED_SITES {
        assert_eq!(super::parse(site.name).expect("named"), site);
    }
    assert_eq!(super::parse("zurich").expect("zurich"), ZURICH);
    let stated = super::parse("46.5, 7.25,1200,270").expect("stated");
    assert_eq!(
        (
            stated.name,
            stated.latitude_deg,
            stated.longitude_deg,
            stated.elevation_m,
            stated.heading_deg
        ),
        ("custom", 46.5, 7.25, 1_200.0, 270.0)
    );
}

#[test]
fn a_malformed_site_is_refused_naming_the_choices() {
    for spec in [
        "n52",
        "46.5,7.25,1200",
        "46.5,7.25,1200,270,1",
        "46.5,7.25,NaN,270",
        "91,7.25,1200,270",
        "46.5,181,1200,270",
        "46.5,7.25,1200,360",
        "46.5,7.25,1200,-1",
    ] {
        let Err(XtaskError::Usage { message }) = super::parse(spec) else {
            panic!("{spec:?} must be refused");
        };
        assert!(message.contains("n51, zurich"), "{spec:?}: {message}");
    }
}
