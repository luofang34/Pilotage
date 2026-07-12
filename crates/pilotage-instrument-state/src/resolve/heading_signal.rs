//! Presentation of the independent heading sample and of true-north
//! quantities in the display reference (NAV-01).

use crate::aircraft::AircraftState;
use crate::heading::{HeadingReference, convert_heading, wrap_2pi};
use crate::signal::{FreshnessPolicy, Sig, SignalStatus};
use crate::validate::StateIntegrity;

use super::{Trust, Wind, finite, group_freshness};

/// Heading resolved from the independent sample (NAV-01): the value,
/// its declared reference, and nothing else — attitude yaw never feeds
/// this. A missing sample resolves `Missing` and the compass rose fails
/// visibly instead of freezing on a fabricated heading.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ResolvedHeading {
    /// Heading in radians clockwise from the declared north; quiet zero
    /// behind a hidden status.
    pub value_rad: Sig<f32>,
    /// The reference every HSI angular quantity is presented in.
    pub reference: HeadingReference,
}

/// Resolves the independent heading sample. Its status folds the same
/// trust chain as every estimate group; the reference passes through
/// typed. An unknown reference fails; absence is `Missing` — the rose
/// fails visibly, never a frozen plausible heading, and attitude yaw is
/// not consulted at any pitch.
pub(super) fn heading_resolved(
    state: &AircraftState,
    policy: &FreshnessPolicy,
    trust: &Trust,
    integrity: &StateIntegrity,
) -> ResolvedHeading {
    let has = state.heading.data.is_some();
    let fresh = group_freshness(policy, has, state.heading.age_ms);
    let status = trust.fold(has, fresh, integrity.heading, state.valid.heading);
    let sample = state.heading.data.unwrap_or(crate::heading::HeadingSample {
        heading_rad: 0.0,
        reference: HeadingReference::Unknown,
    });
    let reference = if has {
        sample.reference
    } else {
        HeadingReference::Unknown
    };
    ResolvedHeading {
        value_rad: finite(Sig::with_status(wrap_2pi(sample.heading_rad), status)),
        reference,
    }
}

/// A usable variation sample, or `None` when absent, stale, faulted, or
/// undeclared — the caller then degrades instead of converting.
fn usable_variation(
    state: &AircraftState,
    policy: &FreshnessPolicy,
) -> Option<crate::heading::MagneticVariation> {
    let fresh = policy.status_for_age(state.variation.age_ms);
    match (state.variation.data, state.valid.variation) {
        (Some(sample), true) if fresh.shows_value() => Some(sample),
        _ => None,
    }
}

/// Presents a NED-derived (true-north) angle in the display reference.
/// A magnetic display without a usable variation degrades the quantity
/// to `Failed` rather than mixing references on one rose.
pub(super) fn presented_true(
    sig: Sig<f32>,
    display: HeadingReference,
    state: &AircraftState,
) -> Sig<f32> {
    if !sig.status.shows_value() || display == HeadingReference::Unknown {
        return sig;
    }
    let variation = usable_variation(state, &FreshnessPolicy::default());
    match convert_heading(
        sig.value,
        HeadingReference::True,
        display,
        variation.as_ref(),
    ) {
        Ok(value) => Sig::with_status(value, sig.status),
        Err(_) => Sig::with_status(0.0, SignalStatus::Failed),
    }
}

pub(super) fn presented_wind(
    wind: Sig<Wind>,
    display: HeadingReference,
    state: &AircraftState,
) -> Sig<Wind> {
    if !wind.status.shows_value() || display == HeadingReference::Unknown {
        return wind;
    }
    let converted = presented_true(
        Sig::with_status(wind.value.from_rad, wind.status),
        display,
        state,
    );
    if converted.status.shows_value() {
        Sig::with_status(
            Wind {
                from_rad: converted.value,
                speed_mps: wind.value.speed_mps,
            },
            wind.status,
        )
    } else {
        Sig::with_status(
            Wind {
                from_rad: 0.0,
                speed_mps: 0.0,
            },
            SignalStatus::Failed,
        )
    }
}
