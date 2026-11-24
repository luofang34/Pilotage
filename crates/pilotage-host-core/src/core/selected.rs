//! The targets an operator selects in place of the plan's: a true
//! heading, an altitude, and an indicated airspeed. The core keeps the
//! selection and gives it to each mission engine it steps. An engine is
//! built only when its session knows the vehicle's capabilities, and a
//! plan loaded in flight has its own engine. Thus the core gives the
//! selection again before each mission tick, and a principal that takes
//! over flies the same selection as the one it replaces.

use pilotage_mission::SelectedTargets;
use tracing::warn;

use super::HostCore;
use crate::error::HostCoreError;

impl HostCore {
    /// Replaces the selection with `targets`. A selection with no target
    /// flies the plan. The mission flies the selection from its next
    /// tick, in flight only. The selection stays in effect when a plan
    /// loaded later takes over.
    ///
    /// # Errors
    ///
    /// [`HostCoreError::NoMission`] when no mission is loaded, and
    /// [`HostCoreError::Select`] when the mission does not fly `targets`:
    /// a target is not a finite number, the airspeed is not above zero,
    /// or the vehicle can stop. A refused selection leaves the current
    /// one in effect.
    pub fn select_targets(&mut self, targets: SelectedTargets) -> Result<(), HostCoreError> {
        if self.principal.is_none() {
            return Err(HostCoreError::NoMission {
                operation: "select_targets",
            });
        }
        // The same check as the engine's, made here because the engine of
        // a principal is not built until its session is open.
        self.selected = targets
            .checked(self.config.mission.can_stop())
            .map_err(|source| HostCoreError::Select { source })?;
        self.apply_selection();
        Ok(())
    }

    /// The selection in effect. No target is selected until
    /// [`Self::select_targets`] sets one.
    #[must_use]
    pub fn selected_targets(&self) -> SelectedTargets {
        self.selected
    }

    /// Gives the selection to each mission engine that does not have it.
    pub(super) fn apply_selection(&mut self) {
        let selected = self.selected;
        for principal in self.principals_mut() {
            let Some(mission) = principal.mission_mut() else {
                continue;
            };
            if mission.selected() == selected {
                continue;
            }
            if let Err(error) = mission.select(selected) {
                warn!(%error, "a mission engine refused the selected targets");
            }
        }
    }
}
