//! Process-owner and launch-gate helper for Aviate simulator tuning.

fn main() -> Result<(), flight_tune_aviate::AviateSupervisorError> {
    flight_tune_aviate::supervisor_main_blocking()
}
