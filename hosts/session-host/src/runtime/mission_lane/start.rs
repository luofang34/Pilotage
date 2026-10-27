//! Starting a lane's run: only once the journal holds it, and only once a
//! person handed control back to a run that lost its lease. A recovery, which
//! only returns home, may start once the taker's lease ended without a
//! hand-back; it never starts while a person holds the lease.

use super::*;

impl LaneTask {
    /// Starts the run `order` names, replacing the run in progress.
    pub(super) async fn start(&mut self, order: Box<RunOrder>) {
        // A run starts only once the journal holds it, so that a
        // restart finds every run that may have had an effect.
        if !self.journal.keep(self.lane.record()).await {
            warn!(run = %order.run_id, "the run waits for the lane journal");
            self.unrecorded = Some(order);
            return;
        }
        let waits = self.run.as_ref().is_some_and(|run| {
            let status = run.status.borrow();
            status.awaiting_hand_back && !(order.recovery && status.control_freed)
        });
        if waits {
            warn!(run = %order.run_id, "the run waits for a hand-back to the run it replaces");
            // The replaced run takes the hand-back only to pass control on:
            // it holds, so its own plan never flies again.
            if let Some(run) = self.run.as_ref() {
                run.controls.send(RunControl::Hold).await.ok();
            }
            self.after_hand_back = Some(order);
            return;
        }
        if let Some(previous) = self.run.take() {
            // The previous run released the vehicle or is releasing
            // it; waiting for it keeps two principals from asking
            // for the same lease.
            previous.controls.send(RunControl::Abort).await.ok();
            previous.handle.await.ok();
        }
        self.runs = self.runs.wrapping_add(1);
        let client = ClientKey::new(RUN_CLIENT_BASE.wrapping_sub(self.runs));
        let (controls, receiver) = mpsc::channel(4);
        // A hold the lane took while the run waited for the journal
        // reaches the run before its first operation.
        if matches!(
            self.lane.state(),
            wire::MissionLaneState::HoldRequested | wire::MissionLaneState::Held
        ) {
            controls.try_send(RunControl::Hold).ok();
        }
        let order = *order;
        let (handle, status) = match order.plan {
            RunPlan::Operations(plan) => operations::spawn_operation_run(
                order.run_id.clone(),
                plan,
                self.operations.clone(),
                receiver,
                RunInputs {
                    results: order.results,
                    recorded: self.journal.ended(),
                },
            ),
            plan => {
                let Some(plan) = MissionPlan::of_lane(plan, order.config) else {
                    return;
                };
                automation::spawn_mission_run(
                    &self.engine,
                    self.start,
                    plan,
                    self.planar_pose_is_truth,
                    client,
                    Some(receiver),
                )
            }
        };
        info!(run = %order.run_id, "mission run started");
        self.run = Some(ActiveRun {
            handle,
            status,
            controls,
            stopping: false,
        });
    }

    /// The run that lost control ended while its replacement waited, so no
    /// hand-back can reach it. A recovery starts and, as any new run, waits
    /// for the lease until no person holds it; work does not start without
    /// a hand-back, so the lane's run ends instead and says why.
    pub(super) async fn on_replaced_run_ended(&mut self) {
        self.run = None;
        let Some(order) = self.after_hand_back.take() else {
            return;
        };
        if order.recovery {
            info!(run = %order.run_id, "the run that lost control ended; the recovery starts");
            self.start(order).await;
            return;
        }
        warn!(run = %order.run_id, "the run that lost control ended before a hand-back");
        self.lane.on_progress(&RunProgress {
            ended: true,
            ended_because: Some(
                "the run that lost control ended before control was handed back; propose the work again",
            ),
            ..RunProgress::default()
        });
    }
}
