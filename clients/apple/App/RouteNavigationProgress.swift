import Foundation
import CoreLocation

struct RouteNavigationProgress: Equatable {
    let assignmentID: String
    let waypoints: [RouteToken]
    var destinationIndex: Int

    var completed: Bool { destinationIndex >= waypoints.count }
    var destination: RouteToken? { completed ? nil : waypoints[destinationIndex] }

    mutating func receive(_ fix: OwnshipFix) {
        guard let destination else { return }
        let position = CLLocation(latitude: fix.latitudeDegrees, longitude: fix.longitudeDegrees)
        let target = CLLocation(latitude: destination.point.latitudeDeg, longitude: destination.point.longitudeDeg)
        if position.distance(from: target) <= 185.2 { destinationIndex &+= 1 }
    }
}

enum RouteLegRole: String {
    case planned, current, past
    var label: String { rawValue.capitalized }
}

extension MissionPlanModel {
    var currentProgress: RouteNavigationProgress? {
        navigationProgress?.assignmentID == assignment?.id ? navigationProgress : nil
    }

    func setCurrentLeg(to waypointID: String) {
        guard let assignment, let index = tokens.firstIndex(where: { $0.id == waypointID }), index > 0 else { return }
        navigationProgress = RouteNavigationProgress(assignmentID: assignment.id, waypoints: tokens, destinationIndex: index)
    }

    func advanceCurrentLeg() {
        guard let currentProgress, !currentProgress.completed else { return }
        navigationProgress?.destinationIndex &+= 1
    }

    func receiveNavigationFix(_ fix: OwnshipFix?) {
        guard let fix else { return }
        navigationProgress?.receive(fix)
    }

    func legRole(destinationIndex: Int, assignmentID: String) -> RouteLegRole {
        guard let progress = navigationProgress, progress.assignmentID == assignmentID else { return .planned }
        if destinationIndex < progress.destinationIndex { return .past }
        return destinationIndex == progress.destinationIndex ? .current : .planned
    }
}
