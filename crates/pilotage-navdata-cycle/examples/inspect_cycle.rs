//! Report what one published cycle contains.
//!
//! ```text
//! cargo run -p pilotage-navdata-cycle --example inspect_cycle -- <cycle.acnav>
//! ```

use std::io::Write;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let path = std::env::args_os()
        .nth(1)
        .ok_or("usage: inspect_cycle <cycle.acnav>")?;
    let snapshot = pilotage_navdata_cycle::load_cycle(&path)?;
    let stations = pilotage_navdata_cycle::weather_station_positions(&snapshot);

    let stdout = std::io::stdout();
    let mut out = stdout.lock();
    writeln!(out, "authority          {:?}", snapshot.cycle.authority)?;
    writeln!(out, "effective          {}", snapshot.cycle.effective_on)?;
    writeln!(
        out,
        "next effective     {}",
        snapshot.cycle.next_effective_on
    )?;
    writeln!(out, "points             {}", snapshot.points.len())?;
    writeln!(out, "airways            {}", snapshot.airways.len())?;
    writeln!(out, "runways            {}", snapshot.runways.len())?;
    writeln!(out, "airspaces          {}", snapshot.airspaces.len())?;
    writeln!(out, "weather stations   {}", stations.len())?;
    if let Some(first) = stations.first() {
        writeln!(
            out,
            "  example          {} at {:.4}, {:.4}",
            first.station_id, first.latitude_deg, first.longitude_deg
        )?;
    }
    Ok(())
}
