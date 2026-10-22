use std::time::Duration;

use pilotage_adapter_api::{
    ApplyOutcome, Disposition, LinkLossEnactError, LinkLossPolicy, RejectReason,
};
use pilotage_protocol::{ControlIntent, ReferenceFrame, ScopedControlFrame, VelocityIntent};

use crate::adapter::{DjiAdapter, DjiControlMode, control::write_deadline, input::Framing};
use crate::flight::{VirtualStickBinding, VirtualStickVelocity};

impl DjiAdapter {
    pub(in crate::adapter) fn motion_enabled(&self) -> bool {
        self.experimental_virtual_stick
            && self.control.mode() == DjiControlMode::Observed
            && self.connection.input.framing() == Framing::LogicLink
    }

    pub(in crate::adapter) fn apply_motion(
        &mut self,
        frame: &ScopedControlFrame,
        now: Duration,
    ) -> ApplyOutcome {
        let result = self.admit_motion(frame, now);
        ApplyOutcome::new(
            self.tick(),
            match result {
                Ok(()) => Disposition::Accepted,
                Err(reason) => Disposition::Rejected(reason),
            },
        )
    }

    fn admit_motion(
        &mut self,
        frame: &ScopedControlFrame,
        now: Duration,
    ) -> Result<(), RejectReason> {
        if frame.vehicle != self.vehicle {
            return Err(RejectReason::UnknownVehicle);
        }
        if !self.motion_enabled() {
            return Err(RejectReason::UnknownScope);
        }
        if self.connection.motion.link_loss {
            return Err(RejectReason::LinkLossEngaged);
        }
        let Some(ControlIntent::Velocity(value)) = frame.intent else {
            return Err(RejectReason::Fenced);
        };
        if frame.carries_payload()
            || !frame.actions.is_empty()
            || !frame.action_ids.is_empty()
            || value.frame != ReferenceFrame::BodyFrd
        {
            return Err(RejectReason::Fenced);
        }
        let value = velocity_with_radial_limit(value)?;
        let until = write_deadline(frame, now)?;
        let binding = VirtualStickBinding {
            attachment: self.observations.incarnation().into_bytes(),
            session: frame.session.as_u64(),
            generation: frame.generation.as_u64(),
            model_revision: self.observations.model_revision(),
        };
        let mut input = super::MotionInput {
            binding,
            sequence: frame.sequence.as_u32(),
            value,
            until,
        };
        if !self
            .connection
            .flight
            .authority_valid(binding.session, binding.generation)
        {
            return Err(RejectReason::Fenced);
        }
        if self.connection.flight.holds_landing_motion(binding) {
            return self.connection.landing_motion(
                input,
                &self.observations,
                now,
                &mut self.control.sequence,
            );
        }
        if self.connection.motion.binding.is_none()
            && value == VirtualStickVelocity::ZERO
            && self.connection.input_clear()
            && self.connection.remote_control_neutral_cached(now)
            && self.observations.model_deadline(now).is_some()
            && self.observations.motion_deadline(now, false).is_none()
        {
            // Neutral client input must not acquire virtual-stick during native
            // takeoff or while on the ground.
            self.connection
                .flight
                .retain_authority(binding.session, binding.generation);
            return Ok(());
        }
        if self.connection.motion.stopped && self.connection.flight.take_landing_override(binding) {
            // The takeover gesture was neutral then deflected. Mode acquisition still
            // starts with zero; only subsequent fresh input can move the aircraft.
            self.connection.motion = super::Motion::default();
            input.value = VirtualStickVelocity::ZERO;
        }
        self.connection
            .accept_motion(input, now, &self.observations, &mut self.control.sequence)
    }

    pub(in crate::adapter) fn motion_link_loss(
        &mut self,
        policy: Option<LinkLossPolicy>,
        now: Duration,
    ) -> Result<(), LinkLossEnactError> {
        let reject = |detail: &str| LinkLossEnactError::ChannelRejected {
            detail: detail.to_owned(),
        };
        if let Some(policy) = policy {
            self.connection.motion.link_loss = true;
            self.connection.motion.stop(now);
            if policy != LinkLossPolicy::Neutralize {
                return Err(reject("unsupported DJI motion link-loss policy"));
            }
            // An asynchronous stop request is not a successful transport write.
            if self.connection.mode_session.active() || self.connection.mode_session.unresolved() {
                return Err(reject(
                    "DJI virtual-stick release is pending; keep stepping the adapter",
                ));
            }
            Ok(())
        } else {
            if self.connection.fault.is_some()
                || !self.connection.remote_control_neutral_cached(now)
                || self.connection.mode_session.active()
                || self.connection.mode_session.unresolved()
                || self.observations.model_deadline(now).is_none()
            {
                return Err(reject(
                    "DJI virtual-stick release and fresh model feedback are required",
                ));
            }
            self.connection.motion.link_loss = false;
            Ok(())
        }
    }
}

fn velocity_with_radial_limit(value: VelocityIntent) -> Result<VirtualStickVelocity, RejectReason> {
    if value.vx.abs() > 1.0 || value.vy.abs() > 1.0 {
        return Err(RejectReason::Other(
            "DJI horizontal axis exceeds its advertised limit".into(),
        ));
    }
    // The shared velocity contract bounds each axis separately. Preserve the
    // direction of a diagonal demand within the device's total-speed limit.
    let length = value.vx.hypot(value.vy);
    let scale = if length > 1.0 {
        (1.0 - f32::EPSILON) / length
    } else {
        1.0
    };
    VirtualStickVelocity::new(value.vx * scale, value.vy * scale, value.vz, value.yaw_rate)
        .map_err(|error| RejectReason::Other(error.to_string()))
}
