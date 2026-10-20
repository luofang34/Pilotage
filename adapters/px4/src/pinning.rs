//! The PX4 parameters an uplink depends on, and the one place a PX4
//! parameter write can come from.
//!
//! Every write a PX4 uplink sends persists on the vehicle, and on a real
//! aircraft some are dangerous: `COM_RC_IN_MODE = 1` disables the RC
//! transmitter. So a table of required values is written only under
//! [`Px4Profile::Simulation`], whose plan carries the [`SimulationWrites`]
//! proof no other profile can construct. Under [`Px4Profile::Physical`]
//! the same table is only read (PARAM_REQUEST_READ), and a value that
//! differs is a [`ParamRefusal`] naming the parameter and the value
//! Pilotage's control model needs, which arming reports.
//!
//! Either way PX4 answers with a PARAM_VALUE holding the value it stores;
//! a parameter counts as in place only when that answer matches.

use std::fmt;
use std::time::{Duration, Instant};

use pilotage_mavlink::codec::{
    PARAM_ID_LEN, ParamValue, encode_param_request_read, encode_param_set, param_id,
};

use crate::config::Px4Profile;

/// How long an unanswered request waits before it is sent again.
const RETRY_PERIOD: Duration = Duration::from_secs(1);

/// The proof that parameter writes are permitted: constructible only from
/// [`Px4Profile::Simulation`], inside this module.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct SimulationWrites(());

/// What the plan does about a parameter that is not in place.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Mode {
    /// Write the required value (simulation only).
    Write(SimulationWrites),
    /// Read the stored value and report a difference.
    Verify,
}

impl Mode {
    fn for_profile(profile: Px4Profile) -> Self {
        match profile {
            Px4Profile::Simulation => Self::Write(SimulationWrites(())),
            Px4Profile::Physical => Self::Verify,
        }
    }
}

/// Why arming waits on the parameters.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) enum ParamRefusal {
    /// The FC has not answered for the parameter yet.
    Unconfirmed {
        /// The parameter.
        name: &'static str,
        /// The value the control model needs.
        expected: ParamValue,
    },
    /// The FC stores a different value, and this profile does not write.
    Mismatch {
        /// The parameter.
        name: &'static str,
        /// The value the control model needs.
        expected: ParamValue,
        /// The value the FC reported.
        found: ParamValue,
    },
}

impl fmt::Display for ParamRefusal {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Unconfirmed { name, expected } => write!(
                f,
                "waiting for PX4 to confirm {name} = {}",
                Shown(*expected)
            ),
            Self::Mismatch {
                name,
                expected,
                found,
            } => write!(
                f,
                "PX4 parameter {name} is {}; Pilotage needs {} (set it on the vehicle: the adapter never writes a physical vehicle's parameters)",
                Shown(*found),
                Shown(*expected)
            ),
        }
    }
}

struct Shown(ParamValue);

impl fmt::Display for Shown {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self.0 {
            ParamValue::Int32(value) => write!(f, "{value}"),
            ParamValue::Real32(value) => write!(f, "{value}"),
        }
    }
}

/// One due parameter request: a write under simulation, a read otherwise.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) struct ParamRequest {
    name: &'static str,
    write: Option<(SimulationWrites, ParamValue)>,
}

impl ParamRequest {
    /// The MAVLink frame for this request.
    pub(crate) fn encode(self, seq: u8, target_system: u8, target_component: u8) -> Vec<u8> {
        match self.write {
            Some((SimulationWrites(()), value)) => {
                encode_param_set(seq, self.name, value, target_system, target_component).to_vec()
            }
            None => {
                encode_param_request_read(seq, self.name, target_system, target_component).to_vec()
            }
        }
    }
}

/// Which of a table of required parameters the FC has confirmed.
#[derive(Debug)]
pub(crate) struct ParamPinning {
    mode: Mode,
    pinned: &'static [(&'static str, ParamValue)],
    reported: Vec<Option<ParamValue>>,
    last_sent: Option<Instant>,
}

impl ParamPinning {
    pub(crate) fn new(profile: Px4Profile, pinned: &'static [(&'static str, ParamValue)]) -> Self {
        Self {
            mode: Mode::for_profile(profile),
            pinned,
            reported: vec![None; pinned.len()],
            last_sent: None,
        }
    }

    /// Whether every parameter is confirmed.
    pub(crate) fn all_confirmed(&self) -> bool {
        self.refusal().is_none()
    }

    /// The first parameter not in place, as arming reports it.
    pub(crate) fn refusal(&self) -> Option<ParamRefusal> {
        self.pinned
            .iter()
            .zip(&self.reported)
            .find_map(|(&(name, expected), reported)| match reported {
                Some(found) if same_value(expected, *found) => None,
                Some(found) if self.mode == Mode::Verify => Some(ParamRefusal::Mismatch {
                    name,
                    expected,
                    found: *found,
                }),
                _ => Some(ParamRefusal::Unconfirmed { name, expected }),
            })
    }

    /// The requests due now: one per parameter not in place, at most once
    /// per [`RETRY_PERIOD`].
    pub(crate) fn due(&mut self, now: Instant) -> Vec<ParamRequest> {
        if self.all_confirmed()
            || self
                .last_sent
                .is_some_and(|at| now.duration_since(at) < RETRY_PERIOD)
        {
            return Vec::new();
        }
        self.last_sent = Some(now);
        let write = match self.mode {
            Mode::Write(proof) => Some(proof),
            Mode::Verify => None,
        };
        self.pinned
            .iter()
            .zip(&self.reported)
            .filter(|((_, expected), reported)| {
                !reported.is_some_and(|found| same_value(*expected, found))
            })
            .map(|(&(name, expected), _)| ParamRequest {
                name,
                write: write.map(|proof| (proof, expected)),
            })
            .collect()
    }

    /// Folds one PARAM_VALUE in. A different value (another writer, a
    /// refused write, or a physical vehicle's own setting) withdraws the
    /// confirmation, so the request is sent again.
    pub(crate) fn observe(&mut self, id: [u8; PARAM_ID_LEN], value: ParamValue) {
        for (&(name, pinned), reported) in self.pinned.iter().zip(self.reported.iter_mut()) {
            if param_id(name) != id {
                continue;
            }
            let was = reported.is_some_and(|found| same_value(pinned, found));
            let matches = same_value(pinned, value);
            if matches != was || reported.is_none() {
                tracing::info!(param = name, ?value, pinned = matches, "PX4 parameter echo");
            }
            *reported = Some(value);
        }
    }
}

/// Exact comparison: the required values are sent verbatim, so the echo
/// returns the same bits.
fn same_value(pinned: ParamValue, echoed: ParamValue) -> bool {
    match (pinned, echoed) {
        (ParamValue::Int32(a), ParamValue::Int32(b)) => a == b,
        (ParamValue::Real32(a), ParamValue::Real32(b)) => a.to_bits() == b.to_bits(),
        _ => false,
    }
}

#[cfg(test)]
mod tests;
