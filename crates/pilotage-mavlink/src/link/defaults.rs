//! The state a link starts in, before any frame has been read.

use std::time::Instant;

use pilotage_adapter_api::SourceIncarnation;

use super::estimator;
use super::{AuthorizationSource, LinkState, ResetPolicy, measurement};

impl Default for LinkState {
    fn default() -> Self {
        Self {
            system_id: 1,
            component_id: 1,
            source_id: 1,
            source_incarnation: SourceIncarnation::new([0; 16]),
            reset_policy: ResetPolicy::Conservative,
            authorization_source: AuthorizationSource::AviatePrivate,
            standard_status_max_lag_ms: estimator::DEFAULT_STANDARD_STATUS_MAX_LAG_MS,
            reset_candidate_max_ms: measurement::DEFAULT_RESET_CANDIDATE_MAX_MS,
            maximum_inter_group_skew_ms: 0,
            attitude: None,
            kinematics: None,
            estimator_status: None,
            baro: None,
            sim_truth: None,
            gnss_fix: None,
            started_at: Instant::now(),
            truth_origin: None,
            gimbal_device: None,
            last_command_ack: None,
            gimbal_configure_ack: None,
            last_heartbeat: None,
            heartbeat_armed: None,
            decoded: 0,
            crc_failures: 0,
            unknown_ids: 0,
            source_epoch: 1,
            last_source_time_ms: None,
            last_accepted_at: None,
            pending_reset: None,
            duplicate_measurements: 0,
            reordered_measurements: 0,
            invalid_estimator_statuses: 0,
            source_resets: 0,
            suspected_resets: 0,
            wrong_sources: 0,
        }
    }
}
