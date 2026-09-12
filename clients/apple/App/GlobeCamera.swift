import simd

/// The globe position and camera distance above the selected point.
struct GlobeCamera {
    static let earthRadius = 6_371_008.8
    var latitude = 40.5
    var longitude = -76.5
    var distance = 12_000_000.0
    var heading = 0.0
    var pitch = 0.0

    var overviewWeight: Double {
        let fraction = min(1, max(0, (distance - 500_000) / 3_500_000))
        return fraction * fraction * (3 - 2 * fraction)
    }

    var displayHeading: Double {
        let wrapped = ((heading + 180).truncatingRemainder(dividingBy: 360) + 360)
            .truncatingRemainder(dividingBy: 360) - 180
        return wrapped * (1 - overviewWeight)
    }

    var displayPitch: Double { pitch * (1 - overviewWeight) }

    func chartZoom(width: Int, height: Int) -> Double {
        let groundWidth = distance * 0.82842712
        let circumference = 2 * Double.pi * Self.earthRadius * cos(latitude * .pi / 180)
        let pixels = Double(max(1, min(width, height)))
        return min(24, max(0, log2(circumference * pixels / (512 * groundWidth))))
    }

    static func frustumTangents(width: Int, height: Int) -> [Float] {
        let aspect = Float(max(width, 1)) / Float(max(height, 1))
        let vertical: Float = 0.41421356 / min(aspect, 1)
        return [vertical * aspect, vertical * aspect, vertical, vertical]
    }

    mutating func zoom(by scale: Double) {
        guard scale.isFinite, scale > 0 else { return }
        distance = min(max(distance / scale, 150), 80_000_000)
    }

    mutating func pan(x: Double, y: Double, height: Double) {
        guard height > 0 else { return }
        let degrees = distance * 0.8284 / Self.earthRadius * 180 / .pi / height
        let bearing = displayHeading * .pi / 180
        let east = x * cos(bearing) - y * sin(bearing)
        let north = x * sin(bearing) + y * cos(bearing)
        latitude = min(max(latitude + north * degrees, -85), 85)
        longitude -= east * degrees / max(cos(latitude * .pi / 180), 0.08)
        longitude = (longitude + 180).truncatingRemainder(dividingBy: 360)
        if longitude < 0 { longitude += 360 }
        longitude -= 180
    }

    var placement: simd_float4x4 {
        let tilt = simd_quatf(angle: Float(displayPitch * .pi / 180), axis: [1, 0, 0])
        let bearing = simd_quatf(angle: Float(displayHeading * .pi / 180), axis: [0, 0, 1])
        var matrix = simd_float4x4(tilt * bearing)
        let scale = Float(1 / Self.earthRadius)
        matrix.columns.0 *= scale
        matrix.columns.1 *= scale
        matrix.columns.2 *= scale
        matrix.columns.3 = [0, 0, -Float(distance / Self.earthRadius), 1]
        return matrix
    }
}

/// A camera movement that follows the shortest path across north and the date line.
struct GlobeCameraMotion {
    let start: GlobeCamera
    let target: GlobeCamera
    let startedAt: Double
    static let duration = 0.35

    func value(at time: Double) -> GlobeCamera {
        let fraction = min(1, max(0, (time - startedAt) / Self.duration))
        guard fraction < 1 else { return target }
        let weight = fraction * fraction * (3 - 2 * fraction)
        var result = start
        result.latitude += (target.latitude - start.latitude) * weight
        result.longitude += Self.angularDelta(from: start.longitude, to: target.longitude) * weight
        result.longitude = Self.angularDelta(from: 0, to: result.longitude)
        result.heading += Self.angularDelta(from: start.heading, to: target.heading) * weight
        result.pitch += (target.pitch - start.pitch) * weight
        result.distance += (target.distance - start.distance) * weight
        return result
    }

    private static func angularDelta(from start: Double, to target: Double) -> Double {
        ((target - start + 180).truncatingRemainder(dividingBy: 360) + 360)
            .truncatingRemainder(dividingBy: 360) - 180
    }
}
