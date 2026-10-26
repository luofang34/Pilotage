//! Admission of an approval of a compiled plan: it binds the plan's digest,
//! it is valid now, and its adaptation envelope covers the plan.

use pilotage_protocol::wire;

use super::LaneConfig;
use super::compile::Compiled;
use super::target::valid_approval;

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
    let envelope = valid_approval(approval, now_unix_ms)?;
    // A recovery the plan cannot fly is refused, not implied.
    let returns_home = envelope.on_expiry() == wire::MissionExpiryPolicy::ReturnHome
        || envelope.on_control_lost() == wire::MissionControlLossPolicy::ReturnHome;
    if returns_home && !compiled.flies() {
        return Err(
            "a plan of operations has no way home; approve it to hold on expiry and on a lost control"
                .to_owned(),
        );
    }
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
