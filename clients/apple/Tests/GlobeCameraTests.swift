import XCTest
import simd
@testable import GlobeCameraUnderTest

final class GlobeCameraTests: XCTestCase {
    func testWholeGlobeFitsNarrowAndWideWindows() {
        let camera = GlobeCamera()
        let globeHalfAngle = asin(1 / (1 + camera.distance / GlobeCamera.earthRadius))
        for (width, height) in [(320, 1200), (1200, 320), (768, 1024), (1024, 768)] {
            let tangents = GlobeCamera.frustumTangents(width: width, height: height)
            for tangent in tangents {
                XCTAssertGreaterThan(atan(Double(tangent)), globeHalfAngle)
            }
        }
    }

    func testZoomBoundsAndInvalidInput() {
        var camera = GlobeCamera()
        camera.zoom(by: .infinity)
        camera.zoom(by: 0)
        XCTAssertEqual(camera.distance, 12_000_000)
        camera.zoom(by: 1e20)
        XCTAssertEqual(camera.distance, 150)
        camera.zoom(by: 1e-20)
        XCTAssertEqual(camera.distance, 80_000_000)
    }

    func testPanCrossesDateLine() {
        var camera = GlobeCamera(latitude: 0, longitude: 179)
        camera.pan(x: -100, y: 0, height: 1000)
        XCTAssertLessThan(camera.longitude, 0)
        XCTAssertGreaterThanOrEqual(camera.longitude, -180)
    }

    func testPolarPanStaysFinite() {
        var camera = GlobeCamera(latitude: 84, longitude: 0)
        camera.pan(x: 1000, y: 1000, height: 100)
        XCTAssertEqual(camera.latitude, 85)
        XCTAssertTrue(camera.longitude.isFinite)
        XCTAssertLessThan(camera.longitude, 180)
    }

    func testGlobePlacementFacesCamera() {
        let camera = GlobeCamera()
        let focus = camera.placement * SIMD4<Float>(0, 0, 0, 1)
        let center = camera.placement * SIMD4<Float>(0, 0, -Float(GlobeCamera.earthRadius), 1)
        XCTAssertEqual(focus.x, 0)
        XCTAssertEqual(focus.y, 0)
        XCTAssertLessThan(focus.z, 0)
        XCTAssertEqual(focus.z - center.z, 1, accuracy: 0.00001)
    }

    func testTiltPreservesScaleAndFocus() {
        var camera = GlobeCamera()
        camera.distance = 134_000
        camera.pitch = 60
        camera.heading = 123
        let matrix = camera.placement
        let scale = Float(1 / GlobeCamera.earthRadius)
        XCTAssertEqual(simd_length(matrix.columns.0), scale, accuracy: scale * 0.0001)
        XCTAssertEqual(simd_length(matrix.columns.1), scale, accuracy: scale * 0.0001)
        XCTAssertEqual(simd_length(matrix.columns.2), scale, accuracy: scale * 0.0001)
        XCTAssertEqual(matrix.columns.3.x, 0)
        XCTAssertEqual(matrix.columns.3.y, 0)
    }

    func testOverviewReturnsToNorthUpWithoutMovingTheMapCenter() {
        var camera = GlobeCamera(latitude: 40.5, longitude: -76.5, distance: 134_000, heading: 350, pitch: 60)
        XCTAssertEqual(camera.displayHeading, -10, accuracy: 1e-10)
        XCTAssertEqual(camera.displayPitch, 60)
        camera.zoom(by: 0.05)
        XCTAssertGreaterThan(camera.displayHeading, -10)
        XCTAssertLessThan(camera.displayHeading, 0)
        camera.zoom(by: 0.5)
        XCTAssertEqual(camera.displayHeading, 0)
        XCTAssertEqual(camera.displayPitch, 0)
        XCTAssertEqual(camera.latitude, 40.5)
        XCTAssertEqual(camera.longitude, -76.5)
    }

    func testGlobalPanUsesTheVisibleNorthUpOrientation() {
        var rotated = GlobeCamera(latitude: 0, longitude: 0, heading: 90)
        var northUp = GlobeCamera(latitude: 0, longitude: 0, heading: 0)
        rotated.pan(x: 100, y: 0, height: 1000)
        northUp.pan(x: 100, y: 0, height: 1000)
        XCTAssertEqual(rotated.latitude, northUp.latitude)
        XCTAssertEqual(rotated.longitude, northUp.longitude)
    }

    func testCameraMotionReturnsAcrossNorthWithoutMovingTheCenter() {
        let start = GlobeCamera(distance: 134_000, heading: 350, pitch: 60)
        var target = start
        target.heading = 0
        target.pitch = 0
        let motion = GlobeCameraMotion(start: start, target: target, startedAt: 10)
        let middle = motion.value(at: 10 + GlobeCameraMotion.duration / 2)
        XCTAssertEqual(middle.heading, 355, accuracy: 0.001)
        XCTAssertEqual(middle.pitch, 30, accuracy: 0.001)
        XCTAssertEqual(middle.latitude, start.latitude)
        XCTAssertEqual(middle.longitude, start.longitude)
        XCTAssertEqual(middle.distance, start.distance)
        let end = motion.value(at: 11)
        XCTAssertEqual(end.heading, 0)
        XCTAssertEqual(end.pitch, 0)
    }

    func testCameraMotionCrossesTheDateLineWithinTheRendererBounds() {
        let start = GlobeCamera(latitude: 20, longitude: 179, distance: 134_000)
        var target = start
        target.longitude = -179
        let motion = GlobeCameraMotion(start: start, target: target, startedAt: 0)
        let position = motion.value(at: GlobeCameraMotion.duration * 0.75)
        XCTAssertLessThan(position.longitude, -179)
        XCTAssertGreaterThanOrEqual(position.longitude, -180)
        XCTAssertEqual(position.latitude, start.latitude)
        XCTAssertEqual(position.distance, start.distance)
    }

    func testChartZoomTracksDistanceAndAvailableWindowWidth() {
        var camera = GlobeCamera(distance: 134_000)
        let zoom = camera.chartZoom(width: 1200, height: 900)
        camera.zoom(by: 2)
        XCTAssertEqual(camera.chartZoom(width: 1200, height: 900), zoom + 1, accuracy: 1e-8)
        XCTAssertEqual(camera.chartZoom(width: 450, height: 1200), zoom, accuracy: 1e-8)
    }
}
