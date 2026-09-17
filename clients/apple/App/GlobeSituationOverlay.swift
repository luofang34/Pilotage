import PilotageCore
import UIKit

/// Live display values use the camera that draws the map beneath them.
@MainActor
final class GlobeSituationOverlay: UIView {
    var batch: DisplayBatch?
    var plannedRoutes: [PlannedMapRoute] = [] {
        didSet {
            if plannedRoutes != oldValue { preparedRoutes = plannedRoutes.map(PreparedMapRoute.init) }
        }
    }
    var preparedRoutes: [PreparedMapRoute] = []
    var project: (([Double]) -> [CGPoint?])?
    var heading = 0.0
    private var hits: [(String, UIBezierPath)] = []
    private var labels: [CGRect] = []

    init() {
        super.init(frame: .zero)
        isOpaque = false
        isUserInteractionEnabled = false
        backgroundColor = .clear
        contentMode = .redraw
    }

    @available(*, unavailable)
    required init?(coder: NSCoder) { nil }

    func feature(at point: CGPoint) -> String? {
        hits.reversed().first { $0.1.contains(point) }?.0
    }

    override func draw(_ rect: CGRect) {
        hits.removeAll(keepingCapacity: true)
        labels.removeAll(keepingCapacity: true)
        guard let context = UIGraphicsGetCurrentContext(), project != nil else { return }
        drawPlannedRoute(context: context)
        guard let batch else { return }
        let hidden = Set(batch.layers.filter { !$0.enabled }.map(\.id))
        let entries = batch.pointStyles.map { Entry.point($0) } + batch.shapeStyles.map { Entry.shape($0) }
        for entry in entries.sorted(by: { ($0.order, $0.id) < ($1.order, $1.id) }) {
            switch entry {
            case let .point(style):
                for point in batch.points where point.styleId == style.id && !hidden.contains(point.layerId) {
                    draw(point, style: style, context: context)
                }
            case let .shape(style):
                for shape in batch.shapes where shape.styleId == style.id && !hidden.contains(shape.layerId) {
                    draw(shape, style: style, context: context)
                }
            }
        }
    }

    private func draw(_ point: DisplayPoint, style: DisplayPointStyle, context: CGContext) {
        guard let position = project?([point.coordinate.latitudeDeg, point.coordinate.longitudeDeg, 0]).first,
              let position, bounds.insetBy(dx: -60, dy: -60).contains(position) else { return }
        context.saveGState()
        context.translateBy(x: position.x, y: position.y)
        context.rotate(by: (point.rotationDeg - heading) * .pi / 180)
        let radius = style.markerText == nil ? style.radiusPoints : style.markerSizePoints * 0.6
        if let marker = style.markerText {
            let attributes: [NSAttributedString.Key: Any] = [
                .font: font(style.markerFontNames, size: style.markerSizePoints),
                .foregroundColor: color(style.fill), .strokeColor: color(style.outline),
                .strokeWidth: -style.outlineWidthPoints,
            ]
            let size = (marker as NSString).size(withAttributes: attributes)
            (marker as NSString).draw(at: CGPoint(x: -size.width / 2, y: -size.height / 2), withAttributes: attributes)
        } else {
            let path = UIBezierPath(ovalIn: CGRect(x: -radius, y: -radius, width: radius * 2, height: radius * 2))
            color(style.fill).setFill()
            color(style.outline).setStroke()
            path.lineWidth = style.outlineWidthPoints
            path.fill()
            path.stroke()
        }
        context.restoreGState()
        let hit = UIBezierPath(ovalIn: CGRect(x: position.x - max(16, radius), y: position.y - max(16, radius),
                                            width: max(32, radius * 2), height: max(32, radius * 2)))
        hits.append((point.id, hit))
        label(point.label, at: position, color: style.labelColor, size: style.labelSizePoints,
              fonts: style.labelFontNames, x: style.labelOffsetX, y: style.labelOffsetY,
              overlaps: style.labelAllowsOverlap, feature: point.id)
    }

    private func draw(_ shape: DisplayShape, style: DisplayShapeStyle, context: CGContext) {
        let top = style.extruded ? max(0, shape.topAboveTerrainM ?? 0) : 0
        let base = style.extruded ? max(0, shape.baseAboveTerrainM ?? 0) : 0
        let path = UIBezierPath()
        path.usesEvenOddFillRule = true
        for ring in shape.rings {
            let coordinates = ring.coordinates.flatMap { [$0.latitudeDeg, $0.longitudeDeg, top] }
            guard let projected = project?(coordinates), projected.count >= 3,
                  projected.allSatisfy({ $0 != nil }) else { continue }
            let vertices = projected.compactMap { $0 }
            guard let first = vertices.first else { continue }
            path.move(to: first)
            for position in vertices.dropFirst() { path.addLine(to: position) }
            path.close()
            if top > base {
                drawSides(ring.coordinates, top: vertices, base: base, color: style.fill)
            }
        }
        guard !path.isEmpty else { return }
        color(style.fill).setFill()
        color(style.outline).setStroke()
        path.lineWidth = style.outlineWidthPoints
        path.fill()
        path.stroke()
        if style.extruded { hits.append((shape.id, path)) }
        let center = CGPoint(x: path.bounds.midX, y: path.bounds.midY)
        label(shape.label, at: center, color: style.labelColor, size: style.labelSizePoints,
              fonts: style.labelFontNames, x: style.labelOffsetX, y: style.labelOffsetY,
              overlaps: style.labelAllowsOverlap, feature: style.extruded ? shape.id : nil)
    }

    private func drawSides(_ coordinates: [DisplayCoordinate], top: [CGPoint], base: Double, color value: DisplayColor) {
        guard let floor = project?(coordinates.flatMap { [$0.latitudeDeg, $0.longitudeDeg, base] }),
              floor.count == top.count else { return }
        color(value).setFill()
        for index in 1..<top.count {
            guard let first = floor[index - 1], let second = floor[index] else { continue }
            let side = UIBezierPath()
            side.move(to: first)
            side.addLine(to: second)
            side.addLine(to: top[index])
            side.addLine(to: top[index - 1])
            side.close()
            side.fill()
        }
    }

    private func label(
        _ text: String?, at position: CGPoint, color value: DisplayColor, size: Double,
        fonts: [String], x: Double, y: Double, overlaps: Bool, feature: String?
    ) {
        guard let text, !text.isEmpty, size > 0 else { return }
        let attributes: [NSAttributedString.Key: Any] = [
            .font: font(fonts, size: size), .foregroundColor: color(value),
            .strokeColor: UIColor.black.withAlphaComponent(0.8), .strokeWidth: -2,
        ]
        let dimensions = (text as NSString).size(withAttributes: attributes)
        let rect = CGRect(x: position.x + x * size - dimensions.width / 2,
                          y: position.y + y * size - dimensions.height / 2,
                          width: dimensions.width, height: dimensions.height)
        guard bounds.intersects(rect), overlaps || !labels.contains(where: { $0.intersects(rect) }) else { return }
        labels.append(rect)
        (text as NSString).draw(in: rect, withAttributes: attributes)
        if let feature { hits.append((feature, UIBezierPath(rect: rect.insetBy(dx: -4, dy: -4)))) }
    }

    private func font(_ names: [String], size: Double) -> UIFont {
        names.lazy.compactMap { UIFont(name: $0, size: size) }.first ?? .systemFont(ofSize: size, weight: .semibold)
    }

    private func color(_ value: DisplayColor) -> UIColor {
        UIColor(red: Double(value.red) / 255, green: Double(value.green) / 255,
                blue: Double(value.blue) / 255, alpha: Double(value.alpha) / 255)
    }
}

private enum Entry {
    case point(DisplayPointStyle)
    case shape(DisplayShapeStyle)

    var order: Int32 {
        switch self { case let .point(value): value.order; case let .shape(value): value.order }
    }

    var id: String {
        switch self { case let .point(value): "point:" + value.id; case let .shape(value): "shape:" + value.id }
    }
}
