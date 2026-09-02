use crate::{
    CampaignBackend, Journal, OpenRollbackOperation, OpenRollbackReport, SessionChallenge,
    SimulatorCapability, SimulatorSessionAcquisition, SimulatorVehicleFactory, TuneError,
    VehicleBinding, VehicleBindingAcquisition, VehicleBindingRollback,
};

use super::super::session;

/// One open attempt that owns every resource it acquires until it commits.
///
/// The transaction holds the simulator session and the vehicle binding.
/// Neither reaches a tuner before every open check passes, and any check
/// that fails first runs the reverse cleanup for whatever the attempt can
/// have acquired.
///
/// A resource counts as acquired from the moment the operation that
/// creates it starts, not from the moment it returns. An operation that
/// fails can still have left the resource behind, and an acquisition
/// identity that names nothing is cleaned up at no cost.
pub(super) struct OpenTransaction<B, F: SimulatorVehicleFactory> {
    backend: B,
    rollback: F::Rollback,
    session: SimulatorSessionAcquisition,
    vehicle: VehicleBindingAcquisition,
    holds_session: bool,
    holds_vehicle: bool,
}

impl<B, F> OpenTransaction<B, F>
where
    B: CampaignBackend,
    F: SimulatorVehicleFactory,
{
    /// Starts one open attempt against the identities the journal states.
    ///
    /// Every acquisition identity comes from the journal session, so an
    /// attempt names the same resources that every earlier attempt on this
    /// campaign named, before it acquires anything.
    pub(super) fn begin(backend: B, factory: &F, journal: &Journal) -> Result<Self, TuneError> {
        let runtimes = &journal.session().runtimes;
        let scenario_runtime =
            runtimes
                .scenario_runtime
                .as_ref()
                .ok_or_else(|| TuneError::InvalidIdentity {
                    detail: "the journal session states no scenario runtime identity".to_owned(),
                })?;
        let session_digest = journal.session_digest()?;
        Ok(Self {
            backend,
            rollback: factory.rollback_handle(),
            session: SimulatorSessionAcquisition::new(
                session_digest,
                runtimes.simulator.digest,
                runtimes.airframe.digest,
            ),
            vehicle: VehicleBindingAcquisition::new(
                session_digest,
                runtimes.vehicle.digest,
                scenario_runtime.digest,
            ),
            holds_session: false,
            holds_vehicle: false,
        })
    }

    /// Proves that no earlier open attempt left a resource behind.
    ///
    /// A rollback that ended in an uncertain acknowledgement leaves the
    /// same acquisition identities this attempt would use. The adapters
    /// answer for their own resources, so the attempt asks them to prove
    /// absence in reverse acquisition order and refuses to acquire
    /// anything until they do.
    ///
    /// # Errors
    ///
    /// Returns [`TuneError::OpenNotReconciled`] when either adapter cannot
    /// prove that the earlier resource is absent.
    pub(super) fn reconcile_prior_open_blocking(&mut self) -> Result<(), TuneError> {
        let mut report = OpenRollbackReport::new();
        report.record(
            OpenRollbackOperation::VehicleBinding,
            self.rollback.release_binding_blocking(&self.vehicle),
        );
        report.record(
            OpenRollbackOperation::SimulatorSession,
            self.backend.close_session_blocking(&self.session),
        );
        if report.is_complete() {
            Ok(())
        } else {
            Err(TuneError::OpenNotReconciled { report })
        }
    }

    /// Opens one simulator session and validates its exact receipt.
    ///
    /// # Errors
    ///
    /// Returns [`TuneError`] when the simulator refuses the challenge or
    /// answers with a receipt that names another session, simulator, or
    /// airframe. Either failure rolls the attempt back first.
    pub(super) fn open_session_blocking(
        &mut self,
        journal: &Journal,
    ) -> Result<SimulatorCapability, TuneError> {
        let challenge = SessionChallenge::new(self.session.session_digest());
        self.holds_session = true;
        let receipt = match self.backend.open_session_blocking(&challenge) {
            Ok(receipt) => receipt,
            Err(source) => {
                return Err(self.fail(TuneError::Adapter {
                    adapter: self.backend.simulator_identity().id.clone(),
                    operation: "open simulator session",
                    source,
                }));
            }
        };
        if let Err(mismatch) = session::validate_simulator_receipt(journal, receipt) {
            return Err(self.fail(mismatch));
        }
        Ok(SimulatorCapability::new(receipt))
    }

    /// Binds one vehicle and validates its exact binding receipt.
    ///
    /// # Errors
    ///
    /// Returns [`TuneError`] when the factory refuses the capability or
    /// returns a binding that names another session, vehicle, or
    /// transition policy. Either failure rolls the attempt back first.
    pub(super) fn bind_vehicle_blocking(
        &mut self,
        journal: &Journal,
        factory: F,
        capability: &SimulatorCapability,
    ) -> Result<VehicleBinding<F::Adapter>, TuneError> {
        let vehicle_id = factory.vehicle_identity().id.clone();
        self.holds_vehicle = true;
        let binding = match factory.bind_blocking(capability) {
            Ok(binding) => binding,
            Err(source) => {
                return Err(self.fail(TuneError::Adapter {
                    adapter: vehicle_id,
                    operation: "bind simulator vehicle",
                    source,
                }));
            }
        };
        if let Err(mismatch) = session::validate_vehicle_binding(journal, &binding) {
            return Err(self.fail(mismatch));
        }
        Ok(binding)
    }

    /// Returns the backend for the open checks that run before the commit.
    pub(super) fn backend_mut(&mut self) -> &mut B {
        &mut self.backend
    }

    /// Rolls the attempt back and states the primary failure with it.
    ///
    /// Cleanup runs in reverse acquisition order, and every applicable
    /// operation runs even after an earlier one fails. The primary failure
    /// stands alone when cleanup proved absence, because there is then
    /// nothing further to state.
    pub(super) fn fail(&mut self, primary: TuneError) -> TuneError {
        let report = self.roll_back_blocking();
        if report.is_complete() {
            primary
        } else {
            TuneError::OpenAndRollbackFailed {
                primary: Box::new(primary),
                rollback: report,
            }
        }
    }

    /// Transfers every acquired resource out of the transaction.
    ///
    /// The transaction is consumed here, so nothing it held can be rolled
    /// back after the tuner owns it.
    pub(super) fn commit(self) -> B {
        self.backend
    }

    fn roll_back_blocking(&mut self) -> OpenRollbackReport {
        let mut report = OpenRollbackReport::new();
        if self.holds_vehicle {
            report.record(
                OpenRollbackOperation::VehicleBinding,
                self.rollback.release_binding_blocking(&self.vehicle),
            );
        }
        if self.holds_session {
            report.record(
                OpenRollbackOperation::SimulatorSession,
                self.backend.close_session_blocking(&self.session),
            );
        }
        if report.is_complete() {
            self.holds_vehicle = false;
            self.holds_session = false;
        }
        report
    }
}
