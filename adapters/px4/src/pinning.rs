//! PX4 parameters an uplink pins at connect, and the confirmation that each
//! write took effect.
//!
//! PX4 echoes every PARAM_SET with a PARAM_VALUE holding the value it now
//! stores; a parameter counts as pinned only when that echo matches.

use std::time::{Duration, Instant};

use pilotage_mavlink::codec::{PARAM_ID_LEN, ParamValue, param_id};

/// How long an unconfirmed write waits before it is sent again.
const RETRY_PERIOD: Duration = Duration::from_secs(1);

/// Which of a table of pinned parameters the FC has confirmed.
#[derive(Debug)]
pub(crate) struct ParamPinning {
    pinned: &'static [(&'static str, ParamValue)],
    confirmed: Vec<bool>,
    last_sent: Option<Instant>,
}

impl ParamPinning {
    pub(crate) fn new(pinned: &'static [(&'static str, ParamValue)]) -> Self {
        Self {
            pinned,
            confirmed: vec![false; pinned.len()],
            last_sent: None,
        }
    }

    /// Whether every pinned parameter is confirmed.
    pub(crate) fn all_confirmed(&self) -> bool {
        self.confirmed.iter().all(|confirmed| *confirmed)
    }

    /// The writes due now: every unconfirmed parameter, at most once per
    /// [`RETRY_PERIOD`].
    pub(crate) fn due(&mut self, now: Instant) -> Vec<(&'static str, ParamValue)> {
        if self.all_confirmed()
            || self
                .last_sent
                .is_some_and(|at| now.duration_since(at) < RETRY_PERIOD)
        {
            return Vec::new();
        }
        self.last_sent = Some(now);
        self.pinned
            .iter()
            .zip(self.confirmed.iter())
            .filter(|(_, confirmed)| !**confirmed)
            .map(|(pinned, _)| *pinned)
            .collect()
    }

    /// Folds one PARAM_VALUE in. A matching value confirms the parameter;
    /// a different value (another writer, or a refused write) withdraws
    /// the confirmation so the write is sent again.
    pub(crate) fn observe(&mut self, id: [u8; PARAM_ID_LEN], value: ParamValue) {
        for ((name, pinned), confirmed) in self.pinned.iter().zip(self.confirmed.iter_mut()) {
            if param_id(name) != id {
                continue;
            }
            let matches = same_value(*pinned, value);
            if matches != *confirmed {
                tracing::info!(param = name, ?value, pinned = matches, "PX4 parameter echo");
            }
            *confirmed = matches;
        }
    }
}

/// Exact comparison: the pinned values are sent verbatim, so the echo
/// returns the same bits.
fn same_value(pinned: ParamValue, echoed: ParamValue) -> bool {
    match (pinned, echoed) {
        (ParamValue::Int32(a), ParamValue::Int32(b)) => a == b,
        (ParamValue::Real32(a), ParamValue::Real32(b)) => a.to_bits() == b.to_bits(),
        _ => false,
    }
}
