//! Runtime policy switches for a host instance.

/// Runtime policy switches for a host instance.
///
/// [`super::start`] derives these from the process environment; tests inject
/// them directly through [`super::start_with_options`].
#[derive(Debug, Clone, Copy, Default)]
pub struct RuntimeOptions {
    /// SIMULATION compatibility: admit numeric legacy payload frames at the
    /// gate's translation boundary. Production control is TYPED-ONLY —
    /// legacy payloads bypass profile-activation binding and carry
    /// uncorrelated edges — so this never rides by default;
    /// [`super::start`] turns it on only for the explicit
    /// `PILOTAGE_LEGACY_COMPAT=1` opt-in.
    pub legacy_compatibility: bool,
}

impl RuntimeOptions {
    pub(super) fn from_env() -> Self {
        let legacy_compatibility = matches!(
            std::env::var("PILOTAGE_LEGACY_COMPAT").as_deref(),
            Ok("1" | "true")
        );
        Self {
            legacy_compatibility,
        }
    }
}
