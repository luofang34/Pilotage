import XCTest

@testable import PilotageCore

/// The iPad and the web client draw the same two directions beside the same aircraft.
///
/// Each has its own implementation, in its own language, and nothing in either would
/// notice the other drifting. Both are held to one corpus of cases instead: a reader
/// comparing the two displays is entitled to see the same course drawn on each, and a
/// difference between them is a defect in whichever one moved.
final class OwnshipMotionConformanceTests: XCTestCase {
    private struct Corpus: Decodable {
        let leaderSeconds: Double
        let trackFloorMps: Double
        let trackReleaseMps: Double
        let leaders: [Leader]
        let bearings: [Bearing]
    }

    private struct Leader: Decodable {
        let name: String
        let latitudeDeg: Double
        let longitudeDeg: Double
        let bearingDeg: Double
        let groundSpeedMps: Double
        let seconds: Double
        let endLongitudeDeg: Double
        let endLatitudeDeg: Double
    }

    private struct Bearing: Decodable {
        let deg: Double
        let wrapped: Double
    }

    /// One metre per second in knots, the same constant the implementation uses, so a
    /// corpus stated in metres per second reaches an interface stated in knots without
    /// inventing a second conversion.
    private static let knotsPerMetrePerSecond: Double = 1.943_844_49

    private func corpus() throws -> Corpus {
        // The corpus sits beside the two clients rather than inside either, so neither
        // owns it and neither can quietly edit its own copy.
        let root = URL(fileURLWithPath: #filePath)
            .deletingLastPathComponent()  // PilotageCoreTests
            .deletingLastPathComponent()  // Tests
            .deletingLastPathComponent()  // PilotageCore
            .deletingLastPathComponent()  // Packages
            .deletingLastPathComponent()  // apple
        let url = root.appendingPathComponent("situation-ownship-motion.corpus.json")
        return try JSONDecoder().decode(Corpus.self, from: Data(contentsOf: url))
    }

    func testTheLeaderReachesTheSamePlaceAsTheWebClient() throws {
        let corpus = try corpus()
        XCTAssertEqual(corpus.leaderSeconds, OwnshipMotion.leaderSeconds, accuracy: 1e-9)

        for leader in corpus.leaders {
            let start = OwnshipMotion.Point(
                longitudeDegrees: leader.longitudeDeg,
                latitudeDegrees: leader.latitudeDeg
            )
            let end = OwnshipMotion.leaderEndpoint(
                from: start,
                bearingDegrees: leader.bearingDeg,
                groundSpeedKnots: leader.groundSpeedMps * Self.knotsPerMetrePerSecond,
                seconds: leader.seconds
            )
            // A tenth of a microdegree is about a centimetre of ground: far below the
            // width of the line, and far above the difference two languages' trigonometry
            // makes on the same formula.
            XCTAssertEqual(
                end.longitudeDegrees, leader.endLongitudeDeg, accuracy: 1e-7,
                "longitude for \(leader.name)"
            )
            XCTAssertEqual(
                end.latitudeDegrees, leader.endLatitudeDeg, accuracy: 1e-7,
                "latitude for \(leader.name)"
            )
        }
    }

    func testEveryLeaderEndsSomewhereOnTheEarth() throws {
        // A step in degrees divides by the cosine of the latitude and names latitudes
        // past 90 beside the pole. The corpus carries those cases; this is the property
        // that has to hold for every one of them.
        for leader in try corpus().leaders {
            let start = OwnshipMotion.Point(
                longitudeDegrees: leader.longitudeDeg,
                latitudeDegrees: leader.latitudeDeg
            )
            let end = OwnshipMotion.leaderEndpoint(
                from: start,
                bearingDegrees: leader.bearingDeg,
                groundSpeedKnots: leader.groundSpeedMps * Self.knotsPerMetrePerSecond
            )
            XCTAssertTrue(end.latitudeDegrees.isFinite, "\(leader.name) ends at a place")
            XCTAssertLessThanOrEqual(abs(end.latitudeDegrees), 90, "\(leader.name) is on the Earth")
            // The invariant a two-vertex line needs: the second vertex within half a turn
            // of the first, or a renderer draws the segment the long way around.
            XCTAssertLessThanOrEqual(
                abs(end.longitudeDegrees - leader.longitudeDeg), 180,
                "\(leader.name) spans no more than half a turn"
            )
        }
    }

    func testBearingsWrapTheSameWayOnBothClients() throws {
        for bearing in try corpus().bearings {
            XCTAssertEqual(
                OwnshipMotion.wrappedBearing(bearing.deg), bearing.wrapped, accuracy: 1e-9,
                "wrapping \(bearing.deg)"
            )
        }
    }

    func testTheCourseBandMatchesTheWebClient() throws {
        let corpus = try corpus()
        let floor = corpus.trackFloorMps * Self.knotsPerMetrePerSecond
        let release = corpus.trackReleaseMps * Self.knotsPerMetrePerSecond
        XCTAssertEqual(OwnshipMotion.trackFloorKnots, floor, accuracy: 1e-6)
        XCTAssertEqual(OwnshipMotion.trackReleaseKnots, release, accuracy: 1e-6)

        // The band is what stops a hovering aircraft flickering its course on and off at
        // the report rate: between the two speeds, a course already drawn stays and a
        // course not yet drawn does not start.
        let between = (floor + release) / 2
        XCTAssertFalse(
            OwnshipMotion.courseIsDrawable(groundSpeedKnots: between, alreadyDrawn: false))
        XCTAssertTrue(
            OwnshipMotion.courseIsDrawable(groundSpeedKnots: between, alreadyDrawn: true))
        XCTAssertFalse(
            OwnshipMotion.courseIsDrawable(groundSpeedKnots: release - 0.01, alreadyDrawn: true))

        // A speed nobody reported is not a slow one.
        XCTAssertFalse(OwnshipMotion.courseIsDrawable(groundSpeedKnots: nil, alreadyDrawn: true))
        XCTAssertFalse(
            OwnshipMotion.courseIsDrawable(groundSpeedKnots: .nan, alreadyDrawn: false))
    }

    func testASpokenBearingRunsFromOneTo360() {
        // Spoken bearings run 001 to 360 and north is 360; zero is the one value the
        // convention does not use. The web client announces the same way.
        XCTAssertEqual(OwnshipMotion.spokenBearing(0), 360)
        XCTAssertEqual(OwnshipMotion.spokenBearing(359.97), 360)
        XCTAssertEqual(OwnshipMotion.spokenBearing(1), 1)
        XCTAssertEqual(OwnshipMotion.spokenBearing(90), 90)
        XCTAssertEqual(OwnshipMotion.spokenBearing(359.4), 359)
    }
}
