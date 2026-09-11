import Foundation
import XCTest
@testable import AviationDataRecords

final class AviationProcedureCatalogTests: XCTestCase {
    private func installed(path: String = "chart.PDF", directory: String = "/installed/procedures") -> InstalledAviationRelease {
        InstalledAviationRelease(release: AviationRelease(
            id: "procedures-2609", product: .procedures, authority: "FAA", edition: "2609", revision: 1,
            channel: "development", validity: nil,
            coverage: AviationCoverage(name: "KTTN", bounds: [-180, -90, 180, 90], minZoom: 0, maxZoom: 0,
                                       complete: false, exclusions: []),
            artifacts: [AviationArtifact(path: path, format: "pdf", bytes: 1, sha256: "test")],
            dependencies: [], rendererCapabilities: ["procedure-pdf-v1"], attributions: []
        ), directory: directory)
    }

    private func index(edition: String = "2609", paths: [String] = ["chart.PDF"]) throws -> Data {
        try JSONSerialization.data(withJSONObject: [
            "schema_version": 1, "edition": edition,
            "charts": paths.map { ["id": "KTTN-chart", "airport": "KTTN", "name": "ILS RWY 06",
                                   "chart_code": "IAP", "artifact": $0] },
        ])
    }

    func testChartUsesTheInstalledPdfAndItsEdition() throws {
        let catalog = try AviationProcedureCatalog.decode(index(), installed: installed())
        let chart = try XCTUnwrap(catalog.charts.first)
        XCTAssertEqual(try catalog.url(for: chart).path, "/installed/procedures/chart.PDF")
        XCTAssertEqual(chart.airport, "KTTN")
        XCTAssertEqual(chart.name, "ILS RWY 06")
    }

    func testMismatchedEditionAndDuplicateChartIdentityAreRejected() throws {
        XCTAssertThrowsError(try AviationProcedureCatalog.decode(index(edition: "2608"), installed: installed()))
        XCTAssertThrowsError(try AviationProcedureCatalog.decode(index(paths: ["chart.PDF", "chart.PDF"]), installed: installed()))
    }

    func testUndeclaredAndEscapingPdfsAreRejected() throws {
        XCTAssertThrowsError(try AviationProcedureCatalog.decode(index(paths: ["missing.PDF"]), installed: installed()))
        for path in ["../chart.PDF", "/private/chart.PDF", "a/./chart.PDF", "a//chart.PDF"] {
            XCTAssertThrowsError(try AviationProcedureCatalog.decode(index(paths: [path]), installed: installed(path: path)))
        }
    }

    func testSymlinkCannotOpenAPdfOutsideThePackage() throws {
        let root = FileManager.default.temporaryDirectory.appendingPathComponent(UUID().uuidString)
        let outside = FileManager.default.temporaryDirectory.appendingPathComponent(UUID().uuidString + ".PDF")
        try FileManager.default.createDirectory(at: root, withIntermediateDirectories: true)
        try Data("%PDF-1.7".utf8).write(to: outside)
        defer {
            try? FileManager.default.removeItem(at: root)
            try? FileManager.default.removeItem(at: outside)
        }
        try FileManager.default.createSymbolicLink(at: root.appendingPathComponent("chart.PDF"),
                                                   withDestinationURL: outside)
        XCTAssertThrowsError(try AviationProcedureCatalog.decode(index(), installed: installed(directory: root.path)))
    }
}
