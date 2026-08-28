import Foundation
@preconcurrency import MapLibre
import PilotageCore

/// The aircraft's own mark on the map, and the course it is on.
///
/// The mark is drawn from a reported position and from nothing else. Absence of a
/// position is absence of the mark: a map that drew the aircraft at a last-known place
/// would be telling a reader where it is when it does not know.
///
/// The same rule governs the two directions beside it. The mark takes the nose when the
/// aircraft reports one against true north, and the leader takes the course over the
/// ground, which in wind is a different direction. Where neither is reported the mark is
/// a shape with no direction in it and no line beside it.
@MainActor
final class SituationOwnshipMark {
    /// The source and layer the mark is drawn from.
    private static let markSource = "pilotage-ownship-mark"
    private static let markLayer = "pilotage-ownship-mark"
    /// The source and layer the course is drawn from.
    private static let leaderSource = "pilotage-ownship-leader"
    private static let leaderLayer = "pilotage-ownship-leader"

    /// The glyph with a point in it, for an aircraft that reported which way it faces.
    private static let pointedGlyph = "▲"
    /// The glyph with no point in it. A reader reads a direction off a point, so a mark
    /// whose heading nobody reported must not have one.
    private static let pointlessGlyph = "●"

    private var installed = false
    private var markShape: MLNShapeSource?
    private var leaderShape: MLNShapeSource?
    /// Whether a course is on the map already, which selects which end of the speed band
    /// applies. Without a band an aircraft drifting either side of the floor flickers its
    /// course on and off at the report rate.
    private var courseDrawn = false

    /// Put the mark and its course on the map for one reported ownship.
    func apply(_ ownship: DisplayOwnship?, to mapStyle: MLNStyle) {
        install(into: mapStyle)
        guard let ownship else {
            withdraw()
            return
        }
        let heading = trueHeading(of: ownship)
        markShape?.shape = try? MLNShape(
            data: markFeature(ownship, heading: heading),
            encoding: String.Encoding.utf8.rawValue
        )
        leaderShape?.shape = try? MLNShape(
            data: leaderFeature(ownship),
            encoding: String.Encoding.utf8.rawValue
        )
    }

    /// Take the mark and the course off the map.
    ///
    /// A course left drawn is a course still claimed, so it goes with the mark rather
    /// than staying behind it.
    private func withdraw() {
        courseDrawn = false
        markShape?.shape = try? MLNShape(data: emptyCollection, encoding: String.Encoding.utf8.rawValue)
        leaderShape?.shape = try? MLNShape(
            data: emptyCollection, encoding: String.Encoding.utf8.rawValue)
    }

    /// The heading the map may turn the mark to, or nothing.
    ///
    /// The map draws in true north. A magnetic heading is a different number by the local
    /// variation, which is tens of degrees in places, and this display has no variation
    /// model to convert one with — so a heading that is not stated against true north is
    /// withheld rather than drawn as though it were.
    private func trueHeading(of ownship: DisplayOwnship) -> Double? {
        OwnshipMotion.drawableHeading(
            degrees: ownship.headingDeg,
            reference: ownship.headingReference.map(Self.reference)
        )
    }

    /// Read the display's reference as the one the shared rules are stated in.
    private static func reference(_ value: DisplayHeadingReference) -> OwnshipMotion.HeadingReference {
        switch value {
        case .trueNorth: .trueNorth
        case .magneticNorth: .magneticNorth
        case .other: .other
        }
    }

    private func markFeature(_ ownship: DisplayOwnship, heading: Double?) -> Data {
        let glyph = heading == nil ? Self.pointlessGlyph : Self.pointedGlyph
        return feature(
            geometry: """
                {"type":"Point","coordinates":[\(ownship.coordinate.longitudeDeg),\
                \(ownship.coordinate.latitudeDeg)]}
                """,
            properties: """
                {"glyph":"\(glyph)","rotation":\(heading ?? 0)}
                """
        )
    }

    private func leaderFeature(_ ownship: DisplayOwnship) -> Data {
        guard let course = ownship.courseDeg, course.isFinite,
            OwnshipMotion.courseIsDrawable(
                groundSpeedKnots: ownship.groundSpeedKt, alreadyDrawn: courseDrawn),
            let speed = ownship.groundSpeedKt
        else {
            courseDrawn = false
            return emptyCollection
        }
        courseDrawn = true
        let start = OwnshipMotion.Point(
            longitudeDegrees: ownship.coordinate.longitudeDeg,
            latitudeDegrees: ownship.coordinate.latitudeDeg
        )
        let end = OwnshipMotion.leaderEndpoint(
            from: start,
            bearingDegrees: OwnshipMotion.wrappedBearing(course),
            groundSpeedKnots: speed
        )
        return feature(
            geometry: """
                {"type":"LineString","coordinates":[\
                [\(start.longitudeDegrees),\(start.latitudeDegrees)],\
                [\(end.longitudeDegrees),\(end.latitudeDegrees)]]}
                """,
            properties: "{}"
        )
    }

    private func feature(geometry: String, properties: String) -> Data {
        Data(
            """
            {"type":"FeatureCollection","features":[\
            {"type":"Feature","properties":\(properties),"geometry":\(geometry)}]}
            """.utf8)
    }

    private var emptyCollection: Data {
        Data(#"{"type":"FeatureCollection","features":[]}"#.utf8)
    }

    /// Build the sources and layers once for a style.
    ///
    /// A batch arrives for every reception, and rebuilding a layer for each one costs a
    /// style teardown thousands of times a minute.
    private func install(into mapStyle: MLNStyle) {
        guard !installed else { return }
        installed = true

        let leader = MLNShapeSource(identifier: Self.leaderSource, shape: nil, options: nil)
        mapStyle.addSource(leader)
        leaderShape = leader
        let line = MLNLineStyleLayer(identifier: Self.leaderLayer, source: leader)
        line.lineColor = NSExpression(forConstantValue: UIColor(red: 0.84, green: 0, blue: 0.43, alpha: 1))
        line.lineWidth = NSExpression(forConstantValue: 2)
        line.lineCap = NSExpression(forConstantValue: "round")
        mapStyle.addLayer(line)

        let mark = MLNShapeSource(identifier: Self.markSource, shape: nil, options: nil)
        mapStyle.addSource(mark)
        markShape = mark
        let symbol = MLNSymbolStyleLayer(identifier: Self.markLayer, source: mark)
        symbol.text = NSExpression(forKeyPath: "glyph")
        symbol.textColor = NSExpression(
            forConstantValue: UIColor(red: 0.84, green: 0, blue: 0.43, alpha: 1))
        symbol.textFontSize = NSExpression(forConstantValue: 18)
        symbol.textHaloColor = NSExpression(forConstantValue: UIColor.white)
        symbol.textHaloWidth = NSExpression(forConstantValue: 1.5)
        symbol.textRotation = NSExpression(forKeyPath: "rotation")
        // Aligned to the map, not the screen. The map can be turned and opens pitched,
        // and a mark aligned to the screen points somewhere the aircraft is not for as
        // long as either holds.
        symbol.textRotationAlignment = NSExpression(forConstantValue: "map")
        symbol.textPitchAlignment = NSExpression(forConstantValue: "map")
        symbol.textAllowsOverlap = NSExpression(forConstantValue: true)
        mapStyle.addLayer(symbol)
    }
}
