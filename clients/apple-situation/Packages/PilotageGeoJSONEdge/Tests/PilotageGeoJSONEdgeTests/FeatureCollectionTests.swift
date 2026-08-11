import Foundation
import Testing

@testable import PilotageGeoJSONEdge

@Test("Point encoding is stable and uses longitude first")
func pointEncoding() throws {
    let feature = GeoJSONPointFeature(
        id: "traffic-1",
        position: GeoJSONPosition(longitudeDegrees: -71.0, latitudeDegrees: 42.0),
        label: "N1\n5500 ft",
        rotationDegrees: 92.0
    )
    let first = try GeoJSONFeatureCollectionEncoder.encode(points: [feature])
    let second = try GeoJSONFeatureCollectionEncoder.encode(points: [feature])
    let coordinate = try firstCoordinate(in: first)

    #expect(first == second)
    #expect(coordinate == [-71.0, 42.0])
}

@Test("Polygon encoding keeps every ring")
func polygonEncoding() throws {
    let exterior = [
        GeoJSONPosition(longitudeDegrees: -75.0, latitudeDegrees: 40.0),
        GeoJSONPosition(longitudeDegrees: -75.0, latitudeDegrees: 41.0),
        GeoJSONPosition(longitudeDegrees: -74.0, latitudeDegrees: 41.0),
        GeoJSONPosition(longitudeDegrees: -75.0, latitudeDegrees: 40.0),
    ]
    let data = try GeoJSONFeatureCollectionEncoder.encode(
        polygons: [GeoJSONPolygonFeature(id: "weather-1", rings: [exterior], label: "SIGMET")]
    )
    let object = try #require(JSONSerialization.jsonObject(with: data) as? [String: Any])
    let features = try #require(object["features"] as? [[String: Any]])
    let geometry = try #require(features.first?["geometry"] as? [String: Any])
    let rings = try #require(geometry["coordinates"] as? [[[Double]]])

    #expect(rings == [[[-75.0, 40.0], [-75.0, 41.0], [-74.0, 41.0], [-75.0, 40.0]]])
}

@Test("Polygon encoding keeps negative terrain heights and fallback state")
func negativeTerrainHeightEncoding() throws {
    let ring = [
        GeoJSONPosition(longitudeDegrees: -75.0, latitudeDegrees: 40.0),
        GeoJSONPosition(longitudeDegrees: -75.0, latitudeDegrees: 41.0),
        GeoJSONPosition(longitudeDegrees: -74.0, latitudeDegrees: 40.0),
        GeoJSONPosition(longitudeDegrees: -75.0, latitudeDegrees: 40.0),
    ]
    let data = try GeoJSONFeatureCollectionEncoder.encode(
        polygons: [
            GeoJSONPolygonFeature(
                id: "traffic-below-terrain",
                rings: [ring],
                label: "REPORTED ALTITUDE",
                baseAboveTerrainMetres: -100,
                topAboveTerrainMetres: -40,
                usesReportedAltitudeFallback: true
            ),
        ]
    )
    let object = try #require(JSONSerialization.jsonObject(with: data) as? [String: Any])
    let features = try #require(object["features"] as? [[String: Any]])
    let properties = try #require(features.first?["properties"] as? [String: Any])

    #expect(properties["base"] as? Double == -100)
    #expect(properties["top"] as? Double == -40)
    #expect(properties["below_terrain"] as? Bool == true)
    #expect(properties["uses_reported_altitude_fallback"] as? Bool == true)
}

private func firstCoordinate(in data: Data) throws -> [Double] {
    let object = try #require(JSONSerialization.jsonObject(with: data) as? [String: Any])
    let features = try #require(object["features"] as? [[String: Any]])
    let geometry = try #require(features.first?["geometry"] as? [String: Any])
    return try #require(geometry["coordinates"] as? [Double])
}
