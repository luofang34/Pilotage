use flight_tune::Digest;

#[derive(Debug, Default)]
pub struct FakeVehicleState {
    pub gain: f64,
    pub active_candidate_digest: Option<Digest>,
    pub bind_count: usize,
    pub ensure_count: usize,
    pub apply_count: usize,
    pub bad_candidate_readback_on_ensure: Option<usize>,
    pub bad_candidate_readback_on_apply: Option<usize>,
    /// Whether a bind left a vehicle binding the factory has to release.
    pub bound: bool,
    /// Every release the factory rollback handle has answered.
    pub release_count: usize,
    /// The bind fails after it records a partial binding.
    pub fail_bind: bool,
    /// The binding receipt names another scenario runtime.
    pub bad_binding_receipt: bool,
    /// The release at this one-based count fails.
    pub fail_release_on: Option<usize>,
}
