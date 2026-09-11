import Foundation
import XCTest
@testable import AviationDataRecords

final class AviationDataTests: XCTestCase {
    private func snapshot(validity: String = "{\"effective_at\":100,\"expires_at\":200}") throws -> AviationDataSnapshot {
        try decodeAviationData(AviationDataSnapshot.self, from: """
        {
          "installed": [{"directory":"/data/releases/nav-2609", "release": {
            "id":"nav-2609", "product":"navdata", "authority":"FAA", "edition":"2609",
            "revision":1, "channel":"development", "validity":\(validity),
            "coverage":{"name":"United States", "bounds":[-180,-90,180,90],
              "min_zoom":0,"max_zoom":14,"complete":false,"exclusions":["Test sample"]},
            "artifacts":[{"path":"nav.acnav","format":"acnav","bytes":12,"sha256":"test"}],
            "dependencies":[],"renderer_capabilities":[],"attributions":["FAA"]
          }}],
          "selections":[{"name":"active-navdata","root":"nav-2609","pinned":true}]
        }
        """)
    }

    func testValidityUsesTheExactHalfOpenUtcInterval() throws {
        let release = try XCTUnwrap(snapshot().installed.first?.release)
        XCTAssertEqual(release.validityLabel(at: Date(timeIntervalSince1970: 99)), "Upcoming")
        XCTAssertEqual(release.validityLabel(at: Date(timeIntervalSince1970: 100)), "Current")
        XCTAssertEqual(release.validityLabel(at: Date(timeIntervalSince1970: 199)), "Current")
        XCTAssertEqual(release.validityLabel(at: Date(timeIntervalSince1970: 200)), "Expired")
    }

    func testExpiredSelectionRemainsAddressableForOfflineUse() throws {
        let snapshot = try snapshot()
        let active = try XCTUnwrap(snapshot.active(.navdata))
        XCTAssertEqual(active.release.validityLabel(at: Date(timeIntervalSince1970: 300)), "Expired")
        XCTAssertEqual(active.artifactURL(format: "acnav")?.path, "/data/releases/nav-2609/nav.acnav")
        XCTAssertNil(active.artifactURL(format: "pmtiles"))
        XCTAssertNil(snapshot.active(.ifrHigh))
    }

    func testMissingExpiryIsVisibleWithoutAnInventedDate() throws {
        let release = try XCTUnwrap(snapshot(validity: "null").installed.first?.release)
        XCTAssertEqual(release.validityLabel(), "No scheduled expiry")
        XCTAssertNil(release.validity)
    }
}
