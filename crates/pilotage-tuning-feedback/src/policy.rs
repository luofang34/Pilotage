//! The bar a consumer requires a campaign to have been run against.

use flight_tune::{Digest, PromotionPolicy, QualificationPolicy, SearchStage};

use crate::{FeedbackError, digest, error::invalid};

/// Domain separators for the two halves of a campaign bar. Both end in a
/// zero byte so no encoded policy can be read as belonging to the other.
const PROMOTION_DOMAIN: &[u8] = b"pilotage-tuning-feedback:promotion-policy:v1\0";
const QUALIFICATION_DOMAIN: &[u8] = b"pilotage-tuning-feedback:qualification-policy:v1\0";

/// The promotion and final qualification policies a campaign must have run
/// against.
///
/// Verification without one answers "is this campaign internally consistent
/// under the policy its own operator wrote", which is not the question
/// anyone installing a calibration is asking. The policies decide which
/// candidate ships, and a campaign run against a bar nobody set reconciles
/// exactly as well as one run against the real bar: every digest matches,
/// nothing is found over limit, and the evidence reads as qualified.
///
/// Requiring the bar as an argument is what makes it impossible to verify
/// without stating which bar was cleared.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RequiredPolicy {
    promotion: Digest,
    qualification: Digest,
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
    ) -> Result<Self, FeedbackError> {
        Ok(Self {
            promotion: digest::domain("promotion policy", PROMOTION_DOMAIN, promotion)?,
            qualification: digest::domain(
                "qualification policy",
                QUALIFICATION_DOMAIN,
                qualification,
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
        Ok(())
    }
}
