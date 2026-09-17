import UIKit

enum RouteLineGeometry {
    static func clipped(_ a: CGPoint, _ b: CGPoint, to bounds: CGRect) -> (CGPoint, CGPoint)? {
        let dx = b.x - a.x, dy = b.y - a.y
        guard [a.x, a.y, b.x, b.y, dx, dy].allSatisfy(\.isFinite) else { return nil }
        var low: CGFloat = 0, high: CGFloat = 1
        for (p, q) in [(-dx, a.x - bounds.minX), (dx, bounds.maxX - a.x),
                       (-dy, a.y - bounds.minY), (dy, bounds.maxY - a.y)] {
            if p == 0 { if q < 0 { return nil }; continue }
            let fraction = q / p
            if p < 0 { low = max(low, fraction) } else { high = min(high, fraction) }
            if low > high { return nil }
        }
        return (CGPoint(x: a.x + low * dx, y: a.y + low * dy), CGPoint(x: a.x + high * dx, y: a.y + high * dy))
    }

    static func path(_ projected: [CGPoint?], in bounds: CGRect) -> UIBezierPath {
        let path = UIBezierPath()
        for (a, b) in zip(projected, projected.dropFirst()) {
            guard let a, let b, let (start, end) = clipped(a, b, to: bounds) else { continue }
            path.move(to: start)
            path.addLine(to: end)
        }
        return path
    }
}
