use std::path::PathBuf;

use crate::Digest;
use crate::error::XPlaneTrialError;
use crate::sample::XPlaneTruthSample;

pub(crate) const PROTOCOL_VERSION: u32 = 2;
const MAXIMUM_LINE_BYTES: usize = 16 * 1024;

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Hello {
    pub protocol_version: u32,
    pub xplane_version: u32,
    pub sdk_version: u32,
    pub host_application_id: u32,
    pub source_build_id: String,
    pub bridge_build_digest: Digest,
    pub aircraft_path: PathBuf,
    pub trial_plugin_path: PathBuf,
    pub bridge_plugin_path: PathBuf,
}

#[derive(Debug, Clone, PartialEq)]
pub(crate) enum Message {
    Hello(Hello),
    Configured {
        generation: u64,
        scenario: Digest,
        condition: Digest,
    },
    Started {
        generation: u64,
        sim_time_s: f64,
        reset_generation: u64,
    },
    Stopped {
        generation: u64,
        sample_count: u64,
        sim_time_s: f64,
    },
    Resetting {
        generation: u64,
    },
    ResetComplete {
        generation: u64,
        reset_generation: u64,
        sim_time_s: f64,
    },
    WindApplied {
        generation: u64,
        condition_generation: u32,
        actual_speed_mps: f64,
        actual_direction_deg: f64,
    },
    Rewind {
        reset_generation: u64,
    },
    Active {
        generation: u64,
        scenario: Digest,
        condition: Digest,
        reset_generation: u64,
    },
    AircraftChanged,
    Sample(XPlaneTruthSample),
    Error {
        generation: u64,
        code: String,
    },
}

pub(crate) fn parse_line(line: &str) -> Result<Message, XPlaneTrialError> {
    if line.len() > MAXIMUM_LINE_BYTES {
        return invalid("line exceeds the protocol limit");
    }
    let fields = line.split_ascii_whitespace().collect::<Vec<_>>();
    let Some(kind) = fields.first().copied() else {
        return invalid("line is empty");
    };
    match kind {
        "HELLO" => parse_hello(&fields),
        "CONFIGURED" => parse_configured(&fields),
        "STARTED" => parse_started(&fields),
        "STOPPED" => parse_stopped(&fields),
        "RESETTING" => parse_resetting(&fields),
        "RESET_COMPLETE" => parse_reset_complete(&fields),
        "WIND_APPLIED" => parse_wind_applied(&fields),
        "REWIND" => parse_rewind(&fields),
        "ACTIVE" => parse_active(&fields),
        "AIRCRAFT_CHANGED" if fields.len() == 1 => Ok(Message::AircraftChanged),
        "SAMPLE" => XPlaneTruthSample::parse_fields(&fields).map(Message::Sample),
        "ERROR" => parse_error(&fields),
        _ => invalid(format!("unknown message type {kind}")),
    }
}

fn parse_hello(fields: &[&str]) -> Result<Message, XPlaneTrialError> {
    if fields.len() != 10 {
        return invalid("HELLO has an invalid field count");
    }
    let hello = Hello {
        protocol_version: number(fields, 1, "protocol version")?,
        xplane_version: number(fields, 2, "X-Plane version")?,
        sdk_version: number(fields, 3, "SDK version")?,
        host_application_id: number(fields, 4, "host application id")?,
        source_build_id: hex_text(fields[5], "source build id")?,
        bridge_build_digest: digest(fields[6], "loaded bridge build digest")?,
        aircraft_path: PathBuf::from(hex_text(fields[7], "aircraft path")?),
        trial_plugin_path: PathBuf::from(hex_text(fields[8], "trial plugin path")?),
        bridge_plugin_path: PathBuf::from(hex_text(fields[9], "bridge plugin path")?),
    };
    if hello.protocol_version != PROTOCOL_VERSION {
        return Err(XPlaneTrialError::ProtocolVersion {
            expected: PROTOCOL_VERSION,
            actual: hello.protocol_version,
        });
    }
    Ok(Message::Hello(hello))
}

fn parse_configured(fields: &[&str]) -> Result<Message, XPlaneTrialError> {
    exact(fields, 4, "CONFIGURED")?;
    Ok(Message::Configured {
        generation: number(fields, 1, "generation")?,
        scenario: digest(fields[2], "scenario digest")?,
        condition: digest(fields[3], "condition digest")?,
    })
}

fn parse_started(fields: &[&str]) -> Result<Message, XPlaneTrialError> {
    exact(fields, 4, "STARTED")?;
    Ok(Message::Started {
        generation: number(fields, 1, "generation")?,
        sim_time_s: finite_number(fields, 2, "simulator time")?,
        reset_generation: number(fields, 3, "reset generation")?,
    })
}

fn parse_stopped(fields: &[&str]) -> Result<Message, XPlaneTrialError> {
    exact(fields, 4, "STOPPED")?;
    Ok(Message::Stopped {
        generation: number(fields, 1, "generation")?,
        sample_count: number(fields, 2, "sample count")?,
        sim_time_s: finite_number(fields, 3, "simulator time")?,
    })
}

fn parse_resetting(fields: &[&str]) -> Result<Message, XPlaneTrialError> {
    exact(fields, 2, "RESETTING")?;
    Ok(Message::Resetting {
        generation: number(fields, 1, "generation")?,
    })
}

fn parse_reset_complete(fields: &[&str]) -> Result<Message, XPlaneTrialError> {
    exact(fields, 4, "RESET_COMPLETE")?;
    Ok(Message::ResetComplete {
        generation: number(fields, 1, "generation")?,
        reset_generation: number(fields, 2, "reset generation")?,
        sim_time_s: finite_number(fields, 3, "simulator time")?,
    })
}

fn parse_wind_applied(fields: &[&str]) -> Result<Message, XPlaneTrialError> {
    exact(fields, 5, "WIND_APPLIED")?;
    let actual_speed_mps = finite_number(fields, 3, "actual wind speed")?;
    let actual_direction_deg = finite_number(fields, 4, "actual wind direction")?;
    if actual_speed_mps < 0.0 || !(0.0..=360.0).contains(&actual_direction_deg) {
        return invalid("WIND_APPLIED values are outside their range");
    }
    Ok(Message::WindApplied {
        generation: number(fields, 1, "generation")?,
        condition_generation: number(fields, 2, "condition generation")?,
        actual_speed_mps,
        actual_direction_deg,
    })
}

fn parse_rewind(fields: &[&str]) -> Result<Message, XPlaneTrialError> {
    exact(fields, 2, "REWIND")?;
    Ok(Message::Rewind {
        reset_generation: number(fields, 1, "reset generation")?,
    })
}

fn parse_active(fields: &[&str]) -> Result<Message, XPlaneTrialError> {
    exact(fields, 5, "ACTIVE")?;
    Ok(Message::Active {
        generation: number(fields, 1, "generation")?,
        scenario: digest(fields[2], "scenario digest")?,
        condition: digest(fields[3], "condition digest")?,
        reset_generation: number(fields, 4, "reset generation")?,
    })
}

fn parse_error(fields: &[&str]) -> Result<Message, XPlaneTrialError> {
    exact(fields, 3, "ERROR")?;
    Ok(Message::Error {
        generation: number(fields, 1, "generation")?,
        code: fields[2].to_owned(),
    })
}

fn exact(fields: &[&str], expected: usize, kind: &str) -> Result<(), XPlaneTrialError> {
    if fields.len() == expected {
        Ok(())
    } else {
        invalid(format!("{kind} has an invalid field count"))
    }
}

fn number<T>(fields: &[&str], index: usize, field: &str) -> Result<T, XPlaneTrialError>
where
    T: std::str::FromStr,
{
    fields
        .get(index)
        .ok_or_else(|| invalid_value(format!("{field} is missing")))?
        .parse::<T>()
        .map_err(|_| invalid_value(format!("{field} is invalid")))
}

pub(crate) fn finite_number(
    fields: &[&str],
    index: usize,
    field: &str,
) -> Result<f64, XPlaneTrialError> {
    let value = number::<f64>(fields, index, field)?;
    if value.is_finite() {
        Ok(value)
    } else {
        invalid(format!("{field} is not finite"))
    }
}

fn digest(value: &str, field: &str) -> Result<Digest, XPlaneTrialError> {
    if value.len() != 64 {
        return invalid(format!("{field} has an invalid length"));
    }
    let mut bytes = [0_u8; 32];
    for (index, pair) in value.as_bytes().chunks_exact(2).enumerate() {
        bytes[index] = (nibble(pair[0], field)? << 4) | nibble(pair[1], field)?;
    }
    Ok(Digest::from_bytes(bytes))
}

fn nibble(value: u8, field: &str) -> Result<u8, XPlaneTrialError> {
    match value {
        b'0'..=b'9' => Ok(value - b'0'),
        b'a'..=b'f' => Ok(value - b'a' + 10),
        _ => invalid(format!("{field} is not lower-case hexadecimal")),
    }
}

fn hex_text(value: &str, field: &str) -> Result<String, XPlaneTrialError> {
    if value == "-" {
        return Ok(String::new());
    }
    if value.is_empty() || !value.len().is_multiple_of(2) || value.len() > 4096 {
        return invalid(format!("{field} has an invalid length"));
    }
    let mut bytes = Vec::with_capacity(value.len() / 2);
    for pair in value.as_bytes().chunks_exact(2) {
        bytes.push((nibble(pair[0], field)? << 4) | nibble(pair[1], field)?);
    }
    String::from_utf8(bytes).map_err(|_| invalid_value(format!("{field} is not UTF-8")))
}

fn invalid<T>(detail: impl Into<String>) -> Result<T, XPlaneTrialError> {
    Err(invalid_value(detail))
}

fn invalid_value(detail: impl Into<String>) -> XPlaneTrialError {
    XPlaneTrialError::InvalidProtocol {
        detail: detail.into(),
    }
}
