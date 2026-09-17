import XCTest
@testable import Pilotage

@MainActor
final class NavigationReleaseTests: XCTestCase {
    private let now = Date(timeIntervalSince1970: 150)

    func testCurrentEditionReplacesAnExpiredSelectionAndKeepsRegionalSources() {
        let old = release("old", start: 0, end: 100)
        let current = release("current", start: 100, end: 200)
        let regional = release("foreign", start: 100, end: 200, bounds: [10, 20, 30, 40])
        let snapshot = AviationDataSnapshot(installed: [old, current, regional],
            selections: [AviationSelection(name: "active-navdata", root: old.id, pinned: true)])
        XCTAssertEqual(Set(NavigationSearchModel.selectedSources(snapshot, at: now).map(\.id)), ["current", "foreign"])
    }

    func testExpiredFallbackRemainsUntilTheDownloadedEditionBecomesEffective() {
        let old = release("old", start: 0, end: 100)
        let next = release("next", start: 200, end: 300)
        let snapshot = AviationDataSnapshot(installed: [old, next], selections: [])
        XCTAssertEqual(NavigationSearchModel.selectedSources(snapshot, at: now).map(\.id), ["old"])
        XCTAssertEqual(NavigationSearchModel.selectedSources(snapshot, at: Date(timeIntervalSince1970: 200)).map(\.id), ["next"])
        XCTAssertTrue(NavigationReleasePolicy.removable(snapshot, replacements: [next], retaining: [], at: now).isEmpty)
    }

    func testCleanupOnlyRemovesSupersededUnreferencedPackages() {
        let old = release("old", start: 0, end: 100)
        let current = release("current", start: 100, end: 200)
        let upcoming = release("future", start: 200, end: 300)
        var snapshot = AviationDataSnapshot(installed: [old, current, upcoming], selections: [])
        XCTAssertEqual(NavigationReleasePolicy.removable(snapshot, replacements: [current], retaining: [], at: now).map(\.id), ["old"])
        XCTAssertTrue(NavigationReleasePolicy.removable(snapshot, replacements: [current], retaining: ["old"], at: now).isEmpty)
        snapshot = AviationDataSnapshot(installed: snapshot.installed,
            selections: [AviationSelection(name: "mission-test", root: old.id, pinned: true)])
        XCTAssertTrue(NavigationReleasePolicy.removable(snapshot, replacements: [current], retaining: [], at: now).isEmpty)
    }

    private func release(_ id: String, start: TimeInterval, end: TimeInterval,
                         bounds: [Double] = [-180, -90, 180, 90]) -> InstalledAviationRelease {
        InstalledAviationRelease(release: AviationRelease(id: id, product: .navdata, authority: "authority",
            edition: id, revision: 1, channel: "development",
            validity: AviationValidity(effectiveAt: Date(timeIntervalSince1970: start), expiresAt: Date(timeIntervalSince1970: end)),
            coverage: AviationCoverage(name: id, bounds: bounds, minZoom: 0, maxZoom: 16, complete: true, exclusions: []),
            artifacts: [], dependencies: [], rendererCapabilities: [], attributions: []), directory: "/test/" + id)
    }
}
