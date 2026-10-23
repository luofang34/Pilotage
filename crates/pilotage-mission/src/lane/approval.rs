//! Admission of an approval: it binds the proposal's digest, permits
//! execution, has not expired, and its adaptation envelope covers the plan.

use pilotage_protocol::wire;

use super::LaneConfig;
use super::compile::Compiled;

/// Height a plan may exceed an envelope by through rounding, metres.
const HEIGHT_TOLERANCE_M: f64 = 1e-6;

/// Checks `approval` against the compiled proposal.
pub(super) fn admit(
    approval: &wire::MissionApproval,
    compiled: &Compiled,
    lane: &LaneConfig,
    now_unix_ms: u64,
) -> Result<(), String> {
    if approval.proposal_digest != compiled.proposal.digest {
        return Err("the approval binds another proposal digest".to_owned());
    }
    still_valid(approval, now_unix_ms)?;
    let envelope = approval
        .envelope
        .as_ref()
        .ok_or("the approval has no adaptation envelope")?;
    if compiled.height_m > envelope.max_height_m + HEIGHT_TOLERANCE_M {
        return Err(format!(
            "the plan flies at {:.1} m; the envelope allows {:.1} m",
            compiled.height_m, envelope.max_height_m
        ));
    }
    if !envelope.bound.is_empty() {
        let outside = compiled
            .proposal
            .tasks
            .iter()
            .flat_map(|task| task.path.iter())
            .find(|point| !inside(point, &envelope.bound));
        if let Some(point) = outside {
            return Err(format!(
                "the plan leaves the approved bound at {:.6}, {:.6}",
                point.latitude_deg, point.longitude_deg
            ));
        }
    }
    let energy_wh = compiled
        .proposal
        .estimates
        .as_ref()
        .map_or(0.0, |e| e.energy_wh);
    if let Some(usable_wh) = lane
        .usable_energy_wh
        .filter(|wh| *wh > 0.0 && energy_wh > 0.0)
    {
        let allowed = usable_wh * (1.0 - envelope.min_reserve_fraction.clamp(0.0, 1.0));
        if energy_wh > allowed {
            return Err(format!(
                "the plan needs {energy_wh:.1} Wh; the reserve leaves {allowed:.1} Wh"
            ));
        }
    }
    Ok(())
}

/// The approval permits execution and has not expired.
pub(super) fn still_valid(
    approval: &wire::MissionApproval,
    now_unix_ms: u64,
) -> Result<(), String> {
    let envelope = approval
        .envelope
        .as_ref()
        .ok_or("the approval has no adaptation envelope")?;
    if !envelope.permit_execute {
        return Err("the approval does not permit execution".to_owned());
    }
    if envelope.expires_unix_ms == 0 || envelope.expires_unix_ms <= now_unix_ms {
        return Err("the approval has no expiry in the future".to_owned());
    }
    Ok(())
}

/// Ray-casting point-in-polygon test in degree space, adequate for areas
/// of a few kilometres away from the poles and the antimeridian.
pub(super) fn inside(point: &wire::GeoPoint, polygon: &[wire::GeoPoint]) -> bool {
    let (x, y) = (point.longitude_deg, point.latitude_deg);
    let mut inside = false;
    let mut previous = polygon.len().wrapping_sub(1);
    for (index, vertex) in polygon.iter().enumerate() {
        let Some(other) = polygon.get(previous) else {
            break;
        };
        let (xi, yi, xj, yj) = (
            vertex.longitude_deg,
            vertex.latitude_deg,
            other.longitude_deg,
            other.latitude_deg,
        );
        if (yi > y) != (yj > y) && x < (xj - xi) * (y - yi) / (yj - yi) + xi {
            inside = !inside;
        }
        previous = index;
    }
    inside
}
