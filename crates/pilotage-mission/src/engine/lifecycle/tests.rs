use super::*;

const fn on(stage: Stage) -> Lifecycle {
    Lifecycle {
        stage,
        suspended: false,
    }
}

const fn held(stage: Stage) -> Lifecycle {
    Lifecycle {
        stage,
        suspended: true,
    }
}

const EVENTS: [LifecycleEvent; 9] = [
    LifecycleEvent::Waiting,
    LifecycleEvent::Arming,
    LifecycleEvent::Climbing,
    LifecycleEvent::OnRoute,
    LifecycleEvent::PlanDoneStopped,
    LifecycleEvent::PlanDoneFlying,
    LifecycleEvent::Failed,
    LifecycleEvent::Held,
    LifecycleEvent::Resumed,
];

/// Every lifecycle the machine can be in: each stage, and each stage a
/// human can hold the scope over.
fn lifecycles() -> Vec<Lifecycle> {
    use Stage::{AfterPlan, Armed, Climb, Complete, Failed, Idle, Route};
    let mut all: Vec<_> = [Idle, Armed, Climb, Route, AfterPlan, Complete, Failed]
        .map(on)
        .to_vec();
    all.extend([Idle, Armed, Climb, Route, AfterPlan].map(held));
    all
}

/// Every (lifecycle, event) pair with its stated outcome.
#[rustfmt::skip]
const TABLE: [(Lifecycle, LifecycleEvent, Lifecycle); 108] = {
    use LifecycleEvent as E;
    use Stage::{AfterPlan, Armed, Climb, Complete, Failed, Idle, Route};
    [
        (on(Idle), E::Waiting, on(Idle)),
        (on(Idle), E::Arming, on(Armed)),
        (on(Idle), E::Climbing, on(Climb)),
        (on(Idle), E::OnRoute, on(Route)),
        (on(Idle), E::PlanDoneStopped, on(Complete)),
        (on(Idle), E::PlanDoneFlying, on(AfterPlan)),
        (on(Idle), E::Failed, on(Failed)),
        (on(Idle), E::Held, held(Idle)),
        (on(Idle), E::Resumed, on(Idle)),

        (on(Armed), E::Waiting, on(Idle)),
        (on(Armed), E::Arming, on(Armed)),
        (on(Armed), E::Climbing, on(Climb)),
        (on(Armed), E::OnRoute, on(Route)),
        (on(Armed), E::PlanDoneStopped, on(Complete)),
        (on(Armed), E::PlanDoneFlying, on(AfterPlan)),
        (on(Armed), E::Failed, on(Failed)),
        (on(Armed), E::Held, held(Armed)),
        (on(Armed), E::Resumed, on(Armed)),

        (on(Climb), E::Waiting, on(Climb)),
        (on(Climb), E::Arming, on(Climb)),
        (on(Climb), E::Climbing, on(Climb)),
        (on(Climb), E::OnRoute, on(Route)),
        (on(Climb), E::PlanDoneStopped, on(Complete)),
        (on(Climb), E::PlanDoneFlying, on(AfterPlan)),
        (on(Climb), E::Failed, on(Failed)),
        (on(Climb), E::Held, held(Climb)),
        (on(Climb), E::Resumed, on(Climb)),

        (on(Route), E::Waiting, on(Route)),
        (on(Route), E::Arming, on(Route)),
        (on(Route), E::Climbing, on(Route)),
        (on(Route), E::OnRoute, on(Route)),
        (on(Route), E::PlanDoneStopped, on(Complete)),
        (on(Route), E::PlanDoneFlying, on(AfterPlan)),
        (on(Route), E::Failed, on(Failed)),
        (on(Route), E::Held, held(Route)),
        (on(Route), E::Resumed, on(Route)),

        (on(AfterPlan), E::Waiting, on(AfterPlan)),
        (on(AfterPlan), E::Arming, on(AfterPlan)),
        (on(AfterPlan), E::Climbing, on(AfterPlan)),
        (on(AfterPlan), E::OnRoute, on(AfterPlan)),
        (on(AfterPlan), E::PlanDoneStopped, on(AfterPlan)),
        (on(AfterPlan), E::PlanDoneFlying, on(AfterPlan)),
        (on(AfterPlan), E::Failed, on(Failed)),
        (on(AfterPlan), E::Held, held(AfterPlan)),
        (on(AfterPlan), E::Resumed, on(AfterPlan)),

        (on(Complete), E::Waiting, on(Complete)),
        (on(Complete), E::Arming, on(Complete)),
        (on(Complete), E::Climbing, on(Complete)),
        (on(Complete), E::OnRoute, on(Complete)),
        (on(Complete), E::PlanDoneStopped, on(Complete)),
        (on(Complete), E::PlanDoneFlying, on(Complete)),
        (on(Complete), E::Failed, on(Failed)),
        (on(Complete), E::Held, on(Complete)),
        (on(Complete), E::Resumed, on(Complete)),

        (on(Failed), E::Waiting, on(Failed)),
        (on(Failed), E::Arming, on(Failed)),
        (on(Failed), E::Climbing, on(Failed)),
        (on(Failed), E::OnRoute, on(Failed)),
        (on(Failed), E::PlanDoneStopped, on(Failed)),
        (on(Failed), E::PlanDoneFlying, on(Failed)),
        (on(Failed), E::Failed, on(Failed)),
        (on(Failed), E::Held, on(Failed)),
        (on(Failed), E::Resumed, on(Failed)),

        (held(Idle), E::Waiting, held(Idle)),
        (held(Idle), E::Arming, held(Idle)),
        (held(Idle), E::Climbing, held(Idle)),
        (held(Idle), E::OnRoute, held(Idle)),
        (held(Idle), E::PlanDoneStopped, held(Idle)),
        (held(Idle), E::PlanDoneFlying, held(Idle)),
        (held(Idle), E::Failed, on(Failed)),
        (held(Idle), E::Held, held(Idle)),
        (held(Idle), E::Resumed, on(Idle)),

        (held(Armed), E::Waiting, held(Armed)),
        (held(Armed), E::Arming, held(Armed)),
        (held(Armed), E::Climbing, held(Armed)),
        (held(Armed), E::OnRoute, held(Armed)),
        (held(Armed), E::PlanDoneStopped, held(Armed)),
        (held(Armed), E::PlanDoneFlying, held(Armed)),
        (held(Armed), E::Failed, on(Failed)),
        (held(Armed), E::Held, held(Armed)),
        (held(Armed), E::Resumed, on(Armed)),

        (held(Climb), E::Waiting, held(Climb)),
        (held(Climb), E::Arming, held(Climb)),
        (held(Climb), E::Climbing, held(Climb)),
        (held(Climb), E::OnRoute, held(Climb)),
        (held(Climb), E::PlanDoneStopped, held(Climb)),
        (held(Climb), E::PlanDoneFlying, held(Climb)),
        (held(Climb), E::Failed, on(Failed)),
        (held(Climb), E::Held, held(Climb)),
        (held(Climb), E::Resumed, on(Climb)),

        (held(Route), E::Waiting, held(Route)),
        (held(Route), E::Arming, held(Route)),
        (held(Route), E::Climbing, held(Route)),
        (held(Route), E::OnRoute, held(Route)),
        (held(Route), E::PlanDoneStopped, held(Route)),
        (held(Route), E::PlanDoneFlying, held(Route)),
        (held(Route), E::Failed, on(Failed)),
        (held(Route), E::Held, held(Route)),
        (held(Route), E::Resumed, on(Route)),

        (held(AfterPlan), E::Waiting, held(AfterPlan)),
        (held(AfterPlan), E::Arming, held(AfterPlan)),
        (held(AfterPlan), E::Climbing, held(AfterPlan)),
        (held(AfterPlan), E::OnRoute, held(AfterPlan)),
        (held(AfterPlan), E::PlanDoneStopped, held(AfterPlan)),
        (held(AfterPlan), E::PlanDoneFlying, held(AfterPlan)),
        (held(AfterPlan), E::Failed, on(Failed)),
        (held(AfterPlan), E::Held, held(AfterPlan)),
        (held(AfterPlan), E::Resumed, on(AfterPlan)),
    ]
};

#[test]
fn every_lifecycle_and_event_has_its_stated_outcome() {
    for lifecycle in lifecycles() {
        for event in EVENTS {
            let rows: Vec<_> = TABLE
                .iter()
                .filter(|(from, on, _)| *from == lifecycle && *on == event)
                .collect();
            assert_eq!(rows.len(), 1, "one row for {lifecycle:?} on {event:?}");
            let (_, _, to) = *rows[0];
            assert_eq!(next(lifecycle, event), to, "{lifecycle:?} on {event:?}");
        }
    }
}

#[test]
fn the_machine_never_leaves_the_lifecycles_the_table_covers() {
    let all = lifecycles();
    for lifecycle in &all {
        for event in EVENTS {
            let to = next(*lifecycle, event);
            assert!(all.contains(&to), "{lifecycle:?} on {event:?} gives {to:?}");
        }
    }
}

#[test]
fn each_lifecycle_reports_its_mission_state() {
    use Stage::{AfterPlan, Armed, Climb, Complete, Failed, Idle, Route};
    let cases = [
        (on(Idle), MissionState::AwaitSolution),
        (on(Armed), MissionState::Arming),
        (on(Climb), MissionState::Climb),
        (on(Route), MissionState::Enroute),
        (on(AfterPlan), MissionState::AfterPlan),
        (on(Complete), MissionState::Complete),
        (on(Failed), MissionState::Failed),
        (held(Idle), MissionState::Held),
        (held(Route), MissionState::Held),
        (held(AfterPlan), MissionState::Held),
    ];
    for (lifecycle, state) in cases {
        assert_eq!(lifecycle.reported(), state, "{lifecycle:?}");
    }
}
