#![allow(clippy::expect_used, clippy::panic)]
use super::*;
use crate::{NavigationPoint, RouteWaypoint};

fn plan() -> CoordinatedPlan {
    CoordinatedPlan {
        schema_version: 1,
        id: "mission".into(),
        revision: 0,
        title: "Survey".into(),
        participants: vec![
            MissionParticipant {
                id: "individual".into(),
                name: "Owner A".into(),
                organization: None,
            },
            MissionParticipant {
                id: "company".into(),
                name: "Owner B".into(),
                organization: Some("Survey organization".into()),
            },
        ],
        assignments: vec![
            assignment("a", "individual", Some(1_000)),
            assignment("b", "company", None),
        ],
        swarms: vec![Swarm {
            id: "group".into(),
            name: "Survey team".into(),
            assignment_ids: vec!["a".into(), "b".into()],
        }],
        timing_targets: vec![TimingTarget {
            id: "target".into(),
            name: "Survey start".into(),
            utc: 4_600,
            tolerance_seconds: 10,
            swarm_id: Some("group".into()),
            members: vec![
                TimingMember {
                    assignment_id: "a".into(),
                    waypoint_id: "end".into(),
                    offset_seconds: 0,
                },
                TimingMember {
                    assignment_id: "b".into(),
                    waypoint_id: "end".into(),
                    offset_seconds: 60,
                },
            ],
        }],
    }
}

fn assignment(id: &str, participant_id: &str, departure: Option<i64>) -> Assignment {
    Assignment {
        id: id.into(),
        name: id.into(),
        participant_id: participant_id.into(),
        vehicle_reference: None,
        route: RoutePlan {
            waypoints: vec![waypoint("start", 0.0), waypoint("end", 1.0)],
            groundspeed_knots: Some(60.0),
            departure_utc: departure,
            vehicle_profile: None,
        },
    }
}

fn waypoint(id: &str, longitude: f64) -> RouteWaypoint {
    RouteWaypoint {
        id: id.into(),
        point: NavigationPoint {
            key: id.into(),
            identifier: id.into(),
            name: String::new(),
            region: String::new(),
            kind: "waypoint".into(),
            latitude_deg: 0.0,
            longitude_deg: longitude,
        },
        source: None,
        altitude_msl_ft: None,
    }
}

#[test]
fn swarm_time_applies_to_each_member_and_unknown_is_not_on_time() {
    let result = assess_coordination(&plan()).expect("assess");
    assert_eq!(result.timings[0].status, "on_time");
    assert_eq!(result.timings[1].status, "unknown");
    assert_eq!(result.timings[1].required_utc, 4660.0);
    assert!(result.timings[1].earliest_departure_utc.expect("window") > 1000.0);
    assert!(result.timings[1].estimated_utc.is_none());
}

#[test]
fn incompatible_targets_report_a_departure_conflict() {
    let mut plan = plan();
    let mut extra = plan.timing_targets[0].clone();
    extra.id = "second".into();
    extra.utc += 100;
    plan.timing_targets.push(extra);
    let result = assess_coordination(&plan).expect("assess");
    assert_eq!(result.issues.len(), 2);
    assert!(result.timings.iter().any(|t| t.status == "early"));
}

#[test]
fn membership_and_waypoint_identity_cannot_silently_drift() {
    let mut plan = plan();
    plan.swarms[0].assignment_ids.pop();
    assert!(plan.validate().is_err());
    plan.swarms[0].assignment_ids.push("b".into());
    plan.assignments[0].route.waypoints.pop();
    assert!(plan.validate().is_err());
}

#[test]
fn export_digest_changes_when_timing_owner_or_route_changes() {
    let mut plan = plan();
    let first: serde_json::Value =
        serde_json::from_str(&export_revision(&plan).expect("export")).expect("json");
    plan.timing_targets[0].members[0].offset_seconds = 1;
    let second: serde_json::Value =
        serde_json::from_str(&export_revision(&plan).expect("export")).expect("json");
    assert_ne!(first["content_digest"], second["content_digest"]);
    assert_eq!(
        first["plan"]["participants"]
            .as_array()
            .expect("participants")
            .len(),
        2
    );
    assert!(first.get("authority").is_none());
}
