import Foundation

/// Where the aircraft points and where it is going: the two directions a moving map
/// draws beside a position.
///
/// They are not the same direction. Heading is where the nose points; track is where
/// the aircraft is actually travelling. In wind they differ by the crab angle, and that
/// difference is what tells a reader the aircraft is crabbing rather than turning. A
/// display that drew one and called it the other would be inventing an attitude nobody
/// measured.
///
/// The rules here are shared with the web client rather than reinvented: both are held
/// to one corpus of cases, so the two clients cannot come to differ about what the same
/// report means.
public enum OwnshipMotion {
    /// Below this ground speed a track direction is noise, not a bearing.
    ///
    /// An aircraft holding station reports a little drift whose direction wanders
    /// through the whole compass. Drawing it would claim a course the aircraft is not
    /// on.
    ///
    /// Stated in metres per second because the threshold is a physical speed that both
    /// clients have to agree on. A round number in each client's own unit is two
    /// different speeds, and a reader comparing the two displays would see the course
    /// appear on one before the other.
    private static let trackFloorMetresPerSecond: Double = 0.5

    /// The speed a course already drawn must fall below before it is taken away.
    ///
    /// Without a band between the two, an aircraft drifting either side of the floor
    /// flickers its course on and off at the report rate.
    private static let trackReleaseMetresPerSecond: Double = 0.35

    /// The floor in the unit a surveillance report states its speed in.
    public static var trackFloorKnots: Double {
        trackFloorMetresPerSecond * knotsPerMetrePerSecond
    }

    /// The release speed in the unit a surveillance report states its speed in.
    public static var trackReleaseKnots: Double {
        trackReleaseMetresPerSecond * knotsPerMetrePerSecond
    }

    /// How far ahead the velocity leader reaches, in seconds.
    ///
    /// The line ends where the aircraft arrives if it holds this velocity, so the
    /// duration is what makes its length mean anything; a leader with no stated
    /// look-ahead is a line of arbitrary length. Sixty seconds is the usual choice on a
    /// situation display.
    public static let leaderSeconds: Double = 60

    /// Metres per degree of latitude, and of longitude at the equator.
    private static let metresPerDegree: Double = 111_111

    /// The Earth radius that constant implies, so a leader and a local frame do not
    /// disagree about the size of the Earth.
    private static let earthRadiusMetres: Double = metresPerDegree * 180 / .pi

    /// One metre per second in knots.
    private static let knotsPerMetrePerSecond: Double = 1.943_844_49

    /// Which north a reported heading is measured from.
    public enum HeadingReference: Equatable, Sendable {
        /// Measured from true north.
        case trueNorth
        /// Measured from magnetic north.
        case magneticNorth
        /// The source stated a reference this display does not know.
        case other
    }

    /// The heading a map drawing in true north may turn a mark to, or nothing.
    ///
    /// A heading is a number and a reference. Magnetic and true differ by the local
    /// variation, which is tens of degrees in places, and a display with no variation
    /// model cannot convert one into the other. A heading not stated against true north
    /// is therefore withheld rather than drawn as though it were one — the mark loses
    /// its point instead of pointing somewhere the aircraft is not.
    public static func drawableHeading(
        degrees: Double?,
        reference: HeadingReference?
    ) -> Double? {
        guard let degrees, degrees.isFinite, reference == .trueNorth else { return nil }
        return wrappedBearing(degrees)
    }

    /// A geographic point, as longitude and latitude in degrees.
    public struct Point: Equatable, Sendable {
        /// Degrees east of the prime meridian.
        public let longitudeDegrees: Double
        /// Degrees north of the equator.
        public let latitudeDegrees: Double

        public init(longitudeDegrees: Double, latitudeDegrees: Double) {
            self.longitudeDegrees = longitudeDegrees
            self.latitudeDegrees = latitudeDegrees
        }
    }

    /// A bearing wrapped into `[0, 360)`.
    public static func wrappedBearing(_ degrees: Double) -> Double {
        guard degrees.isFinite else { return 0 }
        let remainder = degrees.truncatingRemainder(dividingBy: 360)
        return (remainder + 360).truncatingRemainder(dividingBy: 360)
    }

    /// Whether a course this fast may be drawn, given whether one is drawn already.
    ///
    /// The band is what stops a hovering aircraft flickering its course; which end
    /// applies depends on whether there is a course on the map to take away.
    public static func courseIsDrawable(groundSpeedKnots: Double?, alreadyDrawn: Bool) -> Bool {
        guard let speed = groundSpeedKnots, speed.isFinite, speed >= 0 else { return false }
        return speed >= (alreadyDrawn ? trackReleaseKnots : trackFloorKnots)
    }

    /// The place the aircraft reaches by holding this velocity for `seconds`.
    ///
    /// The step is along a great circle. A flat step in degrees divides by the cosine of
    /// the latitude, so near the pole it produces longitudes of thousands of degrees and
    /// latitudes past 90 — places that are not on the Earth. This form is defined
    /// everywhere, including across the pole.
    ///
    /// The longitude is deliberately not wrapped into `[-180, 180)`. A leader is one
    /// two-vertex line and a renderer projects each vertex on its own, so a pair either
    /// side of the antimeridian is drawn the long way, across the whole world, in place
    /// of a line a few kilometres long. What a renderer needs of the second vertex is
    /// that it lie within half a turn of the first, which is what `atan2` returns.
    public static func leaderEndpoint(
        from start: Point,
        bearingDegrees: Double,
        groundSpeedKnots: Double,
        seconds: Double = leaderSeconds
    ) -> Point {
        let metresPerSecond = groundSpeedKnots / knotsPerMetrePerSecond
        let angular = metresPerSecond * seconds / earthRadiusMetres
        let bearing = bearingDegrees * .pi / 180
        let latitude = start.latitudeDegrees * .pi / 180
        let longitude = start.longitudeDegrees * .pi / 180

        let sinLatitude =
            sin(latitude) * cos(angular) + cos(latitude) * sin(angular) * cos(bearing)
        // `asin` of a value a rounding error past 1 is not a number, and a coordinate
        // that is not a number takes the whole line off the map.
        let endLatitude = asin(min(1, max(-1, sinLatitude)))
        let endLongitude =
            longitude
            + atan2(
                sin(bearing) * sin(angular) * cos(latitude),
                cos(angular) - sin(latitude) * sinLatitude
            )
        return Point(
            longitudeDegrees: endLongitude * 180 / .pi,
            latitudeDegrees: endLatitude * 180 / .pi
        )
    }

    /// A bearing as it is spoken, which is not the range it is stored in.
    ///
    /// Headings are given in whole degrees from 001 to 360, and north is 360; zero is
    /// the one value the convention does not use.
    public static func spokenBearing(_ degrees: Double) -> Int {
        let rounded = Int(wrappedBearing(degrees).rounded())
        return (rounded + 359) % 360 + 1
    }
}
