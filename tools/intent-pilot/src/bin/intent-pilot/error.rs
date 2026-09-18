//! Typed errors of the intent pilot.

use std::path::PathBuf;

/// Every way the intent pilot can stop before it has a verdict.
#[derive(Debug, thiserror::Error)]
pub(crate) enum PilotError {
    /// The async runtime cannot start.
    #[error("cannot start the async runtime")]
    Runtime(#[source] std::io::Error),
    /// The command line is not usable.
    #[error("invalid command line: {detail}")]
    Usage {
        /// What is wrong with it.
        detail: String,
    },
    /// The scenario file cannot be read.
    #[error("cannot read scenario {path}")]
    ScenarioRead {
        /// The scenario path.
        path: PathBuf,
        /// The file-system failure.
        #[source]
        source: std::io::Error,
    },
    /// The scenario is not a usable document.
    #[error("cannot use scenario {path}")]
    Scenario {
        /// The scenario path.
        path: PathBuf,
        /// What is wrong with it.
        #[source]
        source: pilotage_agent::AgentError,
    },
    /// The model adapter cannot start.
    #[error("the model adapter is not usable")]
    Model(#[source] intent_pilot::ModelProcessError),
    /// The WebTransport endpoint cannot be built.
    #[error("cannot build the client endpoint")]
    Endpoint(#[source] std::io::Error),
    /// The host refused or dropped the connection attempt.
    #[error("cannot connect to {url}")]
    Connect {
        /// The host URL.
        url: String,
        /// The connection failure.
        #[source]
        source: wtransport::error::ConnectingError,
    },
    /// A stream to the host cannot be opened.
    #[error("cannot open the {stream} stream")]
    Stream {
        /// Which stream.
        stream: &'static str,
        /// The transport failure. It is boxed because the transport has
        /// one error type for each step of opening a stream.
        #[source]
        source: Box<dyn std::error::Error + Send + Sync>,
    },
    /// The host did not complete a protocol step in time.
    #[error("the host did not complete `{step}` in {seconds} s")]
    HostTimeout {
        /// The protocol step.
        step: &'static str,
        /// The deadline that passed.
        seconds: u64,
    },
    /// The host offers no way to fly this scenario.
    #[error("the host does not offer what the pilot needs: {detail}")]
    Unoffered {
        /// What is missing from the advertisement.
        detail: String,
    },
    /// A write to the host failed.
    #[error("the link to the host went down during {stage}")]
    LinkDown {
        /// What the pilot was sending.
        stage: &'static str,
        /// The transport failure, boxed for the same reason as `Stream`.
        #[source]
        source: Box<dyn std::error::Error + Send + Sync>,
    },
    /// Another principal holds the motion scope now. The pilot stops and
    /// does not ask for the lease again.
    #[error("another principal took control; the pilot stopped")]
    ControlLost,
    /// A reader task reported that the transport is gone.
    #[error("the host connection was lost: {detail}")]
    TransportLost {
        /// The reader's report. The client engine carries the cause as
        /// text, so no typed source exists at this point.
        detail: String,
    },
    /// The run record cannot be written.
    #[error("cannot write run record {path}")]
    Record {
        /// The record path.
        path: PathBuf,
        /// The file-system failure.
        #[source]
        source: std::io::Error,
    },
}
