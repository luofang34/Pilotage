import Foundation

/// One position that uses longitude and latitude order.
public struct GeoJSONPosition: Equatable, Sendable {
    /// Longitude in degrees, positive east.
    public let longitudeDegrees: Double
    /// Latitude in degrees, positive north.
    public let latitudeDegrees: Double

    /// Create one position.
    public init(longitudeDegrees: Double, latitudeDegrees: Double) {
        self.longitudeDegrees = longitudeDegrees
        self.latitudeDegrees = latitudeDegrees
    }
}

/// One point feature at the display edge.
public struct GeoJSONPointFeature: Equatable, Sendable {
    /// Stable feature identity.
    public let id: String
    /// Feature position.
    public let position: GeoJSONPosition
    /// Ready-to-display text.
    public let label: String?
    /// Clockwise rotation from geographic north.
    public let rotationDegrees: Double

    /// Create one point feature.
    public init(
        id: String,
        position: GeoJSONPosition,
        label: String?,
        rotationDegrees: Double
    ) {
        self.id = id
        self.position = position
        self.label = label
        self.rotationDegrees = rotationDegrees
    }
}

/// One polygon feature at the display edge.
public struct GeoJSONPolygonFeature: Equatable, Sendable {
    /// Stable feature identity.
    public let id: String
    /// Polygon rings.
    public let rings: [[GeoJSONPosition]]
    /// Ready-to-display text.
    public let label: String?

    /// Create one polygon feature.
    public init(id: String, rings: [[GeoJSONPosition]], label: String?) {
        self.id = id
        self.rings = rings
        self.label = label
    }
}

/// Encodes display-edge features as deterministic GeoJSON.
public enum GeoJSONFeatureCollectionEncoder {
    /// Encode a point feature collection.
    public static func encode(points: [GeoJSONPointFeature]) throws -> Data {
        let features = points.map(pointObject)
        return try encode(features: features)
    }

    /// Encode a polygon feature collection.
    public static func encode(polygons: [GeoJSONPolygonFeature]) throws -> Data {
        let features = polygons.map(polygonObject)
        return try encode(features: features)
    }

    private static func encode(features: [[String: Any]]) throws -> Data {
        let collection: [String: Any] = [
            "features": features,
            "type": "FeatureCollection",
        ]
        return try JSONSerialization.data(withJSONObject: collection, options: [.sortedKeys])
    }

    private static func pointObject(_ point: GeoJSONPointFeature) -> [String: Any] {
        var properties: [String: Any] = ["rotation": point.rotationDegrees]
        properties["label"] = point.label
        return [
            "geometry": [
                "coordinates": positionObject(point.position),
                "type": "Point",
            ],
            "id": point.id,
            "properties": properties,
            "type": "Feature",
        ]
    }

    private static func polygonObject(_ polygon: GeoJSONPolygonFeature) -> [String: Any] {
        var properties: [String: Any] = [:]
        properties["label"] = polygon.label
        return [
            "geometry": [
                "coordinates": polygon.rings.map { $0.map(positionObject) },
                "type": "Polygon",
            ],
            "id": polygon.id,
            "properties": properties,
            "type": "Feature",
        ]
    }

    private static func positionObject(_ position: GeoJSONPosition) -> [Double] {
        [position.longitudeDegrees, position.latitudeDegrees]
    }
}
