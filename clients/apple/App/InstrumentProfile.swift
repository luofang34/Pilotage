import Foundation

/// One tile in an instrument rack, in stack order.
enum InstrumentTile: Codable, Hashable, Identifiable {
    /// Live video from the vehicle, by source name.
    case video(source: String)
    /// One registry panel, by its stable descriptor id.
    case panel(id: String)

    /// Stable identity for presentation APIs.
    var id: String {
        switch self {
        case .video(let source): "video-\(source)"
        case .panel(let id): "panel-\(id)"
        }
    }
}

/// One named composition of the rack: what the operator sees beside the
/// map, top to bottom.
///
/// Profiles are data, not code: a future release can let the operator
/// edit and add them, and nothing here would change shape. The tiles
/// resolve against the registry at paint time, so a profile naming a
/// panel this build does not carry shows a typed reason instead of a
/// blank.
struct InstrumentProfile: Codable, Identifiable, Hashable {
    /// Stable identity for selection persistence.
    let id: String
    /// Operator-facing name.
    let name: String
    /// The rack's tiles, top to bottom.
    let tiles: [InstrumentTile]

    /// The built-in profiles, until operator-defined ones arrive.
    static let builtIn: [InstrumentProfile] = [
        InstrumentProfile(
            id: "px4-flight",
            name: "PX4 flight",
            tiles: [
                .video(source: "gimbal"),
                .panel(id: "pfd"),
                .panel(id: "hsi"),
            ]
        ),
        InstrumentProfile(
            id: "flight",
            name: "Flight",
            tiles: [
                .panel(id: "pfd"),
                .panel(id: "hsi"),
            ]
        ),
        InstrumentProfile(
            id: "primary",
            name: "Primary only",
            tiles: [
                .panel(id: "pfd")
            ]
        ),
    ]

    /// The profile for a stored selection, or the first built-in.
    static func selected(storedId: String) -> InstrumentProfile {
        builtIn.first { $0.id == storedId } ?? builtIn[0]
    }
}
