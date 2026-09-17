import UIKit

final class RouteChipCell: UICollectionViewCell {
    private let label = UILabel()
    private let capsule = UIView()
    static var font: UIFont { .monospacedSystemFont(ofSize: UIFont.preferredFont(forTextStyle: .callout).pointSize, weight: .semibold) }
    static var height: CGFloat { max(44, ceil(font.lineHeight) + 20) }
    static func width(_ token: RouteToken) -> CGFloat {
        min(240, ceil((token.label as NSString).size(withAttributes: [.font: font]).width) + 24)
    }
    static func preview(in bounds: CGRect) -> UIDragPreviewParameters {
        let parameters = UIDragPreviewParameters()
        let capsule = bounds.insetBy(dx: 0, dy: 7)
        parameters.backgroundColor = .clear
        parameters.visiblePath = UIBezierPath(roundedRect: capsule, cornerRadius: capsule.height / 2)
        parameters.shadowPath = UIBezierPath()
        return parameters
    }
    override init(frame: CGRect) {
        super.init(frame: frame)
        contentView.backgroundColor = .clear; backgroundColor = .clear
        contentView.addSubview(capsule); capsule.addSubview(label)
        label.textAlignment = .center; label.lineBreakMode = .byTruncatingTail
        isAccessibilityElement = true; accessibilityTraits = .button
        accessibilityHint = "Tap to edit. Drag to reorder."
    }
    @available(*, unavailable) required init?(coder: NSCoder) { nil }
    override func layoutSubviews() {
        super.layoutSubviews()
        capsule.frame = contentView.bounds.insetBy(dx: 0, dy: 7)
        capsule.layer.cornerRadius = capsule.bounds.height / 2
        label.frame = capsule.bounds.insetBy(dx: 8, dy: 0)
    }
    func configure(_ token: RouteToken) {
        label.text = token.label; label.font = Self.font
        let color: UIColor = token.source == nil ? .secondaryLabel : token.point.kind == "airport" ? .systemBlue : .systemPurple
        label.textColor = color; capsule.backgroundColor = color.withAlphaComponent(0.15)
        accessibilityLabel = token.label; accessibilityIdentifier = "route-chip-" + token.id
    }
}
