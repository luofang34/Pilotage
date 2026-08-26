use thiserror::Error;

/// An error during tuning campaign evidence publication.
#[derive(Debug, Error)]
pub enum CampaignError {
    /// The journal cannot produce a stable evidence snapshot.
    #[error("cannot create a stable journal evidence snapshot: {source}")]
    Snapshot {
        /// The journal failure.
        #[source]
        source: Box<flight_tune::TuneError>,
    },
    /// Independent evidence verification failed.
    #[error("cannot verify campaign evidence: {source}")]
    Verification {
        /// The verification failure.
        #[source]
        source: Box<pilotage_tuning_feedback::FeedbackError>,
    },
    /// Durable evidence storage failed.
    #[error("cannot store campaign evidence: {source}")]
    Storage {
        /// The storage failure.
        #[source]
        source: Box<pilotage_tuning_feedback::FeedbackError>,
    },
}

pub(crate) fn snapshot(source: flight_tune::TuneError) -> CampaignError {
    CampaignError::Snapshot {
        source: Box::new(source),
    }
}

pub(crate) fn verification(source: pilotage_tuning_feedback::FeedbackError) -> CampaignError {
    CampaignError::Verification {
        source: Box::new(source),
    }
}

pub(crate) fn storage(source: pilotage_tuning_feedback::FeedbackError) -> CampaignError {
    CampaignError::Storage {
        source: Box::new(source),
    }
}
