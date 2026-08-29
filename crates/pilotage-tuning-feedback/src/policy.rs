//! The bar a consumer requires a campaign to have been run against.

use flight_tune::{
    Digest, ExecutionRetryPolicy, PromotionPolicy, QualificationPolicy, ResponseTargetTable,
    SearchStage,
};

use crate::{FeedbackError, digest, error::invalid};

/// Domain separators for the two halves of a campaign bar. Both end in a
/// zero byte so no encoded policy can be read as belonging to the other.
const PROMOTION_DOMAIN: &[u8] = b"pilotage-tuning-feedback:promotion-policy:v1\0";
const QUALIFICATION_DOMAIN: &[u8] = b"pilotage-tuning-feedback:qualification-policy:v1\0";
const EXECUTION_RETRY_DOMAIN: &[u8] = b"pilotage-tuning-feedback:execution-retry-policy:v1\0";
const RESPONSE_TARGET_DOMAIN: &[u8] = b"pilotage-tuning-feedback:response-target-table:v1\0";

/// The promotion, final qualification, execution retry, and scoped response
/// target policies a campaign must have run against.
///
/// Verification without one answers "is this campaign internally consistent
/// under the policy its own operator wrote", which is not the question
/// anyone installing a calibration is asking. The policies decide which
/// candidate ships, and a campaign run against a bar nobody set reconciles
/// exactly as well as one run against the real bar: every digest matches,
/// nothing is found over limit, and the evidence reads as qualified.
///
/// The retry limit belongs in the same bar. A campaign that was free to
/// discard failed executions and rerun them states weaker evidence than one
/// that was not, and a consumer that never said which it asked for cannot
/// tell the two apart.
///
/// The scoped response target table is the fourth half, and it is the one
/// that carries every number. The two policies name the objectives; the table
/// states the limit each objective has for each exact scenario. A bar stated
/// without it would name what was measured and nothing about how well.
///
/// Requiring the bar as an argument is what makes it impossible to verify
/// without stating which bar was cleared.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RequiredPolicy {
    promotion: Digest,
    qualification: Digest,
    execution_retry: Digest,
    response_targets: Digest,
}

impl RequiredPolicy {
    /// Binds one promotion policy and one final qualification policy.
    ///
    /// # Errors
    ///
    /// Returns [`FeedbackError`] when a policy cannot be encoded.
    pub fn new(
        promotion: &PromotionPolicy,
        qualification: &QualificationPolicy,
        execution_retry: &ExecutionRetryPolicy,
        response_targets: &ResponseTargetTable,
    ) -> Result<Self, FeedbackError> {
        Ok(Self {
            promotion: digest::domain("promotion policy", PROMOTION_DOMAIN, promotion)?,
            qualification: digest::domain(
                "qualification policy",
                QUALIFICATION_DOMAIN,
                qualification,
            )?,
            execution_retry: digest::domain(
                "execution retry policy",
                EXECUTION_RETRY_DOMAIN,
                execution_retry,
            )?,
            response_targets: digest::domain(
                "response target table",
                RESPONSE_TARGET_DOMAIN,
                response_targets,
            )?,
        })
    }

    /// Returns the required promotion policy identity.
    #[must_use]
    pub const fn promotion(&self) -> Digest {
        self.promotion
    }

    /// Returns the required final qualification policy identity.
    #[must_use]
    pub const fn qualification(&self) -> Digest {
        self.qualification
    }

    /// Returns the required execution retry policy identity.
    #[must_use]
    pub const fn execution_retry(&self) -> Digest {
        self.execution_retry
    }

    /// Returns the required scoped response target table identity.
    #[must_use]
    pub const fn response_targets(&self) -> Digest {
        self.response_targets
    }

    /// Requires that this stage carries exactly the bound policies.
    ///
    /// # Errors
    ///
    /// Returns [`FeedbackError`] when either half differs.
    pub(crate) fn verify(&self, stage: &SearchStage) -> Result<(), FeedbackError> {
        let promotion = digest::domain("promotion policy", PROMOTION_DOMAIN, &stage.promotion)?;
        if promotion != self.promotion {
            return Err(invalid(
                "the campaign ran against a different promotion policy",
            ));
        }
        let qualification = digest::domain(
            "qualification policy",
            QUALIFICATION_DOMAIN,
            &stage.qualification,
        )?;
        if qualification != self.qualification {
            return Err(invalid(
                "the campaign ran against a different final qualification policy",
            ));
        }
        let execution_retry = digest::domain(
            "execution retry policy",
            EXECUTION_RETRY_DOMAIN,
            &stage.execution_retry,
        )?;
        if execution_retry != self.execution_retry {
            return Err(invalid(
                "the campaign ran against a different execution retry policy",
            ));
        }
        let response_targets = digest::domain(
            "response target table",
            RESPONSE_TARGET_DOMAIN,
            &stage.response_targets,
        )?;
        if response_targets != self.response_targets {
            return Err(invalid(
                "the campaign ran against different scoped response targets",
            ));
        }
        Ok(())
    }
}
