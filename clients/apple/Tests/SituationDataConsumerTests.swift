import Foundation
import PilotageCore
import XCTest
@testable import Pilotage

@MainActor
final class SituationDataConsumerTests: XCTestCase {
    func testInstalledDataKeepsTheSessionAndItsLayerChoices() async throws {
        let data = try exampleData()
        let consumer = SituationDataConsumer()
        _ = try await consumer.load(data)
        _ = try consumer.session.setLayerEnabled(layerId: "weather-reports", enabled: false)
        let next = try await consumer.load(data)
        XCTAssertFalse(try XCTUnwrap(next.layers.first { $0.id == "weather-reports" }).enabled)
        XCTAssertEqual(consumer.terrainArchivePath, data.terrainURL.path)
    }

    func testUnreadableNavigationCannotReplaceTheLoadedCycle() async throws {
        let data = try exampleData()
        let consumer = SituationDataConsumer()
        _ = try await consumer.load(data)
        let invalid = AviationSituationData(terrainID: data.terrainID, terrainURL: data.terrainURL,
                                            navigationID: "invalid-cycle", navigationCycle: Data("invalid".utf8))
        do {
            _ = try await consumer.load(invalid)
            XCTFail("An unreadable navigation cycle must fail.")
        } catch {
            XCTAssertEqual(consumer.installed?.navigationID, data.navigationID)
        }
        _ = try await consumer.load(data)
    }

    private func exampleData() throws -> AviationSituationData {
        let examples = try XCTUnwrap(Bundle.main.url(forResource: "AviationDataExamples", withExtension: nil))
        let manifest = examples.appendingPathComponent("terrain-existing/release.json")
        let release = try decodeAviationData(AviationRelease.self, from: String(contentsOf: manifest, encoding: .utf8))
        let terrain = InstalledAviationRelease(release: release, directory: manifest.deletingLastPathComponent().path)
        let navigation = examples.appendingPathComponent("navdata-existing-2026-06-11/navigation.acnav")
        return AviationSituationData(terrainID: terrain.id, terrainURL: try XCTUnwrap(terrain.artifactURL(format: "mbtiles")),
                                     navigationID: "navdata-existing-2026-06-11", navigationCycle: try Data(contentsOf: navigation))
    }
}
