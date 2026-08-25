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
}
