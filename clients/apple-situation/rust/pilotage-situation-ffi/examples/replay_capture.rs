//! Replay a recorded AeroLink reception file through the situation client pipeline.
//!
//! A radio, a transmitter in range, and an iPad are needed to see traffic and weather
//! reach a map. A recording removes all three from the loop: the same reception events
//! run through Surveillance, Airmass, and the presentation adapter here, and the counts
//! this prints say how far each product travelled.
//!
//! The harness writes the file. Turn on "Record receptions" and collect
//! `Documents/receptions-*.ndjson`.
//!
//! ```text
//! cargo run --example replay_capture -- <capture.ndjson>
//! ```

use std::io::{BufRead, BufReader, Write};
use std::path::{Path, PathBuf};

use pilotage_situation_ffi::{PresentationSession, RadioDomainSession, WeatherStationPosition};

#[derive(Default)]
struct Tally {
    lines: u64,
    events_consumed: u64,
    traffic_observations: u64,
    traffic_refusals: u64,
    weather_products: u64,
    track_records: u64,
    weather_records: u64,
    rejected: u64,
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let path: PathBuf = std::env::args_os()
        .nth(1)
        .ok_or("usage: replay_capture <capture.ndjson>")?
        .into();
    // A weather report names its station and carries no position. The position comes from
    // navigation data, and this replay has none. Pass --synthesize-weather-positions to
    // place every station seen at an invented point, which separates a weather path that
    // cannot decode from one that only lacks positions.
    let synthesize = std::env::args().any(|arg| arg == "--synthesize-weather-positions");

    let radio = RadioDomainSession::new()?;
    let presentation = PresentationSession::new();
    let mut run = Run::default();
    replay(&path, &radio, &presentation, synthesize, &mut run)?;

    if synthesize && !run.stations.is_empty() {
        run.display = Some(place_stations(&presentation, &run.stations)?);
    }
    report(&path, &run)
}

/// Everything one replay observed.
#[derive(Default)]
struct Run {
    tally: Tally,
    display: Option<pilotage_situation_ffi::DisplayBatch>,
    first_rejection: Option<String>,
    stations: std::collections::BTreeSet<String>,
}

/// Push every reception in the file through the radio and presentation stages.
fn replay(
    path: &Path,
    radio: &RadioDomainSession,
    presentation: &PresentationSession,
    synthesize: bool,
    run: &mut Run,
) -> Result<(), Box<dyn std::error::Error>> {
    // The recording carries no wall clock, so drive one forward. Lifecycle rules retire a
    // track and expire a product on elapsed time, and a clock that never advances would
    // keep every feature alive and overstate the result.
    let mut utc_millis: i64 = 0;
    let mut monotonic_micros: u64 = 0;

    for line in BufReader::new(std::fs::File::open(path)?).lines() {
        let line = line?;
        if line.trim().is_empty() {
            continue;
        }
        run.tally.lines += 1;
        utc_millis += 100;
        monotonic_micros += 100_000;

        let batch = match radio.accept_reception_event(line, 1, utc_millis, monotonic_micros) {
            Ok(batch) => batch,
            Err(error) => {
                run.tally.rejected += 1;
                if run.first_rejection.is_none() {
                    run.first_rejection = Some(describe(&error));
                }
                continue;
            }
        };
        run.tally.events_consumed += batch.events_consumed;
        run.tally.traffic_observations += batch.traffic_observations;
        run.tally.traffic_refusals += batch.traffic_refusals;
        run.tally.weather_products += batch.weather_products;
        run.tally.track_records += batch.track_records.len() as u64;
        run.tally.weather_records += batch.weather_records.len() as u64;

        for record in batch.track_records {
            run.display = Some(presentation.accept_track_record(record, monotonic_micros)?);
        }
        for record in batch.weather_records {
            if synthesize {
                collect_station_ids(&record, &mut run.stations);
            }
            run.display = Some(presentation.accept_weather_record(record, monotonic_micros)?);
        }
    }
    Ok(())
}

/// Place every station the run saw, so the weather path can be seen end to end.
fn place_stations(
    presentation: &PresentationSession,
    stations: &std::collections::BTreeSet<String>,
) -> Result<pilotage_situation_ffi::DisplayBatch, Box<dyn std::error::Error>> {
    let positions: Vec<WeatherStationPosition> = stations
        .iter()
        .enumerate()
        .map(|(index, station_id)| WeatherStationPosition {
            station_id: station_id.clone(),
            latitude_deg: 40.0 + (index as f64) * 0.05,
            longitude_deg: -75.0 - (index as f64) * 0.05,
        })
        .collect();
    Ok(presentation.replace_weather_station_positions(positions)?)
}

/// Write what each stage carried.
fn report(path: &Path, run: &Run) -> Result<(), Box<dyn std::error::Error>> {
    let stdout = std::io::stdout();
    let mut out = stdout.lock();
    writeln!(out, "capture            {}", path.display())?;
    writeln!(out, "lines read         {}", run.tally.lines)?;
    writeln!(out, "events consumed    {}", run.tally.events_consumed)?;
    writeln!(out, "events rejected    {}", run.tally.rejected)?;
    writeln!(out, "traffic observed   {}", run.tally.traffic_observations)?;
    writeln!(out, "traffic refused    {}", run.tally.traffic_refusals)?;
    writeln!(out, "weather products   {}", run.tally.weather_products)?;
    writeln!(out, "track records      {}", run.tally.track_records)?;
    writeln!(out, "weather records    {}", run.tally.weather_records)?;
    if let Some(reason) = &run.first_rejection {
        writeln!(out, "first rejection    {reason}")?;
    }
    if !run.stations.is_empty() {
        writeln!(out, "stations placed    {}", run.stations.len())?;
    }
    match &run.display {
        Some(batch) => {
            writeln!(out, "map points         {}", batch.points.len())?;
            let mut by_layer: std::collections::BTreeMap<&str, usize> = Default::default();
            for point in &batch.points {
                *by_layer.entry(point.layer_id.as_str()).or_default() += 1;
            }
            for (layer, count) in by_layer {
                writeln!(out, "  layer {layer:<22} {count}")?;
            }
            writeln!(out, "map shapes         {}", batch.shapes.len())?;
            writeln!(
                out,
                "positionless       {}",
                batch.positionless_traffic.len()
            )?;
            writeln!(out, "layers             {}", batch.layers.len())?;
        }
        None => writeln!(
            out,
            "no display batch: nothing reached the presentation adapter"
        )?,
    }
    Ok(())
}

/// Collect every station identity a weather record names.
fn collect_station_ids(record: &str, out: &mut std::collections::BTreeSet<String>) {
    let Ok(value) = serde_json::from_str::<serde_json::Value>(record) else {
        return;
    };
    collect_from_value(&value, out);
}

fn collect_from_value(value: &serde_json::Value, out: &mut std::collections::BTreeSet<String>) {
    match value {
        serde_json::Value::Object(map) => {
            for (key, child) in map {
                if key == "station_id" {
                    collect_strings(child, out);
                }
                collect_from_value(child, out);
            }
        }
        serde_json::Value::Array(items) => {
            for item in items {
                collect_from_value(item, out);
            }
        }
        _ => {}
    }
}

/// A field value is wrapped in its observation state, so take any string beneath it.
fn collect_strings(value: &serde_json::Value, out: &mut std::collections::BTreeSet<String>) {
    match value {
        serde_json::Value::String(text) => {
            out.insert(text.clone());
        }
        serde_json::Value::Object(map) => {
            for child in map.values() {
                collect_strings(child, out);
            }
        }
        serde_json::Value::Array(items) => {
            for item in items {
                collect_strings(item, out);
            }
        }
        _ => {}
    }
}

/// Describe an error with the causes beneath it.
///
/// The top message names the product and the time and not the reason, and the reason is
/// what says whether a rejection is a product this build does not read or a fault.
fn describe(error: &dyn std::error::Error) -> String {
    let mut text = error.to_string();
    let mut source = error.source();
    while let Some(cause) = source {
        text.push_str(" <- ");
        text.push_str(&cause.to_string());
        source = cause.source();
    }
    text
}
