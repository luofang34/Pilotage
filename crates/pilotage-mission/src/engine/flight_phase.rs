//! Flight phase and Required Navigation Performance (RNP) derivation.
//!
//! Each planned leg may declare a [`LegRole`] (or derive one from ARINC 424
//! / CIFP procedure metadata via [`LegRole::from_cifp`]) and an optional
//! procedure-specified RNP in nautical miles. [`PhaseMachine`] advances
//! deterministically across the active leg order so HSI CDI scaling and phase
//! annunciations switch cleanly between `ENR` (`2.0 NM`), `TERM` (`1.0 NM`),
//! and `APR` (`0.3 NM` or tighter procedure RNP).

use super::{FlightPhase, MissionEngine};

#[cfg(test)]
mod tests;

/// Exact ICAO nautical mile in metres.
pub const M_PER_NM: f64 = 1_852.0;

/// Standard RNP for [`FlightPhase::Enroute`], nautical miles.
pub const ENROUTE_RNP_NM: f64 = 2.0;

/// Standard RNP for [`FlightPhase::Terminal`] and [`FlightPhase::Missed`],
/// nautical miles.
pub const TERMINAL_RNP_NM: f64 = 1.0;

/// Standard RNP for [`FlightPhase::Approach`], nautical miles.
pub const APPROACH_RNP_NM: f64 = 0.3;

/// ARINC 424 / CIFP instrument procedure family.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum CifpProcedureKind {
    /// Standard Instrument Departure (SID / DP).
    Sid,
    /// Standard Terminal Arrival (STAR).
    Star,
    /// Instrument Approach Procedure (IAP).
    Approach,
}

/// ARINC 424 waypoint description role on an instrument procedure.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ProcedureFixRole {
    /// Initial Approach Fix (`IAF`).
    InitialApproachFix,
    /// Intermediate Fix (`IF`).
    IntermediateFix,
    /// Final Approach Course Fix (`FACF`).
    FinalApproachCourseFix,
    /// Final Approach Fix (`FAF` / `PFAF`).
    FinalApproachFix,
    /// Step-down fix inside the final approach segment.
    StepDownFix,
    /// Missed Approach Point (`MAPt`).
    MissedApproachPoint,
    /// Missed Approach Holding Fix (`MAHF`).
    MissedApproachHoldingFix,
    /// Final End Point (`FEP`).
    FinalEndpoint,
    /// Runway threshold or landing point.
    Runway,
}

/// Operational role of a leg in a flight plan.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum LegRole {
    /// Standard instrument departure or departure transition leg.
    Departure,
    /// En-route leg between terminal areas.
    Enroute,
    /// Terminal arrival or initial/intermediate approach segment leg.
    Arrival,
    /// Final approach segment leg (past the final approach fix to MAPt/runway).
    Approach,
    /// Missed approach segment leg (beyond the missed approach point).
    MissedApproach,
}

impl LegRole {
    /// Classifies an ARINC 424 / CIFP procedure leg from its procedure family,
    /// terminating fix role, and whether the leg lies past the missed approach
    /// point.
    #[must_use]
    pub const fn from_cifp(
        procedure: CifpProcedureKind,
        fix_role: Option<ProcedureFixRole>,
        after_missed_approach_point: bool,
    ) -> Self {
        match procedure {
            CifpProcedureKind::Sid => Self::Departure,
            CifpProcedureKind::Star => Self::Arrival,
            CifpProcedureKind::Approach => {
                if after_missed_approach_point
                    || matches!(fix_role, Some(ProcedureFixRole::MissedApproachHoldingFix))
                {
                    Self::MissedApproach
                } else if matches!(
                    fix_role,
                    Some(
                        ProcedureFixRole::StepDownFix
                            | ProcedureFixRole::MissedApproachPoint
                            | ProcedureFixRole::FinalEndpoint
                            | ProcedureFixRole::Runway
                    )
                ) {
                    Self::Approach
                } else {
                    Self::Arrival
                }
            }
        }
    }

    /// Returns the [`FlightPhase`] directly associated with this leg role.
    #[must_use]
    pub const fn flight_phase(self) -> FlightPhase {
        match self {
            Self::Enroute => FlightPhase::Enroute,
            Self::Departure | Self::Arrival => FlightPhase::Terminal,
            Self::Approach => FlightPhase::Approach,
            Self::MissedApproach => FlightPhase::Missed,
        }
    }
}

/// Deterministic state machine that resolves [`FlightPhase`] across sequenced
/// legs.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct PhaseMachine {
    last_role: Option<LegRole>,
}

impl PhaseMachine {
    /// Creates a machine prior to any terminal or approach procedure leg.
    #[must_use]
    pub const fn new() -> Self {
        Self { last_role: None }
    }

    /// Advances the phase machine onto a leg carrying `role` and returns the
    /// resulting [`FlightPhase`].
    ///
    /// - Explicit `Some(role)` transitions immediately to `role.flight_phase()`.
    /// - An untagged leg (`None`) after no role, [`LegRole::Enroute`], or
    ///   [`LegRole::Departure`] resolves to [`FlightPhase::Enroute`] (leaving a
    ///   SID onto untagged route fixes enters en-route scaling).
    /// - An untagged leg inside [`LegRole::Arrival`], [`LegRole::Approach`], or
    ///   [`LegRole::MissedApproach`] preserves that procedure segment's phase.
    pub const fn step(&mut self, role: Option<LegRole>) -> FlightPhase {
        match role {
            Some(explicit) => {
                self.last_role = Some(explicit);
                explicit.flight_phase()
            }
            None => match self.last_role {
                None | Some(LegRole::Enroute | LegRole::Departure) => {
                    self.last_role = Some(LegRole::Enroute);
                    FlightPhase::Enroute
                }
                Some(LegRole::Arrival) => FlightPhase::Terminal,
                Some(LegRole::Approach) => FlightPhase::Approach,
                Some(LegRole::MissedApproach) => FlightPhase::Missed,
            },
        }
    }

    /// Evaluates the [`FlightPhase`] at `active_index` over `roles`.
    #[must_use]
    pub fn evaluate(roles: &[Option<LegRole>], active_index: usize) -> FlightPhase {
        if roles.is_empty() {
            return FlightPhase::Enroute;
        }
        let mut machine = Self::new();
        let mut phase = FlightPhase::Enroute;
        for role in roles.iter().take(active_index.saturating_add(1)) {
            phase = machine.step(*role);
        }
        phase
    }
}

/// Returns the standard RNP in nautical miles for `phase`.
#[must_use]
pub const fn default_rnp_nm(phase: FlightPhase) -> f64 {
    match phase {
        FlightPhase::Enroute => ENROUTE_RNP_NM,
        FlightPhase::Terminal | FlightPhase::Missed => TERMINAL_RNP_NM,
        FlightPhase::Approach => APPROACH_RNP_NM,
    }
}

/// Returns the effective RNP in nautical miles for `phase`, tightening to
/// `procedure_rnp_nm` when a finite positive procedure RNP is tighter than the
/// phase default.
#[must_use]
pub fn effective_rnp_nm(phase: FlightPhase, procedure_rnp_nm: Option<f64>) -> f64 {
    let default_nm = default_rnp_nm(phase);
    procedure_rnp_nm
        .filter(|value| value.is_finite() && *value > 0.0)
        .map_or(default_nm, |specified| default_nm.min(specified))
}

impl MissionEngine {
    /// The flight phase of the active leg (or terminal leg after plan
    /// completion).
    #[must_use]
    pub fn flight_phase(&self) -> FlightPhase {
        self.route.flight_phase()
    }

    /// Required Navigation Performance of the active leg in nautical miles.
    #[must_use]
    pub fn rnp_nm(&self) -> f64 {
        let phase = self.flight_phase();
        effective_rnp_nm(phase, self.route.active_rnp_nm())
    }

    /// Required Navigation Performance of the active leg in metres.
    #[must_use]
    pub fn rnp_m(&self) -> f64 {
        self.rnp_nm() * M_PER_NM
    }
}
