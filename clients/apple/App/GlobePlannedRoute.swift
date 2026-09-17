import UIKit

struct PlannedMapRoute: Equatable {
    let id: String
    let name: String
    let tokens: [RouteToken]
    let selected: Bool
    var currentLegIndex: Int?

    func role(endingAt index: Int) -> RouteLegRole {
        guard let currentLegIndex else { return .planned }
        if index < currentLegIndex { return .past }
        return index == currentLegIndex ? .current : .planned
    }
}

struct PreparedMapRoute {
    let route: PlannedMapRoute
    let legs: [[[Double]]]
    init(_ route: PlannedMapRoute) {
        self.route = route
        legs = zip(route.tokens, route.tokens.dropFirst()).map { routeArc(from: $0.point, to: $1.point) }
    }
}

extension GlobeSituationOverlay {
    func drawPlannedRoute(context: CGContext) {
        for prepared in preparedRoutes.sorted(by: { !$0.route.selected && $1.route.selected }) {
            guard let project else { return }
            let route = prepared.route
            context.saveGState()
            context.setAlpha(route.selected ? 1 : 0.5)
            context.setLineDash(phase: 0, lengths: [])
            for (index, parts) in prepared.legs.enumerated() {
                let path = UIBezierPath()
                for points in parts { path.append(RouteLineGeometry.path(project(points), in: bounds.insetBy(dx: -8, dy: -8))) }
                let role = route.role(endingAt: index + 1)
                path.lineCapStyle = .round
                UIColor.white.withAlphaComponent(0.95).setStroke()
                path.lineWidth = role == .current ? 7 : 5
                path.stroke()
                routeColor(role).setStroke()
                path.lineWidth = role == .current ? 4 : 2.5
                path.stroke()
            }
            drawRoutePoints(route, context: context)
            context.restoreGState()
        }
    }

    private func drawRoutePoints(_ route: PlannedMapRoute, context: CGContext) {
        guard let project else { return }
        for (index, token) in route.tokens.enumerated() {
            guard let position = project([token.point.latitudeDeg, token.point.longitudeDeg, 0]).first,
                  let position, bounds.insetBy(dx: -30, dy: -30).contains(position) else { continue }
            let color = routeColor(route.role(endingAt: max(1, index)))
            let circle = UIBezierPath(ovalIn: CGRect(x: position.x - 5, y: position.y - 5, width: 10, height: 10))
            color.setFill()
            UIColor.white.setStroke()
            circle.lineWidth = 2
            circle.fill()
            circle.stroke()
            let attributes: [NSAttributedString.Key: Any] = [.font: UIFont.monospacedSystemFont(ofSize: 13, weight: .semibold),
                .foregroundColor: color, .strokeColor: UIColor.white, .strokeWidth: -3]
            let label = index == 0 && plannedRoutes.count > 1 ? route.name + " · " + token.label : token.label
            (label as NSString).draw(at: CGPoint(x: position.x + 9, y: position.y - 9), withAttributes: attributes)
        }
    }

    private func routeColor(_ role: RouteLegRole) -> UIColor {
        switch role { case .current: .magenta; case .planned: .systemBlue; case .past: .systemOrange }
    }
}

func routeArc(from: NavigationPoint, to: NavigationPoint) -> [[Double]] {
    func vector(_ point: NavigationPoint) -> SIMD3<Double> {
        let latitude = point.latitudeDeg * .pi / 180, longitude = point.longitudeDeg * .pi / 180
        return SIMD3(cos(latitude) * cos(longitude), cos(latitude) * sin(longitude), sin(latitude))
    }
    let a = vector(from), b = vector(to)
    let angle = acos(max(-1, min(1, a.x * b.x + a.y * b.y + a.z * b.z)))
    guard angle < .pi - 0.000001 else { return [] }
    func coordinate(_ fraction: Double) -> [Double] {
        let v = angle < 0.000001 ? a : (a * sin((1 - fraction) * angle) + b * sin(fraction * angle)) / sin(angle)
        return [atan2(v.z, hypot(v.x, v.y)) * 180 / .pi, atan2(v.y, v.x) * 180 / .pi, 0]
    }
    let steps = max(1, Int(ceil(angle * 180 / .pi)))
    var parts: [[Double]] = [], part = coordinate(0), previous = coordinate(0)
    for step in 1...steps {
        let fraction = Double(step) / Double(steps), point = coordinate(fraction)
        if abs(point[1] - previous[1]) > 180 {
            let boundary = previous[1] > 0 ? 180.0 : -180.0
            var low = Double(step - 1) / Double(steps), high = fraction
            for _ in 0..<32 {
                let middle = (low + high) / 2, candidate = coordinate(middle)
                if (candidate[1] > 0) == (previous[1] > 0) { low = middle } else { high = middle }
            }
            let latitude = coordinate((low + high) / 2)[0]
            part += [latitude, boundary, 0]
            parts.append(part)
            part = [latitude, -boundary, 0]
        }
        part += point
        previous = point
    }
    parts.append(part)
    return parts
}
