//! The pilot loop: one scenario, one flight, one verdict.
//!
//! Three things meet here and stay apart. The classifier turns operator
//! text into a destination. The sequencer turns the destination and the
//! vehicle state into a demand, one frame each 50 ms, and never waits for
//! the classifier. The verifier watches simulator truth and knows nothing
//! about the other two.

use std::time::{Duration, Instant};

use pilotage_client_session::ModuleEvent;
use pilotage_protocol::wire;
use tokio::sync::mpsc;
use tokio::task::JoinHandle;

use crate::classifier::{Classifier, Reading, legend};
use crate::cli::Options;
use crate::error::PilotError;
use crate::guidance;
use crate::record::{Entry, RunRecord};
use crate::scenario::Scenario;
use crate::script::Script;
use crate::sequencer::{Destination, Discrete, FlightLimits, Phase, Sequencer};
use crate::session::{MOTION_SCOPE, PROFILE_ID, Session};
use crate::vehicle_state::{TruthState, VehicleState, control_state, truth_state};
use crate::verdict::{Report, Verifier};

/// The control-frame interval. The host drops a holder that is silent
/// for one second and refuses a frame older than 250 ms.
const FRAME_INTERVAL: Duration = Duration::from_millis(50);
/// Time the classifier task gets to stop before it is aborted.
const CLASSIFIER_GRACE: Duration = Duration::from_secs(2);
/// Interval between truth samples in the run record, in seconds. It is
/// short enough to draw the flown path from the record.
const TRACE_INTERVAL_S: f64 = 0.2;
/// Rejected frames written to the record before the rest are only counted.
const REJECTIONS_RECORDED: u32 = 5;

/// Flies the scenario in `options` and returns the verifier's report.
pub(crate) async fn fly(options: &Options) -> Result<Report, PilotError> {
    let scenario = Scenario::load(&options.scenario).await?;
    let verifier = Verifier::new(&scenario).ok_or_else(|| PilotError::ScenarioInvalid {
        detail: "the expectation names an unknown waypoint".to_owned(),
    })?;
    let classifier = Classifier::spawn(&options.classifier_cmd, &scenario).await?;
    tracing::info!(model = %classifier.model, "classifier ready");

    let mut record = RunRecord::create(&options.record).await?;
    record
        .append(&Entry::Start {
            scenario: &scenario,
            classifier_model: &classifier.model,
            legend: &legend(&scenario),
            profile_id: PROFILE_ID,
        })
        .await?;

    let (messages, message_queue) = mpsc::channel(8);
    let (reading_queue, readings) = mpsc::channel(8);
    let classifier_task = tokio::spawn(classifier.serve(message_queue, reading_queue));

    let session = Session::open(&options.url, options.cert_sha256).await?;
    tracing::info!(profile = PROFILE_ID, "motion lease held");
    if !session.offers(Discrete::Arm) {
        return Err(PilotError::Unoffered {
            detail: "vehicle.motion advertises no arm action".to_owned(),
        });
    }
    let limits = FlightLimits {
        cruise_height_m: scenario.cruise_height_m,
        arrival_radius_m: scenario.arrival_radius_m,
        max_linear_mps: session.max_linear_mps(),
        disarm_offered: session.offers(Discrete::Disarm),
    };
    tracing::info!(?limits, "flight limits");

    let mut pilot = Pilot {
        sequencer: Sequencer::new(limits),
        script: Script::new(scenario.messages.clone()),
        scenario,
        verifier,
        record,
        session,
        messages,
        readings,
        planar_truth: options.planar_truth,
        state: None,
        truth: None,
        traced_at_s: None,
        clock: None,
        last_phase: Phase::AwaitIntent,
        rejections: 0,
    };
    let outcome = pilot.run().await;
    if let Ok(report) = &outcome {
        pilot.record.append(&Entry::Verdict { report }).await?;
    }
    pilot.finish(classifier_task).await;
    outcome
}

struct Pilot {
    scenario: Scenario,
    sequencer: Sequencer,
    script: Script,
    verifier: Verifier,
    record: RunRecord,
    session: Session,
    messages: mpsc::Sender<String>,
    readings: mpsc::Receiver<Result<Reading, PilotError>>,
    planar_truth: bool,
    state: Option<VehicleState>,
    truth: Option<TruthState>,
    traced_at_s: Option<f64>,
    /// Set when the first operator message is released. The verdict
    /// deadline and every record time count from it.
    clock: Option<Instant>,
    last_phase: Phase,
    rejections: u32,
}

impl Pilot {
    async fn run(&mut self) -> Result<Report, PilotError> {
        let mut frames = tokio::time::interval(FRAME_INTERVAL);
        frames.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
        loop {
            let decided = tokio::select! {
                _ = frames.tick() => self.on_frame().await?,
                events = self.session.next_events() => {
                    let events = events.ok_or_else(|| PilotError::TransportLost {
                        detail: "the event queue closed".to_owned(),
                    })??;
                    self.on_events(events).await?
                }
                Some(reading) = self.readings.recv() => {
                    self.on_reading(reading).await?;
                    None
                }
            };
            if let Some(report) = decided {
                return Ok(report);
            }
        }
    }

    fn elapsed_s(&self) -> f64 {
        self.clock
            .map_or(0.0, |clock| clock.elapsed().as_secs_f64())
    }

    async fn on_frame(&mut self) -> Result<Option<Report>, PilotError> {
        if !self.session.holds_control() {
            return Err(PilotError::ControlLost);
        }
        let enroute_s = self.sequencer.enroute_seconds(self.elapsed_s());
        if let Some(message) = self.script.release(enroute_s) {
            self.clock.get_or_insert_with(Instant::now);
            tracing::info!(text = %message.text, "operator message");
            // A closed queue means the classifier task ended. The next
            // reading reports why; the frame loop keeps the vehicle safe.
            self.messages.send(message.text.clone()).await.ok();
        }
        let now_s = self.elapsed_s();
        let demand = match self.state {
            Some(state) => {
                let step = self.sequencer.step(&state, now_s);
                if let Some(request) = step.action {
                    tracing::info!(?request, "discrete request");
                    self.session.send_action(request).await?;
                }
                self.note_phase(step.phase, &state, now_s).await?;
                step.demand
            }
            // The lease needs frames before the first telemetry sample.
            None => guidance::STOP,
        };
        self.session.send_demand(demand).await?;
        self.trace_truth(now_s).await?;
        Ok(self.clock.and_then(|_| self.verifier.on_clock(now_s)))
    }

    async fn trace_truth(&mut self, at_s: f64) -> Result<(), PilotError> {
        let due = self
            .traced_at_s
            .is_none_or(|last| at_s - last >= TRACE_INTERVAL_S);
        let (Some(truth), Some(_), true) = (self.truth, self.clock, due) else {
            return Ok(());
        };
        self.traced_at_s = Some(at_s);
        self.record
            .append(&Entry::Truth {
                at_s,
                north_m: truth.north_m,
                east_m: truth.east_m,
                height_m: truth.height_m,
                armed: self.state.and_then(|state| state.armed),
                phase: self.last_phase,
            })
            .await
    }

    async fn note_phase(
        &mut self,
        phase: Phase,
        state: &VehicleState,
        at_s: f64,
    ) -> Result<(), PilotError> {
        if phase == self.last_phase {
            return Ok(());
        }
        self.last_phase = phase;
        tracing::info!(?phase, north = state.north_m, east = state.east_m, "phase");
        self.record
            .append(&Entry::Phase {
                at_s,
                phase,
                north_m: state.north_m,
                east_m: state.east_m,
                height_m: state.height_m,
            })
            .await
    }

    async fn on_events(&mut self, events: Vec<ModuleEvent>) -> Result<Option<Report>, PilotError> {
        for event in events {
            match event {
                ModuleEvent::Telemetry(sample) => {
                    if let Some(report) = self.on_telemetry(&sample) {
                        return Ok(Some(report));
                    }
                }
                ModuleEvent::ActionResult(result) => self.on_action_result(&result),
                ModuleEvent::ControlRejected(rejected) => self.on_rejected(&rejected).await?,
                ModuleEvent::Authority(authority) => self.on_authority(authority).await?,
                ModuleEvent::ConnectionDown { .. } => {
                    return Err(PilotError::TransportLost {
                        detail: "the client engine reported the connection down".to_owned(),
                    });
                }
                _ => {}
            }
        }
        Ok(None)
    }

    fn on_telemetry(&mut self, sample: &wire::TelemetrySample) -> Option<Report> {
        self.state = control_state(sample).or(self.state);
        // The verifier starts with the clock. Truth from before the first
        // message is the vehicle on its pad and proves nothing.
        self.clock?;
        let truth = truth_state(sample, self.planar_truth)?;
        self.truth = Some(truth);
        let armed = self.state.and_then(|state| state.armed);
        self.verifier.observe(&truth, armed, self.elapsed_s())
    }

    fn on_action_result(&mut self, result: &wire::ControlActionResult) {
        let request = if result.action == wire::ControlAction::Arm as i32 {
            Discrete::Arm
        } else if result.action == wire::ControlAction::Disarm as i32 {
            Discrete::Disarm
        } else {
            return;
        };
        if !result.accepted {
            tracing::warn!(?request, detail = %result.detail, "the host refused the request");
        }
        self.sequencer.on_action_result(request, result.accepted);
    }

    async fn on_rejected(&mut self, rejected: &wire::FrameRejected) -> Result<(), PilotError> {
        self.rejections = self.rejections.wrapping_add(1);
        if self.rejections > REJECTIONS_RECORDED {
            return Ok(());
        }
        tracing::warn!(
            reason = rejected.reason,
            "the host rejected a control frame"
        );
        self.record
            .append(&Entry::FrameRejected {
                at_s: self.elapsed_s(),
                reason: rejected.reason,
            })
            .await
    }

    async fn on_authority(&mut self, authority: wire::AuthorityEvent) -> Result<(), PilotError> {
        // Only a request for the motion scope is a request for this lease.
        // A request for another scope, such as the gimbal, is not.
        if let Some(wire::authority_event::Event::ScopeTransferRequested(asked)) = authority.event
            && asked
                .scope
                .as_ref()
                .is_some_and(|scope| scope.value == MOTION_SCOPE)
            && let Some(principal) = asked.from_principal
        {
            tracing::warn!(
                principal = principal.value,
                "another principal asks for control"
            );
            self.session.yield_to(principal.value).await?;
        }
        Ok(())
    }

    async fn on_reading(&mut self, reading: Result<Reading, PilotError>) -> Result<(), PilotError> {
        let at_s = self.elapsed_s();
        let means = self.script.answered().cloned();
        let reading = match reading {
            Ok(reading) => reading,
            Err(error) => {
                // The vehicle keeps its last destination. A classifier
                // fault must not become a flight command.
                tracing::error!(%error, "classifier fault; the destination does not change");
                let detail = error.to_string();
                return self
                    .record
                    .append(&Entry::ClassifierFault { at_s, detail })
                    .await;
            }
        };
        let Some(waypoint) = self.scenario.position(&reading.intent.target) else {
            return Ok(());
        };
        tracing::info!(
            target = %reading.intent.target,
            on_arrival = ?reading.intent.on_arrival,
            target_prob = reading.target_prob,
            model_ms = reading.model_ms,
            "intent"
        );
        if let Some(means) = &means {
            let parse_correct = *means == reading.intent;
            self.record
                .append(&Entry::Message {
                    at_s,
                    reading: &reading,
                    means,
                    parse_correct,
                })
                .await?;
        }
        self.sequencer.set_destination(Destination {
            name: reading.intent.target,
            waypoint,
            on_arrival: reading.intent.on_arrival,
        });
        Ok(())
    }

    async fn finish(self, mut classifier_task: JoinHandle<()>) {
        self.session.close().await;
        // Closing the message queue ends the classifier's serve loop.
        drop(self.messages);
        if tokio::time::timeout(CLASSIFIER_GRACE, &mut classifier_task)
            .await
            .is_err()
        {
            tracing::warn!(task = "classifier", "task did not stop; aborting it");
            classifier_task.abort();
        }
    }
}
