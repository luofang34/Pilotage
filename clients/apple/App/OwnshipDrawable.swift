import Foundation
import PilotageCore

/// Where the vehicle says it is, as the link resolved it.
///
/// Metres per second here, knots at the display edge: the wire states SI and
/// the conversion belongs where the reader is, not spread through the middle.
struct VehicleFix: Equatable, Sendable {
    let latitudeDegrees: Double
    let longitudeDegrees: Double
    /// Where the nose points, true, when the lane states an attitude.
    let headingDegrees: Double?
    /// Track over the ground, when the vehicle is moving fast enough for one.
    let courseDegrees: Double?
    /// Speed over the ground alongside `courseDegrees`.
    let groundSpeedMetresPerSecond: Double?
    /// Whether a simulator's oracle supplied it rather than the estimator.
    let fromSimulator: Bool
}

extension VehicleFix {
    /// Reads one link event, or nothing when it is not a vehicle fix.
    ///
    /// The event states six loose values; naming them is this type's job, not
    /// the link's, so the link's switch stays a list of what arrived rather
    /// than a place where meaning is assigned.
    init?(_ event: LinkEvent) {
        guard case let .vehicleFix(
            latitudeDeg,
            longitudeDeg,
            headingDeg,
            courseDeg,
            groundSpeedMps,
            fromSimulator
        ) = event else { return nil }
        self.init(
            latitudeDegrees: latitudeDeg,
            longitudeDegrees: longitudeDeg,
            headingDegrees: headingDeg,
            courseDegrees: courseDeg,
            groundSpeedMetresPerSecond: groundSpeedMps,
            fromSimulator: fromSimulator
        )
    }
}

extension OwnshipModel {
    /// The resolved fix as the map draws it, or nothing when no source has one.
    ///
    /// Ground speed rides with the course and is converted at this edge, because the
    /// display states knots and every lane below states metres per second.
    ///
    /// The mark is turned only by a heading from the SAME source as its
    /// position, and only by an actual heading.
    ///
    /// `heading` is the map's answer — what to rotate the chart by — and it
    /// falls back through the tablet's compass to course over the ground so
    /// that a north-up map can still turn. Neither belongs on the mark. A
    /// device compass reading turns a mark that is sitting at the VEHICLE,
    /// pointing it wherever the reader happens to be holding the tablet; and
    /// course over the ground is where the vehicle is going, not where it is
    /// facing, so a crabbing aircraft would be drawn pointing along its track.
    /// Both are the substitution the one-lane rule exists to stop: a position
    /// from one measurement turned by another, with nothing on the mark to
    /// say so.
    ///
    /// A heading that is withheld leaves the mark pointless, which is the
    /// honest shape for "here, facing unstated".
    func drawable(groundSpeedMetresPerSecond: Double?) -> DisplayOwnship? {
        guard let fix else { return nil }
        let markHeading = headingForMark(of: fix)
        return DisplayOwnship(
            coordinate: DisplayCoordinate(
                latitudeDeg: fix.latitudeDegrees,
                longitudeDeg: fix.longitudeDegrees
            ),
            courseDeg: fix.courseDegrees,
            groundSpeedKt: groundSpeedMetresPerSecond.map { $0 * 1.943_844_49 },
            headingDeg: markHeading.map(\.trueDegrees),
            headingReference: markHeading.map {
                switch $0.source {
                case .deviceMagnetic: DisplayHeadingReference.magneticNorth
                default: DisplayHeadingReference.trueNorth
                }
            },
            altitudeFt: nil,
            producerInstanceId: 0,
            snapshotRevision: 0
        )
    }
}

extension OwnshipModel {
    /// The heading that belongs to this fix, or nothing.
    ///
    /// An aircraft position may be turned only by an aircraft heading; this
    /// device's position may be turned only by this device's compass. Course
    /// over the ground turns nothing: it is a direction of travel, not an
    /// attitude, and the source enum says so.
    fileprivate func headingForMark(of fix: OwnshipFix) -> HeadingFix? {
        guard let heading else { return nil }
        switch (fix.source, heading.source) {
        case (.aircraft, .aircraft): return heading
        case (.device, .deviceTrue), (.device, .deviceMagnetic): return heading
        default: return nil
        }
    }
}
