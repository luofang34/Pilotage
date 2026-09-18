//! The pilot loop: one flight, scripted or live.
//!
//! Three things meet here and stay apart. The model turns operator text into
//! a directive. The executor turns the directive and the vehicle state into a
//! demand, one frame each 50 ms, and never waits for the model. The verifier
//! watches simulator truth and knows nothing about the other two.

use std::time::{Duration, Instant};

use intent_pilot::{ModelProcess, ModelProcessError};
use pilotage_agent::{
    AgentFlight, Directive, Discrete, ModelReply, ModelRequest, Phase, Report, Scenario,
    TruthState, VehicleOffer, Verifier,
};
use pilotage_client_session::ModuleEvent;
use pilotage_protocol::wire;
use tokio::sync::mpsc;
use tokio::task::JoinHandle;

use crate::cli::Options;
use crate::error::PilotError;
use crate::record::{Entry, RunRecord, Source};
use crate::script::Script;
use crate::session::{MOTION_SCOPE, PROFILE_ID, Session};
use crate::telemetry::{control_state, truth_state};

mod input;
mod messages;

/// The control-frame interval. The host drops a holder that is silent for one
/// second and refuses a frame older than 250 ms.
const FRAME_INTERVAL: Duration = Duration::from_millis(50);
/// Interval between truth samples in the run record, in seconds. It is short
/// enough to draw the flown path from the record.
const TRACE_INTERVAL_S: f64 = 0.2;
/// Time allowed to bring the vehicle home after a verdict, in seconds.
const RECOVERY_TIMEOUT: Duration = Duration::from_secs(120);
/// Rejected frames written to the record before the rest are only counted.
const REJECTIONS_RECORDED: u32 = 5;

/// How a run ended.
pub(crate) enum Outcome {
    /// A scenario run, with the verifier's report.
    Verdict(Report),
    /// A live run. It has no verdict.
    Live,
}

/// One operator message on its way to the model.
pub(crate) struct Ask {
    text: String,
    source: Source,
}

/// The model's answer to one operator message.
struct Answer {
    ask: Ask,
    result: Result<(ModelReply, f64), ModelProcessError>,
}

/// Flies the scenario in `options`.
pub(crate) async fn fly(options: &Options) -> Result<Outcome, PilotError> {
    let scenario = load_scenario(options).await?;
    let verifier = Verifier::new(&scenario).ok_or_else(|| PilotError::Usage {
        detail: "the scenario expectation names an unknown fix".to_owned(),
    })?;
    let model = ModelProcess::spawn(&options.model_cmd)
        .await
        .map_err(PilotError::Model)?;
    tracing::info!(adapter = %model.declaration().adapter, model = %model.declaration().model, "model adapter ready");
    let mut record = RunRecord::create(&options.record).await?;
    let legend = scenario.legend();
    record
        .append(&Entry::Start {
            scenario: &scenario,
            adapter: model.declaration(),
            legend: &legend,
            profile_id: PROFILE_ID,
            live: options.live,
        })
        .await?;

    let session = Session::open(&options.url, options.cert_sha256).await?;
    if !session.offers(Discrete::Arm) {
        return Err(PilotError::Unoffered {
            detail: format!("{MOTION_SCOPE} advertises no arm action"),
        });
    }
    let offer = VehicleOffer {
        max_linear_mps: session.max_linear_mps(),
        disarm_offered: session.offers(Discrete::Disarm),
    };
    tracing::info!(profile = PROFILE_ID, ?offer, "motion lease held");

    let (asks, ask_queue) = mpsc::channel(8);
    let (answer_queue, answers) = mpsc::channel(8);
    let model_task = tokio::spawn(serve_model(model, ask_queue, answer_queue));
    let mut pilot = Pilot {
        flight: AgentFlight::new(&scenario, offer),
        script: Script::new(scenario.messages.clone()),
        verifier,
        record,
        session,
        asks,
        answers,
        operator: options.live.then(input::operator_lines),
        live: options.live,
        quitting: false,
        planar_truth: options.planar_truth,
        truth: None,
        traced_at_s: None,
        clock: None,
        last_phase: Phase::Idle,
        rejections: 0,
    };
    let outcome = pilot.run().await;
    if outcome.is_ok() {
        pilot.recover().await;
    }
    pilot.finish(&outcome, model_task).await?;
    outcome
}

async fn load_scenario(options: &Options) -> Result<Scenario, PilotError> {
    let path = &options.scenario;
    let text =
        tokio::fs::read_to_string(path)
            .await
            .map_err(|source| PilotError::ScenarioRead {
                path: path.clone(),
                source,
            })?;
    Scenario::parse(&text).map_err(|source| PilotError::Scenario {
        path: path.clone(),
        source,
    })
}

/// Runs the exchange with the model in a task of its own. One reply takes
/// longer than the control-frame staleness bound, and the frame loop must not
/// wait for it.
async fn serve_model(
    mut model: ModelProcess,
    mut asks: mpsc::Receiver<(Ask, ModelRequest)>,
    answers: mpsc::Sender<Answer>,
) {
    while let Some((ask, request)) = asks.recv().await {
        let result = model.ask(&request).await;
        if answers.send(Answer { ask, result }).await.is_err() {
            break;
        }
    }
    model.stop().await;
}

struct Pilot {
    flight: AgentFlight,
    script: Script,
    verifier: Verifier,
    record: RunRecord,
    session: Session,
    asks: mpsc::Sender<(Ask, ModelRequest)>,
    answers: mpsc::Receiver<Answer>,
    operator: Option<mpsc::Receiver<input::Line>>,
    live: bool,
    quitting: bool,
    planar_truth: bool,
    truth: Option<TruthState>,
    traced_at_s: Option<f64>,
    /// Set when the first operator message is released. The verdict deadline
    /// and every record time count from it.
    clock: Option<Instant>,
    last_phase: Phase,
    rejections: u32,
}

impl Pilot {
    async fn run(&mut self) -> Result<Outcome, PilotError> {
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
                Some(answer) = self.answers.recv() => {
                    self.on_answer(answer).await?;
                    None
                }
                Some(line) = input::next(&mut self.operator) => {
                    self.on_operator(line).await?;
                    None
                }
            };
            if let Some(outcome) = decided {
                return Ok(outcome);
            }
        }
    }

    /// Brings the vehicle home and lands it when a run ends in the air. The
    /// verdict is already decided, and this does not change it. A pilot that
    /// leaves with its vehicle in the air hands it to the link-loss policy.
    async fn recover(&mut self) {
        if matches!(
            self.flight.phase(),
            Phase::Idle | Phase::Landed | Phase::Disarming
        ) {
            return;
        }
        tracing::info!("the run is decided; the vehicle returns and lands");
        self.live = true;
        self.quitting = true;
        let home = Directive::ReturnToBase {};
        if self.flight.fly(&home, self.elapsed_s()).is_err() {
            return;
        }
        match tokio::time::timeout(RECOVERY_TIMEOUT, self.run()).await {
            Ok(Ok(_)) => {}
            Ok(Err(error)) => tracing::warn!(%error, "the recovery stopped early"),
            Err(_) => tracing::warn!("the vehicle is not down after the recovery time"),
        }
    }

    fn elapsed_s(&self) -> f64 {
        self.clock
            .map_or(0.0, |clock| clock.elapsed().as_secs_f64())
    }

    async fn on_frame(&mut self) -> Result<Option<Outcome>, PilotError> {
        if !self.session.holds_control() {
            return Err(PilotError::ControlLost);
        }
        let flying_s = self.flight.flying_seconds(self.elapsed_s());
        // The return to base restarts the flying time, and a scripted message
        // must not ride on it after the run ended.
        if !self.quitting
            && let Some(message) = self.script.release(flying_s)
        {
            let text = message.text.clone();
            self.ask(text, Source::Script).await;
        }
        let now_s = self.elapsed_s();
        let demand = match self.flight.step(now_s) {
            Some(step) => {
                if let Some(request) = step.action {
                    tracing::info!(?request, "discrete request");
                    self.session.send_action(request).await?;
                }
                self.note_phase(step.phase, now_s).await?;
                step.demand
            }
            // The lease needs frames before the first telemetry sample.
            None => pilotage_agent::Demand::default(),
        };
        self.session.send_demand(demand).await?;
        self.trace_truth(now_s).await?;
        if self.live {
            let down = matches!(self.flight.phase(), Phase::Idle | Phase::Landed);
            return Ok((self.quitting && down).then_some(Outcome::Live));
        }
        let report = self.clock.and_then(|_| self.verifier.on_clock(now_s));
        Ok(report.map(Outcome::Verdict))
    }

    async fn note_phase(&mut self, phase: Phase, at_s: f64) -> Result<(), PilotError> {
        if phase == self.last_phase {
            return Ok(());
        }
        let Some(state) = self.flight.state().copied() else {
            return Ok(());
        };
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
                heading_deg: truth.yaw_rad.map(|yaw| yaw.to_degrees().rem_euclid(360.0)),
                armed: self.flight.state().and_then(|state| state.armed),
                phase: self.last_phase,
            })
            .await
    }

    async fn on_events(&mut self, events: Vec<ModuleEvent>) -> Result<Option<Outcome>, PilotError> {
        for event in events {
            match event {
                ModuleEvent::Telemetry(sample) => {
                    if let Some(report) = self.on_telemetry(&sample) {
                        return Ok(Some(Outcome::Verdict(report)));
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
        if let Some(state) = control_state(sample) {
            self.flight.observe(state);
        }
        // The verifier starts with the clock. Truth from before the first
        // message is the vehicle on its pad and proves nothing.
        self.clock?;
        let truth = truth_state(sample, self.planar_truth)?;
        self.truth = Some(truth);
        if self.live {
            return None;
        }
        let armed = self.flight.state().and_then(|state| state.armed);
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
        self.flight.on_action_result(request, result.accepted);
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
        // Only a request for the motion scope is a request for this lease. A
        // request for another scope, such as the gimbal, is not.
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

    async fn finish(
        mut self,
        outcome: &Result<Outcome, PilotError>,
        mut model_task: JoinHandle<()>,
    ) -> Result<(), PilotError> {
        let at_s = self.elapsed_s();
        match outcome {
            Ok(Outcome::Verdict(report)) => self.record.append(&Entry::Verdict { report }).await?,
            Ok(Outcome::Live) => self.record.append(&Entry::LiveEnded { at_s }).await?,
            Err(_) => {}
        }
        self.session.close().await;
        // A closed ask queue ends the model task's loop.
        drop(self.asks);
        if tokio::time::timeout(Duration::from_secs(2), &mut model_task)
            .await
            .is_err()
        {
            tracing::warn!(task = "model", "task did not stop; aborting it");
            model_task.abort();
        }
        Ok(())
    }
}
