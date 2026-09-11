import PilotageCore
import UIKit
import XCTest
@testable import Pilotage

@MainActor
final class GlobeSituationOverlayTests: XCTestCase {
    func testDisplayedTrafficCanBeSelectedAndRemovalClearsItsHitRegion() {
        let view = overlay()
        view.project = { positions in
            XCTAssertEqual(positions, [40.5, -76.5, 0])
            return [CGPoint(x: 100, y: 100)]
        }
        view.batch = batch()
        render(view)
        XCTAssertEqual(view.feature(at: CGPoint(x: 100, y: 100)), "aircraft")
        XCTAssertNil(view.feature(at: CGPoint(x: 20, y: 20)))
        var empty = batch()
        empty.points = []
        view.batch = empty
        render(view)
        XCTAssertNil(view.feature(at: CGPoint(x: 100, y: 100)))
    }

    func testHiddenTrafficIsNeitherDrawnNorSelectable() {
        let view = overlay()
        view.project = { _ in
            XCTFail("A hidden point must not enter the draw projection.")
            return [CGPoint(x: 100, y: 100)]
        }
        var hidden = batch()
        hidden.layers = [DisplayLayerControl(id: "traffic", title: "Traffic", enabled: false,
            sourceState: .live, sourceStateLabel: "Live", sourceDetail: "Test source")]
        view.batch = hidden
        render(view)
        XCTAssertNil(view.feature(at: CGPoint(x: 100, y: 100)))
    }

    func testGlobeOcclusionRemovesTheTrafficHitRegion() {
        let view = overlay()
        view.project = { _ in [nil] }
        view.batch = batch()
        render(view)
        XCTAssertNil(view.feature(at: CGPoint(x: 100, y: 100)))
    }

    private func overlay() -> GlobeSituationOverlay {
        let view = GlobeSituationOverlay()
        view.frame = CGRect(x: 0, y: 0, width: 200, height: 200)
        return view
    }

    private func render(_ view: GlobeSituationOverlay) {
        _ = UIGraphicsImageRenderer(size: view.bounds.size).image { _ in view.draw(view.bounds) }
    }

    private func batch() -> DisplayBatch {
        let red = DisplayColor(red: 255, green: 0, blue: 0, alpha: 255)
        let style = DisplayPointStyle(id: "target", fill: red, outline: red, outlineWidthPoints: 1,
            radiusPoints: 8, markerText: nil, markerSizePoints: 16, markerFontNames: [], markerAllowsOverlap: true,
            labelColor: red, labelSizePoints: 12, labelFontNames: [], labelOffsetX: 0, labelOffsetY: 2,
            labelAllowsOverlap: false, order: 1)
        let point = DisplayPoint(id: "aircraft", layerId: "traffic",
            coordinate: DisplayCoordinate(latitudeDeg: 40.5, longitudeDeg: -76.5), styleId: "target",
            label: nil, altitudeFt: nil, rotationDeg: 0, positionIsExtrapolated: false,
            producerInstanceId: 1, snapshotRevision: 1)
        return DisplayBatch(layers: [], pointStyles: [style], shapeStyles: [], points: [point],
            pointChanges: [], shapes: [], positionlessTraffic: [], trafficDetails: [], omittedProducts: 0, ownship: nil)
    }
}
