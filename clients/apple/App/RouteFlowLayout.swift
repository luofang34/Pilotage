import SwiftUI
import UIKit

struct RouteFlowLayout: UIViewRepresentable {
    let tokens: [RouteToken]
    @Binding var text: String
    let insertion: Int
    let focusRequest: UInt64
    let endEditingRequest: UInt64
    let allowsReorder: Bool
    let reorder: ([RouteToken]) -> Bool
    let select: (RouteToken) -> Void
    let backspace: () -> Void
    let submit: () -> Void
    let invalidInput: () -> Void
    var editsWholeRoute: Bool = false
    var editingID: String? = nil

    func makeCoordinator() -> Coordinator { Coordinator(self) }
    func makeUIView(context: Context) -> RouteInputView {
        let view = RouteInputView()
        view.collection.backgroundColor = .clear
        view.collection.isScrollEnabled = false
        view.collection.dragInteractionEnabled = true
        view.collection.register(RouteChipCell.self, forCellWithReuseIdentifier: "waypoint")
        view.collection.dataSource = context.coordinator; view.collection.delegate = context.coordinator
        view.collection.dragDelegate = context.coordinator; view.collection.dropDelegate = context.coordinator
        view.field.delegate = context.coordinator
        view.field.keyboardType = .asciiCapable; view.field.autocorrectionType = .no; view.field.spellCheckingType = .no
        view.field.autocapitalizationType = .allCharacters; view.field.smartDashesType = .no; view.field.smartQuotesType = .no
        view.field.returnKeyType = .go; view.field.placeholder = "Type route"
        view.field.accessibilityIdentifier = "route-entry"; view.field.accessibilityLabel = "Route identifiers"
        view.field.emptyBackspace = { [weak coordinator = context.coordinator] in coordinator?.parent.backspace() }
        view.field.invalidInput = { [weak coordinator = context.coordinator] in coordinator?.parent.invalidInput() }
        view.field.addTarget(context.coordinator, action: #selector(Coordinator.textChanged), for: .editingChanged)
        context.coordinator.installWriting(in: view)
        return view
    }
    func updateUIView(_ view: RouteInputView, context: Context) {
        let coordinator = context.coordinator
        coordinator.parent = self
        coordinator.updateWriting(in: view)
        view.field.font = RouteChipCell.font; view.measure = { geometry(width: $0) }
        if view.field.text != text { view.field.text = text }
        if coordinator.tokens != tokens || coordinator.typeSize != context.environment.dynamicTypeSize {
            coordinator.tokens = tokens; coordinator.typeSize = context.environment.dynamicTypeSize
            view.collection.reloadData()
        }
        view.setNeedsLayout()
        if coordinator.lastFocus != focusRequest {
            coordinator.lastFocus = focusRequest
            if editsWholeRoute { view.paragraph.becomeFirstResponder() } else { view.field.becomeFirstResponder() }
        }
        if coordinator.lastEndEditing != endEditingRequest {
            coordinator.lastEndEditing = endEditingRequest; view.endEditing(true)
        }
    }
    func sizeThatFits(_ proposal: ProposedViewSize, uiView: RouteInputView, context: Context) -> CGSize? {
        let width = proposal.width.flatMap { $0.isFinite ? max(1, $0) : nil } ?? 400
        return CGSize(width: width, height: editsWholeRoute ? RouteWritingView.height : geometry(width: width).height)
    }
    private func geometry(width: CGFloat) -> RouteInputGeometry {
        RouteInputGeometry(width: width, itemWidths: tokens.map(RouteChipCell.width), insertion: min(tokens.count, insertion),
            textWidth: (text as NSString).size(withAttributes: [.font: RouteChipCell.font]).width, rowHeight: RouteChipCell.height)
    }

    final class Coordinator: NSObject, UICollectionViewDataSource, UICollectionViewDelegate,
                             UICollectionViewDragDelegate, UICollectionViewDropDelegate, UITextFieldDelegate {
        var parent: RouteFlowLayout
        var tokens: [RouteToken]
        var typeSize: DynamicTypeSize?
        var lastFocus: UInt64 = 0
        var lastEndEditing: UInt64 = 0
        weak var view: RouteInputView?
        var writingFocusID: String?
        init(_ parent: RouteFlowLayout) { self.parent = parent; tokens = parent.tokens }
        @objc func textChanged(_ field: UITextField) { parent.text = field.text ?? "" }
        func textFieldShouldReturn(_ textField: UITextField) -> Bool {
            // A separator keeps subsequent keystrokes distinct while the lookup is in flight.
            if let text = textField.text, !text.isEmpty, text.last?.isWhitespace == false {
                textField.text = text + " "; textChanged(textField)
            }
            parent.submit(); return false
        }
        func textField(_ field: UITextField, shouldChangeCharactersIn range: NSRange, replacementString string: String) -> Bool {
            guard let normalized = RouteIdentifierInput.normalized(string), let replacementRange = Range(range, in: field.text ?? ""),
                  (field.text ?? "").replacingCharacters(in: replacementRange, with: normalized).utf8.count <= 4096 else {
                parent.invalidInput(); return false
            }
            if normalized != string, let start = field.position(from: field.beginningOfDocument, offset: range.location),
               let end = field.position(from: start, offset: range.length), let selected = field.textRange(from: start, to: end) {
                field.replace(selected, withText: normalized); textChanged(field); return false
            }
            return true
        }
        func collectionView(_ collectionView: UICollectionView, numberOfItemsInSection section: Int) -> Int { tokens.count }
        func collectionView(_ collectionView: UICollectionView, cellForItemAt indexPath: IndexPath) -> UICollectionViewCell {
            let cell = collectionView.dequeueReusableCell(withReuseIdentifier: "waypoint", for: indexPath)
            (cell as? RouteChipCell)?.configure(tokens[indexPath.item]); return cell
        }
        func collectionView(_ collectionView: UICollectionView, didSelectItemAt indexPath: IndexPath) { parent.select(tokens[indexPath.item]) }
        func collectionView(_ collectionView: UICollectionView, itemsForBeginning session: UIDragSession, at indexPath: IndexPath) -> [UIDragItem] {
            guard parent.allowsReorder else { return [] }
            let item = UIDragItem(itemProvider: NSItemProvider(object: tokens[indexPath.item].id as NSString))
            item.localObject = tokens[indexPath.item].id; session.localContext = self; return [item]
        }
        func collectionView(_ collectionView: UICollectionView, dragPreviewParametersForItemAt indexPath: IndexPath) -> UIDragPreviewParameters? {
            collectionView.cellForItem(at: indexPath).map { RouteChipCell.preview(in: $0.bounds) }
        }
        func collectionView(_ collectionView: UICollectionView, dropPreviewParametersForItemAt indexPath: IndexPath) -> UIDragPreviewParameters? {
            collectionView.cellForItem(at: indexPath).map { RouteChipCell.preview(in: $0.bounds) }
        }
        func collectionView(_ collectionView: UICollectionView, dropSessionDidUpdate session: UIDropSession,
                            withDestinationIndexPath destinationIndexPath: IndexPath?) -> UICollectionViewDropProposal {
            guard session.localDragSession?.localContext as? Coordinator === self else { return UICollectionViewDropProposal(operation: .forbidden) }
            return UICollectionViewDropProposal(operation: .move, intent: .insertAtDestinationIndexPath)
        }
        func collectionView(_ collectionView: UICollectionView, performDropWith coordinator: UICollectionViewDropCoordinator) {
            guard coordinator.session.localDragSession?.localContext as? Coordinator === self,
                  let item = coordinator.items.first, let id = item.dragItem.localObject as? String,
                  let from = tokens.firstIndex(where: { $0.id == id }) else { return }
            let to = min(tokens.count - 1, coordinator.destinationIndexPath?.item ?? tokens.count - 1)
            var updated = tokens
            updated.insert(updated.remove(at: from), at: to)
            let applied = parent.reorder(updated)
            if applied {
                collectionView.performBatchUpdates {
                    tokens = updated
                    collectionView.moveItem(at: IndexPath(item: from, section: 0), to: IndexPath(item: to, section: 0))
                }
            }
            coordinator.drop(item.dragItem, toItemAt: IndexPath(item: applied ? to : from, section: 0))
        }
    }
}
