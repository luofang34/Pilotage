//! The fixed-wing paths of the uplink: a velocity read as a course, an
//! airspeed, and a climb rate, and a heading with an altitude.

use pilotage_flight_uplink::HeadingAltitudeDemand;

use super::FlightUplink;

impl FlightUplink {
    /// Sends the frames of one heading and altitude demand. Returns whether
    /// every frame the policy made was sent: a refused send is counted in
    /// [`Self::send_failures`] and is a demand the flight controller did
    /// not receive.
    pub fn send_heading_altitude(&mut self, demand: HeadingAltitudeDemand) -> bool {
        let failures_before = self.send_failures;
        for frame in self.policy.heading_altitude_frames(self.elapsed(), demand) {
            self.send(&frame);
        }
        self.send_failures == failures_before
    }

    /// Sends one fixed-wing velocity setpoint in the local NED frame, with
    /// the yaw held when the velocity has no horizontal direction. Returns
    /// whether the setpoint was sent, or nothing was due.
    pub fn send_fixed_wing_velocity(&mut self, velocity_ned_mps: [f32; 3], yaw_rad: f32) -> bool {
        let failures_before = self.send_failures;
        let now = self.elapsed();
        if let Some(frame) = self
            .policy
            .fixed_wing_velocity_frame(now, velocity_ned_mps, yaw_rad)
        {
            self.send(&frame);
        }
        self.send_failures == failures_before
    }
}
