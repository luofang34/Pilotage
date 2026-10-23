//! Injected camera faults, to test a frame-quality judge.
//!
//! The text form is a list of faults separated by `;`. Each fault is
//! `KIND@WINDOW`:
//!
//! - `KIND` is `dark`, `blur`, or `overexposed`.
//! - `WINDOW` is a time window `T0-T1s` in seconds of simulation time from
//!   the adapter start, for example `dark@30-45s`, or a geographic box
//!   `LAT0,LON0,LAT1,LON1` in degrees that contains the vehicle position,
//!   for example `blur@40.330,-74.670,40.335,-74.665`.

use core::fmt;

/// What a fault does to the image.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FaultKind {
    /// The image is nearly black.
    Dark,
    /// The image is out of focus.
    Blur,
    /// The image is washed out to white.
    Overexposed,
}

/// When or where a fault is active.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum FaultWindow {
    /// Between two simulation times, seconds; the end is exclusive.
    Time {
        /// Start, seconds.
        start_s: f64,
        /// End, seconds.
        end_s: f64,
    },
    /// While the vehicle is inside a latitude and longitude box, degrees.
    Area {
        /// South edge.
        south: f64,
        /// West edge.
        west: f64,
        /// North edge.
        north: f64,
        /// East edge.
        east: f64,
    },
}

/// One injected fault.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct CameraFault {
    /// The effect.
    pub kind: FaultKind,
    /// The activation window.
    pub window: FaultWindow,
}

/// A fault specification that does not parse.
#[derive(Debug, Clone, PartialEq, thiserror::Error)]
#[error("invalid camera fault {entry:?}: {reason}")]
pub struct FaultParseError {
    /// The rejected entry.
    pub entry: String,
    /// What is wrong with it.
    pub reason: &'static str,
}

impl CameraFault {
    /// Whether the fault is active at `time_s` with the vehicle at
    /// `lat_deg`, `lon_deg`.
    #[must_use]
    pub fn active(&self, time_s: f64, lat_deg: f64, lon_deg: f64) -> bool {
        match self.window {
            FaultWindow::Time { start_s, end_s } => time_s >= start_s && time_s < end_s,
            FaultWindow::Area {
                south,
                west,
                north,
                east,
            } => (south..=north).contains(&lat_deg) && (west..=east).contains(&lon_deg),
        }
    }
}

/// Parses the text form in the module documentation. An empty text gives
/// no faults.
///
/// # Errors
/// Returns the first entry that does not parse.
pub fn parse_faults(text: &str) -> Result<Vec<CameraFault>, FaultParseError> {
    text.split(';')
        .map(str::trim)
        .filter(|entry| !entry.is_empty())
        .map(parse_fault)
        .collect()
}

fn parse_fault(entry: &str) -> Result<CameraFault, FaultParseError> {
    let fail = |reason| FaultParseError {
        entry: entry.to_owned(),
        reason,
    };
    let (kind, window) = entry
        .split_once('@')
        .ok_or_else(|| fail("expected KIND@WINDOW"))?;
    let kind = match kind.trim() {
        "dark" => FaultKind::Dark,
        "blur" => FaultKind::Blur,
        "overexposed" => FaultKind::Overexposed,
        _ => return Err(fail("the kind must be dark, blur, or overexposed")),
    };
    let window = window.trim();
    let window = match window.strip_suffix('s') {
        Some(times) => parse_time(times).map_err(fail)?,
        None => parse_area(window).map_err(fail)?,
    };
    Ok(CameraFault { kind, window })
}

fn parse_time(times: &str) -> Result<FaultWindow, &'static str> {
    let (start, end) = times.split_once('-').ok_or("a time window is T0-T1s")?;
    let parse = |value: &str| {
        value
            .trim()
            .parse::<f64>()
            .map_err(|_| "a time is a number of seconds")
    };
    let (start_s, end_s) = (parse(start)?, parse(end)?);
    if !start_s.is_finite() || !end_s.is_finite() || start_s < 0.0 || end_s <= start_s {
        return Err("a time window needs 0 <= T0 < T1");
    }
    Ok(FaultWindow::Time { start_s, end_s })
}

fn parse_area(area: &str) -> Result<FaultWindow, &'static str> {
    let values = area
        .split(',')
        .map(|value| value.trim().parse::<f64>())
        .collect::<Result<Vec<_>, _>>()
        .map_err(|_| "a box is LAT0,LON0,LAT1,LON1 in degrees")?;
    let [lat0, lon0, lat1, lon1] = values.as_slice() else {
        return Err("a box is LAT0,LON0,LAT1,LON1 in degrees");
    };
    let latitudes = (-90.0..=90.0).contains(lat0) && (-90.0..=90.0).contains(lat1);
    let longitudes = (-180.0..=180.0).contains(lon0) && (-180.0..=180.0).contains(lon1);
    if !latitudes || !longitudes {
        return Err("a box corner is outside the latitude or longitude range");
    }
    if lat0 == lat1 || lon0 == lon1 {
        return Err("a box needs two different corners");
    }
    Ok(FaultWindow::Area {
        south: lat0.min(*lat1),
        west: lon0.min(*lon1),
        north: lat0.max(*lat1),
        east: lon0.max(*lon1),
    })
}

impl fmt::Display for FaultKind {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::Dark => "dark",
            Self::Blur => "blur",
            Self::Overexposed => "overexposed",
        })
    }
}

/// Applies `kind` to an RGB image of `width` by `height` pixels.
pub fn apply(kind: FaultKind, rgb: &mut [u8], width: usize, height: usize) {
    match kind {
        FaultKind::Dark => {
            for value in rgb.iter_mut() {
                *value = (f32::from(*value) * 0.1) as u8;
            }
        }
        FaultKind::Overexposed => {
            for value in rgb.iter_mut() {
                *value = (f32::from(*value) * 2.5 + 110.0).min(255.0) as u8;
            }
        }
        FaultKind::Blur => {
            for _ in 0..2 {
                box_blur(rgb, width, height, 6);
            }
        }
    }
}

/// A separable box blur of radius `radius` pixels.
fn box_blur(rgb: &mut [u8], width: usize, height: usize, radius: usize) {
    if width == 0 || height == 0 || rgb.len() != width * height * 3 {
        return;
    }
    let mut scratch = vec![0u8; rgb.len()];
    blur_pass(rgb, &mut scratch, width, height, radius, true);
    blur_pass(&scratch, rgb, width, height, radius, false);
}

fn blur_pass(
    source: &[u8],
    target: &mut [u8],
    width: usize,
    height: usize,
    radius: usize,
    horizontal: bool,
) {
    let (lines, length) = if horizontal {
        (height, width)
    } else {
        (width, height)
    };
    let index = |line: usize, at: usize| {
        if horizontal {
            (line * width + at) * 3
        } else {
            (at * width + line) * 3
        }
    };
    for line in 0..lines {
        for at in 0..length {
            let from = at.saturating_sub(radius);
            let to = (at + radius).min(length - 1);
            let mut sum = [0u32; 3];
            for sample in from..=to {
                let i = index(line, sample);
                for channel in 0..3 {
                    sum[channel] += u32::from(source[i + channel]);
                }
            }
            let count = (to - from + 1) as u32;
            let i = index(line, at);
            for channel in 0..3 {
                target[i + channel] = (sum[channel] / count) as u8;
            }
        }
    }
}

#[cfg(test)]
mod tests;
