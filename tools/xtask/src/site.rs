//! The `--site` vocabulary: a named airfield from pilotage-world, or one
//! an operator states.

pub use pilotage_world::{DEFAULT_SITE, Ground, NAMED_SITES, Placement, Site, ZURICH};

use crate::error::XtaskError;

/// Parses `--site`: a name from [`NAMED_SITES`], or
/// `latitude_deg,longitude_deg,elevation_m,heading_deg`.
///
/// # Errors
///
/// Returns [`XtaskError::Usage`] for an unknown name, a malformed
/// list, or a value out of range.
pub fn parse(spec: &str) -> Result<Site, XtaskError> {
    if let Some(site) = Site::named(spec) {
        return Ok(site);
    }
    let usage = |reason: &str| XtaskError::Usage {
        message: format!(
            "--site {spec:?}: {reason}; give one of {} or \
             latitude_deg,longitude_deg,elevation_m,heading_deg",
            NAMED_SITES.map(|site| site.name).join(", ")
        ),
    };
    let values: Vec<f64> = spec
        .split(',')
        .map(|value| value.trim().parse::<f64>())
        .collect::<Result<_, _>>()
        .map_err(|_| usage("not a known site or a list of numbers"))?;
    let [latitude_deg, longitude_deg, elevation_m, heading_deg] = values[..] else {
        return Err(usage("a stated site has four values"));
    };
    if !values.iter().all(|value| value.is_finite()) {
        return Err(usage("every value must be finite"));
    }
    if latitude_deg.abs() > 85.0 || longitude_deg.abs() > 180.0 {
        return Err(usage("the position is off the map"));
    }
    if !(0.0..360.0).contains(&heading_deg) {
        return Err(usage("the heading must be 0 to 360 degrees"));
    }
    Ok(Site {
        name: "custom",
        latitude_deg,
        longitude_deg,
        elevation_m,
        heading_deg,
    })
}

/// Where the `index`-th vehicle on `ground` of `site` starts.
///
/// # Errors
///
/// Returns [`XtaskError::SitePosition`] for a site position the world
/// model refuses.
pub fn place(site: &Site, ground: Ground, index: usize) -> Result<Placement, XtaskError> {
    site.placement(ground, index)
        .map_err(|source| XtaskError::SitePosition {
            site: site.name,
            source,
        })
}

#[cfg(test)]
mod tests;
