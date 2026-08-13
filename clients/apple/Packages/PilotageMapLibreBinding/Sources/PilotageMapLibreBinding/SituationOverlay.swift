import Foundation
@preconcurrency import MapLibre
import PilotageGeoJSONEdge
import PilotageCore
import UIKit

/// An error at the display binding boundary.
public enum SituationOverlayError: Error, Equatable {
    /// A point refers to a style that is not in the point style catalog.
    case unknownPointStyle(String)
    /// A polygon refers to a style that is not in the polygon style catalog.
    case unknownShapeStyle(String)
}

@MainActor
final class SituationOverlay {
    private var layerIdentifiers: [String] = []
    private var pointLayerIdentifiers: Set<String> = []
    private var points: [String: DisplayPoint] = [:]
    private var sourceIdentifiers: [String] = []
    private var managedSources: [String: MLNShapeSource] = [:]
    private var installedCatalog: [String] = []

    var interactiveLayerIdentifiers: Set<String> { pointLayerIdentifiers }

    /// Put one batch on the map.
    ///
    /// The style catalog is display policy and changes only when the policy does, while a
    /// batch arrives for every reception. Rebuilding every source and every layer for each
    /// batch costs a full style teardown thousands of times a minute, so the layers are
    /// built once for a catalog and only the feature data is replaced after that.
    func apply(_ batch: DisplayBatch, to mapStyle: MLNStyle) throws {
        updatePoints(with: batch)
        let currentPoints = points.values.sorted { $0.id < $1.id }
        try validate(batch, points: currentPoints)
        let entries = catalog(for: batch)
        let signature = entries.map(\.identifier)
        if signature != installedCatalog {
            removeManagedContent(from: mapStyle)
            for entry in entries {
                switch entry {
                case let .point(pointStyle):
                    try addPointStyle(pointStyle, points: currentPoints, to: mapStyle)
                case let .shape(shapeStyle):
                    try addShapeStyle(shapeStyle, batch: batch, to: mapStyle)
                }
            }
            installedCatalog = signature
            return
        }
        for entry in entries {
            switch entry {
            case let .point(pointStyle):
                try replaceShape(
                    in: sourceID("point", pointStyle.id),
                    with: pointData(pointStyle, points: currentPoints)
                )
            case let .shape(shapeStyle):
                try replaceShape(
                    in: sourceID("shape", shapeStyle.id),
                    with: shapeData(shapeStyle, batch: batch)
                )
            }
        }
    }

    private func replaceShape(in identifier: String, with data: Data) throws {
        guard let source = managedSources[identifier] else { return }
        source.shape = try MLNShape(data: data, encoding: String.Encoding.utf8.rawValue)
    }

    private func pointData(_ pointStyle: DisplayPointStyle, points: [DisplayPoint]) -> Data {
        let features = points
            .filter { $0.styleId == pointStyle.id }
            .map(GeoJSONPointFeature.init)
        return (try? GeoJSONFeatureCollectionEncoder.encode(points: features)) ?? Data()
    }

    private func shapeData(_ shapeStyle: DisplayShapeStyle, batch: DisplayBatch) -> Data {
        let features = batch.shapes
            .filter { $0.styleId == shapeStyle.id }
            .map(GeoJSONPolygonFeature.init)
        return (try? GeoJSONFeatureCollectionEncoder.encode(polygons: features)) ?? Data()
    }

    private func validate(_ batch: DisplayBatch, points: [DisplayPoint]) throws {
        let pointStyles = Set(batch.pointStyles.map(\.id))
        if let point = points.first(where: { !pointStyles.contains($0.styleId) }) {
            throw SituationOverlayError.unknownPointStyle(point.styleId)
        }
        let shapeStyles = Set(batch.shapeStyles.map(\.id))
        if let shape = batch.shapes.first(where: { !shapeStyles.contains($0.styleId) }) {
            throw SituationOverlayError.unknownShapeStyle(shape.styleId)
        }
    }

    private func catalog(for batch: DisplayBatch) -> [CatalogEntry] {
        let points = batch.pointStyles.map(CatalogEntry.point)
        let shapes = batch.shapeStyles.map(CatalogEntry.shape)
        return (points + shapes).sorted {
            ($0.order, $0.identifier) < ($1.order, $1.identifier)
        }
    }

    private func addPointStyle(
        _ pointStyle: DisplayPointStyle,
        points: [DisplayPoint],
        to mapStyle: MLNStyle
    ) throws {
        let features = points
            .filter { $0.styleId == pointStyle.id }
            .map(GeoJSONPointFeature.init)
        let data = try GeoJSONFeatureCollectionEncoder.encode(points: features)
        let source = try addSource(data: data, identifier: sourceID("point", pointStyle.id), to: mapStyle)
        if let marker = pointStyle.markerText {
            addMarkerLayer(marker, style: pointStyle, source: source, to: mapStyle)
        } else {
            addCircleLayer(style: pointStyle, source: source, to: mapStyle)
        }
        addPointLabelLayer(style: pointStyle, source: source, to: mapStyle)
    }

    private func updatePoints(with batch: DisplayBatch) {
        if points.isEmpty || batch.pointChanges.isEmpty {
            replacePoints(with: batch.points)
            return
        }
        for change in batch.pointChanges {
            switch change.kind {
            case .upsert:
                if let point = change.point {
                    points[change.id] = point
                }
            case .stale:
                if let point = batch.points.first(where: { $0.id == change.id }) {
                    points[change.id] = point
                }
            case .remove:
                if let target = change.transferTo,
                   let point = batch.points.first(where: { $0.id == target }) {
                    points[target] = point
                }
                points.removeValue(forKey: change.id)
            }
        }
    }

    private func replacePoints(with source: [DisplayPoint]) {
        points.removeAll(keepingCapacity: true)
        for point in source {
            points[point.id] = point
        }
    }

    private func addShapeStyle(
        _ shapeStyle: DisplayShapeStyle,
        batch: DisplayBatch,
        to mapStyle: MLNStyle
    ) throws {
        let features = batch.shapes
            .filter { $0.styleId == shapeStyle.id }
            .map(GeoJSONPolygonFeature.init)
        let data = try GeoJSONFeatureCollectionEncoder.encode(polygons: features)
        let source = try addSource(data: data, identifier: sourceID("shape", shapeStyle.id), to: mapStyle)
        if shapeStyle.extruded {
            addFillExtrusionLayer(style: shapeStyle, source: source, to: mapStyle)
            addBelowTerrainFillLayer(style: shapeStyle, source: source, to: mapStyle)
        } else {
            addFillLayer(style: shapeStyle, source: source, to: mapStyle)
        }
        addLineLayer(style: shapeStyle, source: source, to: mapStyle)
        addShapeLabelLayer(style: shapeStyle, source: source, to: mapStyle)
    }

    private func addSource(data: Data, identifier: String, to mapStyle: MLNStyle) throws -> MLNShapeSource {
        let shape = try MLNShape(data: data, encoding: String.Encoding.utf8.rawValue)
        let source = MLNShapeSource(identifier: identifier, shape: shape, options: nil)
        mapStyle.addSource(source)
        sourceIdentifiers.append(identifier)
        managedSources[identifier] = source
        return source
    }

    private func addCircleLayer(
        style: DisplayPointStyle,
        source: MLNShapeSource,
        to mapStyle: MLNStyle
    ) {
        let layer = MLNCircleStyleLayer(identifier: layerID("mark", style.id), source: source)
        layer.circleColor = constant(color(style.fill))
        layer.circleRadius = constant(style.radiusPoints)
        layer.circleStrokeColor = constant(color(style.outline))
        layer.circleStrokeWidth = constant(style.outlineWidthPoints)
        add(layer, to: mapStyle, interactive: true)
    }

    private func addMarkerLayer(
        _ marker: String,
        style: DisplayPointStyle,
        source: MLNShapeSource,
        to mapStyle: MLNStyle
    ) {
        let layer = MLNSymbolStyleLayer(identifier: layerID("mark", style.id), source: source)
        layer.text = constant(marker)
        layer.textColor = constant(color(style.fill))
        layer.textFontSize = constant(style.markerSizePoints)
        layer.textFontNames = constant(style.markerFontNames)
        layer.textHaloColor = constant(color(style.outline))
        layer.textHaloWidth = constant(style.outlineWidthPoints)
        layer.textRotation = NSExpression(forKeyPath: "rotation")
        layer.textRotationAlignment = constant("map")
        layer.textAllowsOverlap = constant(style.markerAllowsOverlap)
        add(layer, to: mapStyle, interactive: true)
    }

    private func addPointLabelLayer(
        style: DisplayPointStyle,
        source: MLNShapeSource,
        to mapStyle: MLNStyle
    ) {
        let layer = MLNSymbolStyleLayer(identifier: layerID("label", style.id), source: source)
        configureLabel(
            layer,
            color: style.labelColor,
            size: style.labelSizePoints,
            fontNames: style.labelFontNames,
            offsetX: style.labelOffsetX,
            offsetY: style.labelOffsetY,
            allowsOverlap: style.labelAllowsOverlap
        )
        add(layer, to: mapStyle, interactive: true)
    }

    private func addFillLayer(
        style: DisplayShapeStyle,
        source: MLNShapeSource,
        to mapStyle: MLNStyle
    ) {
        let layer = MLNFillStyleLayer(identifier: layerID("fill", style.id), source: source)
        layer.fillColor = constant(color(style.fill))
        add(layer, to: mapStyle)
    }

    /// Raise one shape family between the heights its features carry.
    ///
    /// A polygon draped on the terrain cannot say how high a target or a weather volume
    /// sits. A feature with no height falls back to the surface, which draws the same
    /// outline a flat fill would.
    private func addFillExtrusionLayer(
        style: DisplayShapeStyle,
        source: MLNShapeSource,
        to mapStyle: MLNStyle
    ) {
        let layer = MLNFillExtrusionStyleLayer(identifier: layerID("fill", style.id), source: source)
        layer.predicate = NSPredicate(format: "below_terrain == NO")
        layer.fillExtrusionColor = constant(color(style.fill))
        layer.fillExtrusionOpacity = constant(Double(style.fill.alpha) / 255.0)
        layer.fillExtrusionBase = NSExpression(forKeyPath: "base")
        layer.fillExtrusionHeight = NSExpression(forKeyPath: "top")
        // The pad is what a reader sees of an aircraft that reports a height, so it is
        // what they press. Left out of the interactive set, the only part of the target
        // on screen is the one part that answers nothing.
        add(layer, to: mapStyle, interactive: true)
    }

    /// Draw a flat fill when MapLibre cannot use a negative extrusion height.
    private func addBelowTerrainFillLayer(
        style: DisplayShapeStyle,
        source: MLNShapeSource,
        to mapStyle: MLNStyle
    ) {
        let layer = MLNFillStyleLayer(identifier: layerID("below", style.id), source: source)
        layer.predicate = NSPredicate(format: "below_terrain == YES")
        layer.fillColor = constant(color(style.fill))
        // An aircraft below the terrain is still an aircraft to press.
        add(layer, to: mapStyle, interactive: true)
    }

    private func addLineLayer(
        style: DisplayShapeStyle,
        source: MLNShapeSource,
        to mapStyle: MLNStyle
    ) {
        let layer = MLNLineStyleLayer(identifier: layerID("line", style.id), source: source)
        layer.lineColor = constant(color(style.outline))
        layer.lineWidth = constant(style.outlineWidthPoints)
        add(layer, to: mapStyle)
    }

    private func addShapeLabelLayer(
        style: DisplayShapeStyle,
        source: MLNShapeSource,
        to mapStyle: MLNStyle
    ) {
        let layer = MLNSymbolStyleLayer(identifier: layerID("label", style.id), source: source)
        configureLabel(
            layer,
            color: style.labelColor,
            size: style.labelSizePoints,
            fontNames: style.labelFontNames,
            offsetX: style.labelOffsetX,
            offsetY: style.labelOffsetY,
            allowsOverlap: style.labelAllowsOverlap
        )
        add(layer, to: mapStyle)
    }

    private func configureLabel(
        _ layer: MLNSymbolStyleLayer,
        color: DisplayColor,
        size: Double,
        fontNames: [String],
        offsetX: Double,
        offsetY: Double,
        allowsOverlap: Bool
    ) {
        layer.text = NSExpression(forKeyPath: "label")
        layer.textColor = constant(self.color(color))
        layer.textFontSize = constant(size)
        layer.textFontNames = constant(fontNames)
        layer.textOffset = constant(NSValue(cgVector: CGVector(dx: offsetX, dy: offsetY)))
        layer.textAllowsOverlap = constant(allowsOverlap)
    }

    private func add(
        _ layer: MLNStyleLayer,
        to mapStyle: MLNStyle,
        interactive: Bool = false
    ) {
        mapStyle.addLayer(layer)
        layerIdentifiers.append(layer.identifier)
        if interactive {
            pointLayerIdentifiers.insert(layer.identifier)
        }
    }

    private func removeManagedContent(from mapStyle: MLNStyle) {
        for identifier in layerIdentifiers.reversed() {
            if let layer = mapStyle.layer(withIdentifier: identifier) {
                mapStyle.removeLayer(layer)
            }
        }
        for identifier in sourceIdentifiers.reversed() {
            if let source = mapStyle.source(withIdentifier: identifier) {
                mapStyle.removeSource(source)
            }
        }
        layerIdentifiers.removeAll(keepingCapacity: true)
        pointLayerIdentifiers.removeAll(keepingCapacity: true)
        sourceIdentifiers.removeAll(keepingCapacity: true)
        managedSources.removeAll(keepingCapacity: true)
        installedCatalog.removeAll(keepingCapacity: true)
    }

    private func sourceID(_ kind: String, _ styleID: String) -> String {
        "pilotage-\(kind)-source-\(styleID)"
    }

    private func layerID(_ kind: String, _ styleID: String) -> String {
        "pilotage-\(kind)-layer-\(styleID)"
    }

    private func color(_ value: DisplayColor) -> UIColor {
        UIColor(
            red: CGFloat(value.red) / 255.0,
            green: CGFloat(value.green) / 255.0,
            blue: CGFloat(value.blue) / 255.0,
            alpha: CGFloat(value.alpha) / 255.0
        )
    }

    private func constant(_ value: Any) -> NSExpression {
        NSExpression(forConstantValue: value)
    }
}

private enum CatalogEntry {
    case point(DisplayPointStyle)
    case shape(DisplayShapeStyle)

    var order: Int32 {
        switch self {
        case let .point(style): style.order
        case let .shape(style): style.order
        }
    }

    var identifier: String {
        switch self {
        case let .point(style): "point-\(style.id)"
        case let .shape(style): "shape-\(style.id)"
        }
    }
}

private extension GeoJSONPointFeature {
    init(_ point: DisplayPoint) {
        self.init(
            id: point.id,
            position: GeoJSONPosition(
                longitudeDegrees: point.coordinate.longitudeDeg,
                latitudeDegrees: point.coordinate.latitudeDeg
            ),
            label: point.label,
            rotationDegrees: point.rotationDeg
        )
    }
}

private extension GeoJSONPolygonFeature {
    init(_ shape: DisplayShape) {
        self.init(
            id: shape.id,
            rings: shape.rings.map { ring in
                ring.coordinates.map { coordinate in
                    GeoJSONPosition(
                        longitudeDegrees: coordinate.longitudeDeg,
                        latitudeDegrees: coordinate.latitudeDeg
                    )
                }
            },
            label: shape.label,
            baseAboveTerrainMetres: shape.baseAboveTerrainM,
            topAboveTerrainMetres: shape.topAboveTerrainM,
            usesReportedAltitudeFallback: shape.usesReportedAltitudeFallback
        )
    }
}
