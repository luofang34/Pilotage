import Foundation

/// Roll and pitch from one attitude source.
public struct AircraftAttitude: Equatable, Sendable {
    /// Right-wing-down roll in degrees.
    public let rollDegrees: Double
    /// Nose-up pitch in degrees.
    public let pitchDegrees: Double

    /// Make one attitude value.
    public init(rollDegrees: Double, pitchDegrees: Double) {
        self.rollDegrees = rollDegrees
        self.pitchDegrees = pitchDegrees
    }
}

/// A local zero-pitch and zero-bank correction.
public struct AttitudeCage: Equatable, Sendable {
    /// Roll captured when the aircraft was level.
    public private(set) var rollOffsetDegrees: Double?
    /// Pitch captured when the aircraft was level.
    public private(set) var pitchOffsetDegrees: Double?

    /// Make an empty or restored correction.
    public init(
        rollOffsetDegrees: Double? = nil,
        pitchOffsetDegrees: Double? = nil
    ) {
        self.rollOffsetDegrees = rollOffsetDegrees
        self.pitchOffsetDegrees = pitchOffsetDegrees
    }

    /// Capture the current pitch and bank as level.
    @discardableResult
    public mutating func recenter(on attitude: AircraftAttitude) -> Bool {
        guard attitude.rollDegrees.isFinite, attitude.pitchDegrees.isFinite else {
            return false
        }
        rollOffsetDegrees = attitude.rollDegrees
        pitchOffsetDegrees = attitude.pitchDegrees
        return true
    }

    /// Remove the level correction.
    public mutating func clear() {
        rollOffsetDegrees = nil
        pitchOffsetDegrees = nil
    }

    /// Apply the level correction without changing heading.
    public func apply(to attitude: AircraftAttitude) -> AircraftAttitude {
        let roll = Self.normalizedRoll(
            attitude.rollDegrees - (rollOffsetDegrees ?? 0)
        )
        let pitch = attitude.pitchDegrees - (pitchOffsetDegrees ?? 0)
        return AircraftAttitude(
            rollDegrees: roll,
            pitchDegrees: min(max(pitch, -90), 90)
        )
    }

    private static func normalizedRoll(_ value: Double) -> Double {
        let turn = value.truncatingRemainder(dividingBy: 360)
        if turn >= 180 { return turn - 360 }
        if turn < -180 { return turn + 360 }
        return turn
    }
}
