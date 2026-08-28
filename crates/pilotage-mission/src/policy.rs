//! Fixed policy for the operational mission document.

/// Maximum retries after the first arm attempt.
///
/// Three retries permit four total attempts. This limit handles short
/// rejection bursts and prevents an unlimited action loop.
pub const OPERATIONAL_RETRY_LIMIT: u16 = 3;

/// Maximum wall-clock wait for one directive receipt, in nanoseconds.
///
/// Five seconds is longer than the reliable local action round trip. It
/// still stops a run promptly when the action path does not answer.
pub const OPERATIONAL_RECEIPT_TIMEOUT_NS: u64 = 5_000_000_000;

/// Maximum wall-clock duration of one operational mission, in nanoseconds.
///
/// The 24-hour limit permits a full-day session. It also prevents an
/// operational run from having an unlimited lifetime.
pub const OPERATIONAL_WALL_DEADLINE_NS: u64 = 86_400_000_000_000;

/// Maximum simulator-time duration of the arm phase, in nanoseconds.
///
/// The two-minute limit includes the wait for a navigation solution and the
/// reliable arm receipt.
pub const ARM_PHASE_DEADLINE_NS: u64 = 120_000_000_000;

/// Maximum simulator-time duration of the climb phase, in nanoseconds.
///
/// The 30-minute limit is larger than the configured operational climb and
/// still bounds a vehicle that cannot capture altitude.
pub const CLIMB_PHASE_DEADLINE_NS: u64 = 1_800_000_000_000;

/// Maximum simulator-time duration of the follow-plan phase, in nanoseconds.
///
/// The 23-hour limit permits a long route. The 24-hour mission wall deadline
/// remains the final bound for the complete run.
pub const FOLLOW_PLAN_PHASE_DEADLINE_NS: u64 = 82_800_000_000_000;
