//! The agent client module: the shared core with no I/O.
//!
//! An agent flies directives. A directive is one instruction in the words of
//! air traffic control, such as `direct_to`, `heading` or `join_procedure`. A
//! model outside Pilotage reads an operator message and replies with a
//! directive through the model port. This crate checks that reply against what
//! the vehicle advertises, flies it with deterministic code, and judges the
//! flight from a truth source that the model cannot touch.
//!
//! The crate has no transport, no clock and no wire types. A platform port
//! gives it vehicle state and time, and sends the demands that it returns.

mod capability;
mod directive;
mod error;
mod eval;
mod executor;
mod flight;
mod grounding;
mod guidance;
mod model_port;
mod scenario;
mod state;
mod verdict;

pub use capability::{FlightEnvelope, NumberRange};
pub use directive::{Arrival, Directive, DirectiveKind, HoldPoint, TurnDirection};
pub use error::AgentError;
pub use eval::{
    Case, CaseResult, Outcome, SlotScore, Suite, SuiteReport, Tally, score_case, summarize,
};
pub use executor::{Discrete, Executor, FlightLimits, Phase, Step};
pub use flight::{AgentFlight, VehicleOffer};
pub use grounding::check_grounding;
pub use guidance::Demand;
pub use model_port::{
    AdapterDeclaration, Frame, FrameSupport, ModelReply, ModelRequest, Projection, Refusal,
    SlotProbabilities, check_reply,
};
pub use scenario::{
    Checkpoint, EndState, Expectation, Fix, HOME, OperatorMessage, Procedure, Scenario, Trigger,
};
pub use state::{TruthState, VehicleState, yaw_of_quaternion};
pub use verdict::{Report, Verdict, Verifier};
