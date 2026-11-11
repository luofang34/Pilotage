//! The heading and altitude path of a flight controller that holds
//! heading, altitude, and airspeed with its own sensors.

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
}
