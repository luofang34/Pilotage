import Foundation

struct MissionParticipant: Codable, Equatable, Identifiable, Sendable {
    var id: String
    var name: String
    var organization: String?
}

struct MissionAssignment: Codable, Equatable, Identifiable, Sendable {
    var id: String
    var name: String
    var participantId: String
    var vehicleReference: String?
    var route: PlannedRoute
}

struct MissionSwarm: Codable, Equatable, Identifiable, Sendable {
    var id: String
    var name: String
    var assignmentIds: [String]
}

struct MissionTimingMember: Codable, Equatable, Sendable {
    var assignmentId: String
    var waypointId: String
    var offsetSeconds: Int32 = 0
}

struct MissionTimingTarget: Codable, Equatable, Identifiable, Sendable {
    var id: String
    var name: String
    var utc: Int64
    var toleranceSeconds: UInt32
    var swarmId: String?
    var members: [MissionTimingMember]
}

struct MissionTimingAssessment: Decodable, Identifiable {
    let targetId: String
    let assignmentId: String
    let waypointId: String
    let requiredUtc: Double
    let estimatedUtc: Double?
    let deviationSeconds: Double?
    let earliestDepartureUtc: Double?
    let latestDepartureUtc: Double?
    let status: String
    var id: String { targetId + ":" + assignmentId }
    var statusLabel: String {
        switch status {
        case "on_time": "Within target window"
        case "early": "Early"
        case "late": "Late"
        default: "Time unknown"
        }
    }
}

struct CoordinatedMissionAssessment: Decodable {
    let routes: [String: PlannedRouteSummary]
    let timings: [MissionTimingAssessment]
    let issues: [String]
}

struct MissionDraft: Codable, Equatable, Sendable {
    var schemaVersion = 1
    var id = UUID().uuidString
    var revision: UInt64 = 0
    var title = "Untitled mission"
    var participants = [MissionParticipant(id: "local", name: "Me")]
    var assignments = [MissionAssignment(id: UUID().uuidString, name: "Vehicle 1", participantId: "local", route: PlannedRoute())]
    var swarms: [MissionSwarm] = []
    var timingTargets: [MissionTimingTarget] = []
}
