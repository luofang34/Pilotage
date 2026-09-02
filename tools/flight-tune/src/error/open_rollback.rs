use std::error::Error;
use std::fmt;

use crate::AdapterError;

/// One reverse-cleanup operation of an open transaction.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OpenRollbackOperation {
    /// Release or contain the vehicle binding.
    VehicleBinding,
    /// Close the simulator session.
    SimulatorSession,
}

impl OpenRollbackOperation {
    /// Returns the stable diagnostic name of this operation.
    #[must_use]
    pub const fn name(self) -> &'static str {
        match self {
            Self::VehicleBinding => "release vehicle binding",
            Self::SimulatorSession => "close simulator session",
        }
    }
}

impl fmt::Display for OpenRollbackOperation {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.name())
    }
}

/// Every reverse-cleanup operation one open attempt ran, in its order.
///
/// A later operation runs after an earlier failure, so the report states
/// each outcome rather than the first one. An empty report states that the
/// attempt held nothing to release.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct OpenRollbackReport {
    outcomes: Vec<(OpenRollbackOperation, Option<AdapterError>)>,
}

impl OpenRollbackReport {
    /// Creates a report for an attempt that has released nothing.
    #[must_use]
    pub const fn new() -> Self {
        Self {
            outcomes: Vec::new(),
        }
    }

    pub(crate) fn record(
        &mut self,
        operation: OpenRollbackOperation,
        result: Result<(), AdapterError>,
    ) {
        self.outcomes.push((operation, result.err()));
    }

    /// Returns every operation this cleanup ran, in the order it ran them.
    pub fn operations(&self) -> impl Iterator<Item = OpenRollbackOperation> + '_ {
        self.outcomes.iter().map(|(operation, _)| *operation)
    }

    /// Returns every operation that failed, in the order it ran.
    pub fn failures(&self) -> impl Iterator<Item = (OpenRollbackOperation, &AdapterError)> + '_ {
        self.outcomes
            .iter()
            .filter_map(|(operation, error)| error.as_ref().map(|error| (*operation, error)))
    }

    /// States whether every operation this cleanup ran proved absence.
    #[must_use]
    pub fn is_complete(&self) -> bool {
        self.failures().next().is_none()
    }

    /// States whether this cleanup ran no operation at all.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.outcomes.is_empty()
    }
}

impl fmt::Display for OpenRollbackReport {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        if self.is_empty() {
            return formatter.write_str("no open resource required cleanup");
        }
        let mut separator = "";
        for (operation, error) in &self.outcomes {
            match error {
                Some(error) => write!(formatter, "{separator}{operation} failed: {error}")?,
                None => write!(formatter, "{separator}{operation} succeeded")?,
            }
            separator = "; ";
        }
        Ok(())
    }
}

impl Error for OpenRollbackReport {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        self.failures()
            .next()
            .map(|(_, error)| error as &(dyn Error + 'static))
    }
}
