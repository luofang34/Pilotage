//! The coordinator's configuration.
//!
//! - `PILOTAGE_COORDINATOR_MEMBERS`: a JSON file listing the members:
//!   `[{"name": "SIM-1", "url": "https://127.0.0.1:4501/pilotage",
//!   "cert_sha256": "<64 hex>", "launch": [lat, lon, alt_m]}]`. The name is
//!   the member's lane name; the certificate pins its host. An optional
//!   `"token_file"` names the absolute path of the secret the coordinator
//!   presents to that host; the approver secret lets it pass the team's
//!   approval down.
//! - `PILOTAGE_LANE_NAME`: the team lane's name; default `TEAM`.

use std::path::PathBuf;

use pilotage_coordination::{Member, TeamConfig};
use pilotage_mission::SurveyDefaults;
use pilotage_protocol::wire;
use serde::Deserialize;

use crate::error::HostError;
use crate::runtime::options::env_var;

/// One member host.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct MemberHost {
    pub name: String,
    pub url: String,
    pub cert_sha256: [u8; 32],
    pub token: Option<pilotage_protocol::JoinToken>,
}

#[derive(Debug, Clone, PartialEq)]
pub(crate) struct CoordinatorConfig {
    pub team: TeamConfig,
    pub members: Vec<MemberHost>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct MemberEntry {
    name: String,
    url: String,
    cert_sha256: String,
    launch: [f64; 3],
    #[serde(default)]
    token_file: Option<PathBuf>,
}

impl CoordinatorConfig {
    /// Reads the configuration.
    ///
    /// # Errors
    /// [`HostError::CoordinatorMembers`] for a missing or malformed file.
    pub(crate) fn from_env_blocking() -> Result<Self, HostError> {
        let path = env_var("PILOTAGE_COORDINATOR_MEMBERS")?
            .map(PathBuf::from)
            .ok_or_else(|| {
                members_error(PathBuf::new(), "PILOTAGE_COORDINATOR_MEMBERS is not set")
            })?;
        let text = std::fs::read_to_string(&path)
            .map_err(|e| members_error(path.clone(), &e.to_string()))?;
        let entries: Vec<MemberEntry> =
            serde_json::from_str(&text).map_err(|e| members_error(path.clone(), &e.to_string()))?;
        if entries.is_empty() {
            return Err(members_error(path, "the file lists no member"));
        }
        let mut members = Vec::new();
        let mut hosts = Vec::new();
        for entry in entries {
            let cert_sha256 = hex32(&entry.cert_sha256).ok_or_else(|| {
                members_error(
                    path.clone(),
                    &format!("{}: cert_sha256 is not 64 hex digits", entry.name),
                )
            })?;
            let token = entry
                .token_file
                .as_deref()
                .map(crate::runtime::network::read_token_blocking)
                .transpose()
                .map_err(|e| members_error(path.clone(), &format!("{}: {e}", entry.name)))?;
            let [latitude_deg, longitude_deg, launch_altitude_m] = entry.launch;
            members.push(Member {
                name: entry.name.clone(),
                launch: wire::GeoPoint {
                    latitude_deg,
                    longitude_deg,
                },
                launch_altitude_m,
            });
            hosts.push(MemberHost {
                name: entry.name,
                url: entry.url,
                cert_sha256,
                token,
            });
        }
        Ok(Self {
            team: TeamConfig {
                name: env_var("PILOTAGE_LANE_NAME")?.unwrap_or_else(|| "TEAM".to_owned()),
                members,
                defaults: SurveyDefaults::default(),
                recovery_policy: "each member holds in place; the team flies no new line"
                    .to_owned(),
            },
            members: hosts,
        })
    }
}

fn members_error(path: PathBuf, reason: &str) -> HostError {
    HostError::CoordinatorMembers {
        path,
        reason: reason.to_owned(),
    }
}

fn hex32(text: &str) -> Option<[u8; 32]> {
    let text = text.trim();
    if text.len() != 64 {
        return None;
    }
    let mut out = [0_u8; 32];
    for (index, byte) in out.iter_mut().enumerate() {
        *byte = u8::from_str_radix(text.get(2 * index..2 * index + 2)?, 16).ok()?;
    }
    Some(out)
}
