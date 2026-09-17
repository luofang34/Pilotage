import XCTest
@testable import Pilotage

@MainActor
final class MissionPlannerTests: XCTestCase {
    func testDurationRejectsValuesOutsideTheDisplayRange() {
        for seconds in [Double.infinity, .nan, .greatestFiniteMagnitude, -1] {
            XCTAssertEqual(MissionPlanModel.duration(seconds), "—")
        }
        XCTAssertEqual(MissionPlanModel.duration(3660), "1+01")
        XCTAssertEqual(MissionPlanModel.duration(253_402_300_740), "70389527+59")
    }

    func testResolvedRoutePersistsOrderSourceAndCalculatedValues() async throws {
        let directory = FileManager.default.temporaryDirectory.appendingPathComponent(UUID().uuidString)
        defer { try? FileManager.default.removeItem(at: directory) }
        let url = directory.appendingPathComponent("mission.json")
        let plan = MissionPlanModel()
        await plan.start(url: url)
        plan.add(match("AAA", longitude: 0))
        plan.add(match("BBB", longitude: 1))
        plan.changeRoute { $0.groundspeedKnots = 60; $0.departureUtc = 100 }
        await plan.flush()
        XCTAssertEqual(try XCTUnwrap(plan.estimate).distanceNm, 60.04, accuracy: 0.05)
        XCTAssertEqual(plan.tokens.map(\.label), ["AAA", "BBB"])
        let reopened = MissionPlanModel()
        await reopened.start(url: url)
        XCTAssertEqual(reopened.draft, plan.draft)
        XCTAssertEqual(reopened.tokens[0].source?.releaseId, "source-1")
        reopened.tokens.reverse()
        await reopened.flush()
        XCTAssertEqual(try XCTUnwrap(reopened.estimate?.legs.last?.trackTrueDeg), 270, accuracy: 0.01)
    }

    func testInvalidEditDoesNotCorruptSavedDraft() async throws {
        let directory = FileManager.default.temporaryDirectory.appendingPathComponent(UUID().uuidString)
        defer { try? FileManager.default.removeItem(at: directory) }
        let url = directory.appendingPathComponent("mission.json")
        let plan = MissionPlanModel()
        await plan.start(url: url)
        plan.add(match("AAA", longitude: 0))
        await plan.flush()
        let before = plan.draft
        plan.changeRoute { $0.groundspeedKnots = -1 }
        XCTAssertNotNil(plan.errorMessage)
        XCTAssertEqual(plan.draft, before)
        let reopened = MissionPlanModel()
        await reopened.start(url: url)
        XCTAssertEqual(reopened.draft, before)
    }

    func testUnreadableDraftIsNotReplacedByAnEmptyDraft() async throws {
        let directory = FileManager.default.temporaryDirectory.appendingPathComponent(UUID().uuidString)
        defer { try? FileManager.default.removeItem(at: directory) }
        try FileManager.default.createDirectory(at: directory, withIntermediateDirectories: true)
        let url = directory.appendingPathComponent("mission.json")
        let bytes = Data("invalid".utf8)
        try bytes.write(to: url)
        let plan = MissionPlanModel()
        await plan.start(url: url)
        plan.add(match("AAA", longitude: 0))
        await plan.flush()
        XCTAssertNotNil(plan.errorMessage)
        XCTAssertEqual(try Data(contentsOf: url), bytes)
    }

    func testAssignmentsKeepSeparateRoutesAndSwarmTimingSurvivesRestore() async throws {
        let directory = FileManager.default.temporaryDirectory.appendingPathComponent(UUID().uuidString)
        defer { try? FileManager.default.removeItem(at: directory) }
        let url = directory.appendingPathComponent("mission.json")
        let plan = MissionPlanModel()
        await plan.start(url: url)
        plan.add(match("AAA", longitude: 0))
        plan.add(match("BBB", longitude: 1))
        let first = try XCTUnwrap(plan.assignment)
        plan.change { draft in
            draft.participants.append(MissionParticipant(id: "other", name: "Other owner", organization: "Survey team"))
            draft.assignments.append(MissionAssignment(id: "second", name: "Vehicle 2", participantId: "other", route: PlannedRoute()))
        }
        plan.selectedAssignmentId = "second"
        XCTAssertTrue(plan.tokens.isEmpty)
        plan.add(match("CCC", longitude: 2))
        let secondWaypoint = try XCTUnwrap(plan.tokens.first).id
        plan.change { draft in
            draft.swarms.append(MissionSwarm(id: "swarm", name: "Survey", assignmentIds: [first.id, "second"]))
            draft.timingTargets.append(MissionTimingTarget(id: "target", name: "Arrive", utc: 5000,
                toleranceSeconds: 30, swarmId: "swarm", members: [
                    MissionTimingMember(assignmentId: first.id, waypointId: first.route.waypoints[1].id),
                    MissionTimingMember(assignmentId: "second", waypointId: secondWaypoint, offsetSeconds: 60)]))
        }
        await plan.flush()
        XCTAssertEqual(plan.assessment?.timings.count, 2)
        XCTAssertTrue(plan.assessment?.timings.allSatisfy { $0.status == "unknown" } == true)
        plan.tokens.removeAll()
        XCTAssertEqual(plan.tokens.count, 1)
        XCTAssertNotNil(plan.errorMessage)
        let restored = MissionPlanModel()
        await restored.start(url: url)
        XCTAssertEqual(restored.draft, plan.draft)
        XCTAssertEqual(restored.draft.assignments[0].route.waypoints.count, 2)
    }

    private func match(_ identifier: String, longitude: Double) -> NavigationMatch {
        NavigationMatch(point: NavigationPoint(key: identifier, identifier: identifier,
            kind: "airport", name: "Test airport", region: "TEST", latitudeDeg: 0, longitudeDeg: longitude),
            source: NavigationSource(releaseId: "source-1", authority: "test", edition: "1",
                sourceDigest: String(repeating: "a", count: 64), effectiveAt: 0, expiresAt: 100_000))
    }
}
