import UIKit

final class RouteInputField: UITextField {
    var emptyBackspace: (() -> Void)?
    var invalidInput: (() -> Void)?
    override func deleteBackward() {
        if (text ?? "").isEmpty { emptyBackspace?() } else { super.deleteBackward() }
    }
    override func insertText(_ text: String) {
        if text == "\n" { _ = delegate?.textFieldShouldReturn?(self); return }
        guard let normalized = RouteIdentifierInput.normalized(text) else { invalidInput?(); return }
        super.insertText(normalized)
    }
    override func replace(_ range: UITextRange, withText text: String) {
        guard let normalized = RouteIdentifierInput.normalized(text),
              (self.text ?? "").utf8.count - (self.text(in: range) ?? "").utf8.count + normalized.utf8.count <= 4096 else {
            invalidInput?(); return
        }
        super.replace(range, withText: normalized)
    }
}

struct RouteInputGeometry {
    let items: [CGRect]
    let field: CGRect
    let height: CGFloat
    init(width: CGFloat, itemWidths: [CGFloat], insertion: Int, textWidth: CGFloat, rowHeight: CGFloat) {
        let width = width.isFinite ? max(1, width) : 400
        var x: CGFloat = 0, y: CGFloat = 0, frames: [CGRect] = [], entry = CGRect.zero
        for index in 0...itemWidths.count {
            if index == insertion {
                let minimum = min(width, max(100, textWidth + 20))
                if x > 0 && x + minimum > width { x = 0; y += rowHeight + 6 }
                let fieldWidth = index == itemWidths.count ? width - x : minimum
                entry = CGRect(x: x, y: y, width: max(1, fieldWidth), height: rowHeight)
                x += fieldWidth + 6
            }
            guard index < itemWidths.count else { continue }
            let itemWidth = min(width, itemWidths[index])
            if x > 0 && x + itemWidth > width { x = 0; y += rowHeight + 6 }
            frames.append(CGRect(x: x, y: y, width: itemWidth, height: rowHeight))
            x += itemWidth + 6
        }
        items = frames; field = entry; height = y + rowHeight
    }
}

final class RouteChipLayout: UICollectionViewLayout {
    var frames: [CGRect] = []
    var size = CGSize.zero
    override var collectionViewContentSize: CGSize { size }
    override func layoutAttributesForElements(in rect: CGRect) -> [UICollectionViewLayoutAttributes]? {
        frames.indices.compactMap { layoutAttributesForItem(at: IndexPath(item: $0, section: 0)) }.filter { $0.frame.intersects(rect) }
    }
    override func layoutAttributesForItem(at indexPath: IndexPath) -> UICollectionViewLayoutAttributes? {
        guard frames.indices.contains(indexPath.item) else { return nil }
        let attributes = UICollectionViewLayoutAttributes(forCellWith: indexPath)
        attributes.frame = frames[indexPath.item]
        return attributes
    }
}

final class RouteInputView: UIView {
    let layout = RouteChipLayout()
    lazy var collection = UICollectionView(frame: .zero, collectionViewLayout: layout)
    let field = RouteInputField()
    let paragraph = RouteWritingView()
    var editsWholeRoute = false
    var measure: ((CGFloat) -> RouteInputGeometry)?
    override init(frame: CGRect) {
        super.init(frame: frame); addSubview(collection); addSubview(field); addSubview(paragraph)
        paragraph.isHidden = true
    }
    @available(*, unavailable) required init?(coder: NSCoder) { nil }
    override func layoutSubviews() {
        super.layoutSubviews()
        paragraph.frame = bounds
        if editsWholeRoute { return }
        guard let geometry = measure?(bounds.width) else { return }
        collection.frame = bounds
        layout.frames = geometry.items; layout.size = bounds.size; layout.invalidateLayout()
        field.frame = geometry.field
    }
}
