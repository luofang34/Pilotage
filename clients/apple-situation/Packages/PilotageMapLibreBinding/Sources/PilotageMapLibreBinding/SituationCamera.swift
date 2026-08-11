import Foundation

/// Where the camera is pointing.
///
/// Heading and tilt are reported apart from the renderer so the controls that undo them do
/// not have to import it. A reader who has turned or tilted the map needs one way back to
/// north and one way back to straight down, and needs to see that there is something to
/// undo.
public struct SituationCamera: Equatable, Sendable {
    /// Clockwise rotation of the map away from north, in degrees.
    public let headingDegrees: Double
    /// Angle away from straight down, in degrees.
    public let pitchDegrees: Double

    /// Create a camera reading.
    public init(headingDegrees: Double, pitchDegrees: Double) {
        self.headingDegrees = headingDegrees
        self.pitchDegrees = pitchDegrees
    }

    /// Whether the map is turned far enough off north for a reader to notice.
    ///
    /// A fraction of a degree is a rounding artefact of a pinch, not a decision, and a
    /// control that appears for one would flicker during an ordinary zoom.
    public var isRotated: Bool { headingDegrees.magnitude > 0.5 && headingDegrees.magnitude < 359.5 }

    /// Whether the map is tilted away from straight down.
    public var isTilted: Bool { pitchDegrees > 0.5 }
}
