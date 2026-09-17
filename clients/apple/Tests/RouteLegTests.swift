import UIKit
import XCTest
@testable import Pilotage

@MainActor
final class RouteLegTests: XCTestCase {
    func testZoomedLegCrossesTheViewportWhenBothEndsAreOffscreen() {
        let view = GlobeSituationOverlay()
        view.frame = CGRect(x: 0, y: 0, width: 200, height: 200)
        view.plannedRoutes = [PlannedMapRoute(id: "route", name: "Route", tokens: [token("a", -1), token("b", 1)], selected: true)]
        view.project = { coordinates in stride(from: 0, to: coordinates.count, by: 3).map {
            CGPoint(x: 100 + coordinates[$0 + 1] * 10000, y: 100)
        } }
        let pixels = render(view)
        XCTAssertGreaterThan(pixels[(100 * 200 + 100) * 4 + 3], 0)
        view.project = { coordinates in Array(repeating: nil, count: coordinates.count / 3) }
        XCTAssertTrue(render(view).allSatisfy { $0 == 0 })
    }

    func testAntimeridianPartsDoNotDrawALineAcrossTheOtherSideOfTheMap() {
        let view = GlobeSituationOverlay()
        view.frame = CGRect(x: 0, y: 0, width: 400, height: 200)
        view.plannedRoutes = [PlannedMapRoute(id: "route", name: "Route", tokens: [token("a", 179), token("b", -179)], selected: true)]
        view.project = { coordinates in stride(from: 0, to: coordinates.count, by: 3).map {
            CGPoint(x: 200 + coordinates[$0 + 1], y: 100)
        } }
        let pixels = render(view)
        XCTAssertEqual(pixels[(100 * 400 + 200) * 4 + 3], 0)
        XCTAssertGreaterThan(pixels[(100 * 400 + 379) * 4 + 3], 0)
    }

    func testNavigationProgressSeparatesPlannedCurrentAndPastLegsAndClearsOnEdit() async throws {
        let directory = FileManager.default.temporaryDirectory.appendingPathComponent(UUID().uuidString)
        defer { try? FileManager.default.removeItem(at: directory) }
        let plan = MissionPlanModel()
        await plan.start(url: directory.appendingPathComponent("mission.json"))
        plan.tokens = [token("a", 0), token("b", 1), token("c", 2)]
        let id = try XCTUnwrap(plan.assignment?.id)
        XCTAssertEqual(plan.legRole(destinationIndex: 1, assignmentID: id), .planned)
        plan.setCurrentLeg(to: "b")
        XCTAssertEqual(plan.legRole(destinationIndex: 1, assignmentID: id), .current)
        plan.receiveNavigationFix(OwnshipFix(latitudeDegrees: 0, longitudeDegrees: 0.5, courseDegrees: 90, source: .device))
        XCTAssertEqual(plan.legRole(destinationIndex: 1, assignmentID: id), .current)
        plan.receiveNavigationFix(OwnshipFix(latitudeDegrees: 0, longitudeDegrees: 1, courseDegrees: 90, source: .device))
        XCTAssertEqual(plan.legRole(destinationIndex: 1, assignmentID: id), .past)
        XCTAssertEqual(plan.legRole(destinationIndex: 2, assignmentID: id), .current)
        XCTAssertEqual(plan.legRole(destinationIndex: 1, assignmentID: "another"), .planned)
        plan.tokens.reverse()
        XCTAssertNil(plan.navigationProgress)
        await plan.flush()
    }

    private func token(_ id: String, _ longitude: Double) -> RouteToken {
        RouteToken(id: id, point: NavigationPoint(key: id, identifier: id, kind: "waypoint", name: "", region: "",
            latitudeDeg: 0, longitudeDeg: longitude), source: nil)
    }

    private func render(_ view: GlobeSituationOverlay) -> [UInt8] {
        let width = Int(view.bounds.width), height = Int(view.bounds.height)
        var bytes = [UInt8](repeating: 0, count: width * height * 4)
        bytes.withUnsafeMutableBytes { buffer in
            guard let context = CGContext(data: buffer.baseAddress, width: width, height: height,
                bitsPerComponent: 8, bytesPerRow: width * 4, space: CGColorSpaceCreateDeviceRGB(),
                bitmapInfo: CGImageAlphaInfo.premultipliedLast.rawValue) else { XCTFail("Bitmap context"); return }
            UIGraphicsPushContext(context)
            view.draw(view.bounds)
            UIGraphicsPopContext()
        }
        return bytes
    }
}
