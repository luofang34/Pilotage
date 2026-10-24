//! Plans the fleet's survey in memory, with the same lanes the hosts run,
//! to find where a camera fault must sit. Planning is deterministic: the
//! line a rehearsal gives an aircraft is the line its real run flies.

use std::collections::BTreeMap;

use navigate_contract::{ClockDomainId, GeodeticPosition};
use pilotage_coordination::{Member, MemberIntent, TeamConfig, TeamLane};
use pilotage_mission::{
    ExecutionTarget, IntentContext, Lane, LaneConfig, MissionConfig, OwnshipSource, SurveyDefaults,
};
use pilotage_protocol::wire;

use super::plan::{Airframe, Fault, Launch, MissionFile};
use crate::error::XtaskError;

/// The civil time every rehearsal intent carries; planning does not read it.
const REHEARSAL_MS: u64 = 1_800_000_000_000;
/// The part of the line the fault covers, from its start to its end.
const FAULT_SPAN: (f64, f64) = (0.12, 0.88);
/// Boxes along the covered part.
const FAULT_BOXES: u32 = 15;
/// Each box's half size, degrees of latitude and longitude: about 13 m
/// each way, wider than the camera's ground track at survey height.
const FAULT_HALF_DEG: (f64, f64) = (0.000_12, 0.000_16);

/// One lane of the in-memory fleet.
enum Node {
    Team(TeamLane, BTreeMap<String, Node>),
    Vehicle(String, Lane),
}

impl Node {
    fn take_changed(&mut self) -> Option<wire::MissionStatus> {
        match self {
            Self::Team(team, _) => team.take_changed(),
            Self::Vehicle(_, lane) => lane.take_changed(),
        }
    }
}

/// The aircraft lanes' own proposals, by lane name.
type Proposals = BTreeMap<String, wire::MissionProposal>;

/// The `PILOTAGE_SIM_CAMERA_FAULT` value that puts `fault` on its line:
/// a chain of boxes along the middle of the line the aircraft plans.
///
/// # Errors
///
/// Returns [`XtaskError::MissionFault`] when the fleet proposes nothing or
/// the aircraft's plan has no such line.
pub(crate) fn fault_spec(file: &MissionFile, fault: &Fault) -> Result<String, XtaskError> {
    let lane = format!("SIM-{}", fault.aircraft);
    let proposals = rehearse(file);
    let Some(proposal) = proposals.get(&lane) else {
        return Err(XtaskError::MissionFault {
            reason: format!("{lane} proposed no plan in the rehearsal"),
        });
    };
    // The fleet file names the line as a person reads it.
    let Some(line) = proposal.tasks.iter().find(|t| t.name == fault.line) else {
        let lines: Vec<&str> = proposal.tasks.iter().map(|t| t.name.as_str()).collect();
        return Err(XtaskError::MissionFault {
            reason: format!(
                "{lane} plans no {} (it plans {})",
                fault.line,
                lines.join(", ")
            ),
        });
    };
    let (Some(start), Some(end)) = (line.path.first(), line.path.last()) else {
        return Err(XtaskError::MissionFault {
            reason: format!("{lane} {} has no path", fault.line),
        });
    };
    let (half_lat, half_lon) = FAULT_HALF_DEG;
    let boxes: Vec<String> = (0..FAULT_BOXES)
        .map(|i| {
            let at = FAULT_SPAN.0
                + (FAULT_SPAN.1 - FAULT_SPAN.0) * f64::from(i) / f64::from(FAULT_BOXES - 1);
            let lat = start.latitude_deg + (end.latitude_deg - start.latitude_deg) * at;
            let lon = start.longitude_deg + (end.longitude_deg - start.longitude_deg) * at;
            format!(
                "{}@{:.6},{:.6},{:.6},{:.6}",
                fault.kind,
                lat - half_lat,
                lon - half_lon,
                lat + half_lat,
                lon + half_lon
            )
        })
        .collect();
    Ok(boxes.join(";"))
}

/// Proposes the fleet's objective to its lead and returns every
/// aircraft's own proposal.
pub(crate) fn rehearse(file: &MissionFile) -> Proposals {
    let sections = file
        .sections
        .iter()
        .map(|section| {
            let aircraft = section
                .aircraft
                .iter()
                .map(|a| {
                    let lane = a.lane();
                    let node = Node::Vehicle(lane.clone(), vehicle(&lane, file.airframe));
                    (lane, node)
                })
                .collect();
            let members = section
                .aircraft
                .iter()
                .map(|a| member(&a.lane(), a.launch))
                .collect();
            let team = team(&section.name, members);
            (section.name.clone(), Node::Team(team, aircraft))
        })
        .collect();
    let members = file
        .sections
        .iter()
        .map(|s| member(&s.name, s.launch))
        .collect();
    let mut root = Node::Team(team(&file.command.name, members), sections);
    let propose = wire::MissionIntent {
        request_id: "rehearsal".to_owned(),
        action: Some(wire::mission_intent::Action::Propose(
            file.objective.objective("xtask", REHEARSAL_MS),
        )),
    };
    let mut proposals = Proposals::new();
    let mut ids = 0_u64;
    deliver(&mut root, &propose, &mut ids, &mut proposals);
    proposals
}

/// Delivers `intent` to `node` and routes every member exchange it
/// causes until it settles. Returns the receipts `node` decided.
fn deliver(
    node: &mut Node,
    intent: &wire::MissionIntent,
    ids: &mut u64,
    proposals: &mut Proposals,
) -> Vec<wire::MissionReceipt> {
    let context = next_context(ids);
    let (team, children) = match node {
        Node::Vehicle(name, lane) => {
            let reply = lane.on_intent(intent, &context);
            if let Some(proposal) = &reply.receipt.proposal {
                proposals.insert(name.clone(), proposal.clone());
            }
            return vec![reply.receipt];
        }
        Node::Team(team, children) => (team, children),
    };
    let reply = team.on_intent(intent, &context);
    let mut decided: Vec<wire::MissionReceipt> = reply.receipt.into_iter().collect();
    let mut pending = reply.intents;
    while let Some(MemberIntent { member, intent }) = pending.pop() {
        let Some(child) = children.get_mut(&member) else {
            continue;
        };
        let context = next_context(ids);
        for receipt in deliver(child, &intent, ids, proposals) {
            let answer = team.on_member_receipt(&member, &receipt, &context);
            decided.extend(answer.receipt);
            pending.extend(answer.intents);
        }
        if let Some(status) = child.take_changed() {
            pending.extend(team.on_member_status(&status, &context));
        }
    }
    decided
}

fn next_context(ids: &mut u64) -> IntentContext {
    *ids = ids.wrapping_add(1);
    IntentContext {
        now_unix_ms: REHEARSAL_MS,
        fresh_id: format!("rehearsal-{ids}"),
    }
}

fn member(name: &str, [latitude_deg, longitude_deg, launch_altitude_m]: Launch) -> Member {
    Member {
        name: name.to_owned(),
        launch: wire::GeoPoint {
            latitude_deg,
            longitude_deg,
        },
        launch_altitude_m,
    }
}

/// A section or command lead, configured as the coordinator host does.
fn team(name: &str, members: Vec<Member>) -> TeamLane {
    TeamLane::new(TeamConfig {
        name: name.to_owned(),
        members,
        defaults: SurveyDefaults::default(),
        recovery_policy: "hold".to_owned(),
    })
}

/// An aircraft lane, configured as the simulated multicopter host does.
fn vehicle(name: &str, airframe: Airframe) -> Lane {
    let mut template = MissionConfig::new(
        String::new(),
        GeodeticPosition::new(0.0, 0.0, 0.0),
        ClockDomainId::new(1),
    );
    template.ownship_source = OwnshipSource::vehicle_estimate();
    template.execution_target = ExecutionTarget::Simulator;
    Lane::new(LaneConfig {
        name: name.to_owned(),
        simulated: true,
        execution_strategy: "host-guided-velocity".to_owned(),
        template,
        defaults: SurveyDefaults::default(),
        usable_energy_wh: Some(airframe.energy_wh),
        cruise_power_w: Some(airframe.cruise_power_w),
        recovery_policy: "hold".to_owned(),
        judged: false,
    })
}
