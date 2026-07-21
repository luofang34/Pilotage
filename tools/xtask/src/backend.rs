//! The FC-backend seam: each backend plans its simulator/FC stages and
//! contributes the host's adapter and environment. The launcher itself
//! stays backend-agnostic so future FC families (PX4, JSBSim) are new
//! implementations, not new orchestration.

use std::path::PathBuf;

use crate::cli::Profile;
use crate::error::XtaskError;
use crate::process::ProcessSpec;
use crate::readiness::Readiness;

pub(crate) mod aviate_gz;
mod px4_gz;

/// Everything a backend may need to plan its stages.
#[derive(Debug)]
pub struct SessionContext {
    /// Workspace root of this repository.
    pub repo_root: PathBuf,
    /// Host WebTransport port.
    pub host_port: u16,
    /// Static viewer port.
    pub viewer_port: u16,
    /// Session profile handed to the host.
    pub profile: Profile,
    /// Directory stage logs are written under.
    pub log_dir: PathBuf,
}

/// One plannable launch step: a process and the signal proving it is up.
#[derive(Debug)]
pub struct Stage {
    /// The process to run.
    pub spec: ProcessSpec,
    /// The readiness signal to wait for before the next stage.
    pub readiness: Readiness,
}

/// A launchable FC/simulator family.
pub trait SimBackend {
    /// Backend name as selected by `--fc`.
    fn name(&self) -> &'static str;
    /// The session host `--adapter` this backend's telemetry plane uses.
    fn host_adapter(&self) -> &'static str;
    /// Extra environment the host needs for this backend.
    fn host_env(&self, ctx: &SessionContext) -> Vec<(String, String)>;
    /// Validates tools/artifacts and plans the simulator and FC stages,
    /// in launch order.
    ///
    /// # Errors
    ///
    /// Returns [`XtaskError::MissingArtifact`] with an actionable hint
    /// when a required tool or build product is absent.
    fn plan(&self, ctx: &SessionContext) -> Result<Vec<Stage>, XtaskError>;
    /// Builds this backend's own gitignored artifacts so a fresh checkout
    /// runs out of the box. Best-effort by contract: a backend whose extra
    /// artifact only enriches the session (e.g. camera video that degrades to
    /// no-video) must warn and return `Ok` when its toolchain is absent, so a
    /// missing optional dependency never blocks the flight. The default is a
    /// no-op for backends with nothing extra to build.
    ///
    /// # Errors
    ///
    /// Returns a typed [`XtaskError`] only for a failure that must abort the
    /// session; recoverable/optional build failures are logged, not returned.
    fn prepare(&self, ctx: &SessionContext) -> Result<(), XtaskError> {
        let _ = ctx;
        Ok(())
    }
    /// `pgrep -f` patterns that mark a stale session of this backend.
    fn stale_process_patterns(&self) -> Vec<&'static str>;
    /// Resets the running simulation world and FC.
    ///
    /// # Errors
    ///
    /// Returns [`XtaskError::CommandFailed`] when the reset reports
    /// failure.
    fn reset(&self, repo_root: &std::path::Path) -> Result<(), XtaskError>;
}

/// Resolves `--fc` to a backend, fail-closed on unknown names.
///
/// # Errors
///
/// Returns [`XtaskError::UnknownBackend`] for any name this launcher
/// does not implement.
pub fn backend_for(name: &str) -> Result<Box<dyn SimBackend>, XtaskError> {
    match name {
        // Canonical names pair the FC family with the simulator behind
        // it; the bare FC name stays accepted as the family's default.
        "aviate-gz" | "aviate" => Ok(Box::new(aviate_gz::AviateGz)),
        "px4-gz" | "px4" => Ok(Box::new(px4_gz::Px4Gz)),
        _ => Err(XtaskError::UnknownBackend {
            name: name.to_owned(),
        }),
    }
}

#[cfg(test)]
#[allow(clippy::expect_used, clippy::panic)]
mod tests {
    use super::backend_for;
    use crate::error::XtaskError;

    #[test]
    fn backend_selection_fails_closed() {
        assert_eq!(backend_for("aviate").expect("known").name(), "aviate-gz");
        assert_eq!(backend_for("aviate-gz").expect("known").name(), "aviate-gz");
        assert_eq!(backend_for("px4").expect("known").name(), "px4-gz");
        assert_eq!(backend_for("px4-gz").expect("known").name(), "px4-gz");
        let refusal = backend_for("px4-jsbsim");
        assert!(matches!(refusal, Err(XtaskError::UnknownBackend { .. })));
    }
}
