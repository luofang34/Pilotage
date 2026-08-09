import Foundation
@preconcurrency import MapLibre
import PilotageGeoJSONEdge
import PilotageSituationCore
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
    private var sourceIdentifiers: [String] = []

    func apply(_ batch: DisplayBatch, to mapStyle: MLNStyle) throws {
        try validate(batch)
        removeManagedContent(from: mapStyle)
        for entry in catalog(for: batch) {
            switch entry {
            case let .point(pointStyle):
                try addPointStyle(pointStyle, batch: batch, to: mapStyle)
            case let .shape(shapeStyle):
                try addShapeStyle(shapeStyle, batch: batch, to: mapStyle)
            }
        }
    }

    private func validate(_ batch: DisplayBatch) throws {
        let pointStyles = Set(batch.pointStyles.map(\.id))
        if let point = batch.points.first(where: { !pointStyles.contains($0.styleId) }) {
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
        batch: DisplayBatch,
        to mapStyle: MLNStyle
    ) throws {
        let features = batch.points
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
        addFillLayer(style: shapeStyle, source: source, to: mapStyle)
        addLineLayer(style: shapeStyle, source: source, to: mapStyle)
        addShapeLabelLayer(style: shapeStyle, source: source, to: mapStyle)
    }

    private func addSource(data: Data, identifier: String, to mapStyle: MLNStyle) throws -> MLNShapeSource {
        let shape = try MLNShape(data: data, encoding: String.Encoding.utf8.rawValue)
        let source = MLNShapeSource(identifier: identifier, shape: shape, options: nil)
        mapStyle.addSource(source)
        sourceIdentifiers.append(identifier)
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
        add(layer, to: mapStyle)
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
        add(layer, to: mapStyle)
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
        add(layer, to: mapStyle)
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

    private func add(_ layer: MLNStyleLayer, to mapStyle: MLNStyle) {
        mapStyle.addLayer(layer)
        layerIdentifiers.append(layer.identifier)
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
        sourceIdentifiers.removeAll(keepingCapacity: true)
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
            label: shape.label
        )
    }
}
