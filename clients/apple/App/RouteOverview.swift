import CoreLocation

struct RouteOverview {
    let center: CLLocationCoordinate2D
    let radiusMeters: Double
    init?(_ coordinates: [CLLocationCoordinate2D]) {
        guard !coordinates.isEmpty, coordinates.allSatisfy(CLLocationCoordinate2DIsValid) else { return nil }
        let longitudes = coordinates.map { ($0.longitude + 360).truncatingRemainder(dividingBy: 360) }.sorted()
        var gap = -1.0, start = longitudes[0], span = 0.0
        for index in longitudes.indices {
            let next = index + 1 < longitudes.count ? longitudes[index + 1] : longitudes[0] + 360
            if next - longitudes[index] > gap {
                gap = next - longitudes[index]; start = next; span = 360 - gap
            }
        }
        let latitude = ((coordinates.map(\.latitude).min() ?? 0) + (coordinates.map(\.latitude).max() ?? 0)) / 2
        var longitude = (start + span / 2).truncatingRemainder(dividingBy: 360)
        if longitude > 180 { longitude -= 360 }
        center = CLLocationCoordinate2D(latitude: latitude, longitude: longitude)
        let location = CLLocation(latitude: latitude, longitude: longitude)
        radiusMeters = max(5_000, coordinates.map { location.distance(from: CLLocation(latitude: $0.latitude, longitude: $0.longitude)) }.max() ?? 0)
    }
}
