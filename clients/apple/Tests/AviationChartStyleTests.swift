import Foundation
import XCTest
@testable import AviationDataRecords

final class AviationChartStyleTests: XCTestCase {
    private func installed(path: String = "network.pmtiles", directory: String = "/installed/release",
                           product: AviationProduct = .ifrLow) -> InstalledAviationRelease {
        let artifacts = [(path, "pmtiles"), ("symbols.png", "resource"), ("symbols.json", "resource")].map {
            AviationArtifact(path: $0.0, format: $0.1, bytes: 1, sha256: "test")
        }
        return InstalledAviationRelease(release: AviationRelease(
            id: product.rawValue + "-2609", product: product, authority: "FAA", edition: "2609", revision: 1,
            channel: "development", validity: nil,
            coverage: AviationCoverage(name: "Test", bounds: [-180, -85, 180, 85], minZoom: 3,
                                       maxZoom: 8, complete: false, exclusions: ["Test"]),
            artifacts: artifacts, dependencies: [], rendererCapabilities: ["ifr-chart-v1"], attributions: []
        ), directory: directory)
    }

    private func style(path: String = "network.pmtiles", tiles: String = "pilotage://network/{z}/{x}/{y}") -> [String: Any] {
        ["version": 8, "sprite": "pilotage://symbols", "layers": [],
         "sources": ["network": ["type": "vector", "tiles": [tiles]]],
         "metadata": ["edition": "producer-style-date",
                      "pilotage:resources": [["uri": "pilotage://network", "path": "/foreign/private", "format": "pmtiles"]],
                      "pilotage:resource-artifacts": [
                        ["uri": "pilotage://network", "path": path, "format": "pmtiles"],
                        ["uri": "pilotage://symbols.png", "path": "symbols.png", "format": "file"],
                        ["uri": "pilotage://symbols.json", "path": "symbols.json", "format": "file"],
                      ]]]
    }

    func testResolvedStyleUsesStorePathsAndPreservesProducerProvenance() throws {
        let chart = try AviationChartStyle.resolve(JSONSerialization.data(withJSONObject: style()), installed: installed())
        let resolved = try XCTUnwrap(JSONSerialization.jsonObject(with: Data(chart.json.utf8)) as? [String: Any])
        let metadata = try XCTUnwrap(resolved["metadata"] as? [String: Any])
        let bindings = try XCTUnwrap(metadata["pilotage:resources"] as? [[String: String]])
        XCTAssertEqual(bindings.first?["path"], "/installed/release/network.pmtiles")
        XCTAssertFalse(chart.json.contains("/foreign/private"))
        XCTAssertEqual(metadata["edition"] as? String, "producer-style-date")
    }

    func testUnlistedOrTraversingArtifactsCannotBecomeRendererResources() throws {
        for path in ["missing.pmtiles", "../private", "/private", "a//private", "a/./private"] {
            let release = path == "missing.pmtiles" ? installed() : installed(path: path)
            XCTAssertThrowsError(try AviationChartStyle.resolve(
                JSONSerialization.data(withJSONObject: style(path: path)), installed: release
            ))
        }
    }

    func testSymlinkCannotEscapeAnInstalledRelease() throws {
        let root = FileManager.default.temporaryDirectory.appendingPathComponent(UUID().uuidString)
        try FileManager.default.createDirectory(at: root, withIntermediateDirectories: true)
        defer { try? FileManager.default.removeItem(at: root) }
        try FileManager.default.createSymbolicLink(at: root.appendingPathComponent("network.pmtiles"),
                                                   withDestinationURL: URL(fileURLWithPath: "/private"))
        XCTAssertThrowsError(try AviationChartStyle.resolve(
            JSONSerialization.data(withJSONObject: style()), installed: installed(directory: root.path)
        ))
    }

    func testOfflineChartRejectsRemoteTilesAndGlyphs() throws {
        XCTAssertThrowsError(try AviationChartStyle.resolve(JSONSerialization.data(withJSONObject:
            style(tiles: "https://example.org/{z}/{x}/{y}")), installed: installed()))
        var glyphStyle = style()
        glyphStyle["glyphs"] = "https://example.org/{fontstack}/{range}.pbf"
        XCTAssertThrowsError(try AviationChartStyle.resolve(
            JSONSerialization.data(withJSONObject: glyphStyle), installed: installed()))
    }

    func testSharedMapKeepsOneArchiveBindingAndSeparateVisibleGroups() throws {
        var raw = style()
        raw["layers"] = [["id": "route", "type": "line", "source": "network", "source-layer": "airways"]]
        let data = try JSONSerialization.data(withJSONObject: raw)
        let low = try AviationChartStyle.resolve(data, installed: installed())
        let high = try AviationChartStyle.resolve(data, installed: installed(product: .ifrHigh))
        let terrain = geographic(.terrain)
        let coastline = geographic(.basemap)
        let base = Data("""
        {"version":8,"sources":{"pilotage-coastline":{"type":"vector"},"pilotage-terrain":{"type":"raster-dem"}},
         "layers":[{"id":"background","type":"background"}]}
        """.utf8)
        let map = try AviationMapStyle.assemble(terrain: terrain, coastline: coastline, charts: [low, high], template: base)
        let output = try XCTUnwrap(JSONSerialization.jsonObject(with: Data(map.json.utf8)) as? [String: Any])
        XCTAssertEqual((output["projection"] as? [String: String])?["type"], "globe")
        let metadata = try XCTUnwrap(output["metadata"] as? [String: Any])
        let resources = try XCTUnwrap(metadata["pilotage:resources"] as? [[String: String]])
        XCTAssertEqual(resources.filter { $0["format"] == "pmtiles" }.count, 1)
        XCTAssertEqual(Set(metadata["maplibre:display-groups"] as? [String] ?? []), ["terrain", "ifr_low", "ifr_high"])
        let layers = try XCTUnwrap(output["layers"] as? [[String: Any]])
        let groups = layers.compactMap { ($0["metadata"] as? [String: String])?["maplibre:display-group"] }
        XCTAssertTrue(groups.contains("ifr_low"))
        XCTAssertTrue(groups.contains("ifr_high"))
    }

    private func geographic(_ product: AviationProduct) -> InstalledAviationRelease {
        let sample = installed(product: product).release
        let release = AviationRelease(id: sample.id, product: product, authority: sample.authority,
            edition: sample.edition, revision: 1, channel: "development", validity: nil,
            coverage: sample.coverage,
            artifacts: [AviationArtifact(path: "archive.mbtiles", format: "mbtiles", bytes: 1, sha256: "test")],
            dependencies: [], rendererCapabilities: [], attributions: [])
        return InstalledAviationRelease(release: release, directory: "/installed/" + release.id)
    }
}
