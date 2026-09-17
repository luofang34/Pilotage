import UIKit

final class RouteWritingView: UITextView {
    static var height: CGFloat { max(132, RouteChipCell.height * 3) }
    var submit: (() -> Void)?
    var invalidInput: (() -> Void)?

    override init(frame: CGRect, textContainer: NSTextContainer?) {
        super.init(frame: frame, textContainer: textContainer)
        backgroundColor = .clear; textColor = .label
        textContainerInset = UIEdgeInsets(top: 10, left: 0, bottom: 10, right: 0)
        keyboardType = .asciiCapable; autocapitalizationType = .allCharacters
        autocorrectionType = .no; spellCheckingType = .no
        smartDashesType = .no; smartQuotesType = .no; returnKeyType = .go
        accessibilityIdentifier = "route-writing"; accessibilityLabel = "Edit route text"
    }
    @available(*, unavailable) required init?(coder: NSCoder) { nil }

    override func insertText(_ text: String) {
        if text == "\n" { submit?(); return }
        guard let normalized = RouteIdentifierInput.normalized(text) else { invalidInput?(); return }
        super.insertText(normalized)
    }
    override func replace(_ range: UITextRange, withText text: String) {
        guard let normalized = RouteIdentifierInput.normalized(text),
              self.text.utf8.count - (self.text(in: range) ?? "").utf8.count + normalized.utf8.count <= 4096 else {
            invalidInput?(); return
        }
        super.replace(range, withText: normalized)
    }
}

extension RouteFlowLayout.Coordinator: UITextViewDelegate, @MainActor UIIndirectScribbleInteractionDelegate {
    func installWriting(in view: RouteInputView) {
        self.view = view
        view.paragraph.delegate = self
        view.paragraph.submit = { [weak self] in self?.parent.submit() }
        view.paragraph.invalidInput = { [weak self] in self?.parent.invalidInput() }
        view.addInteraction(UIIndirectScribbleInteraction(delegate: self))
    }
    func updateWriting(in view: RouteInputView) {
        writingFocusID = parent.editingID.map { "waypoint:" + $0 }
        let wasWriting = view.paragraph.isFirstResponder
        let entering = parent.editsWholeRoute && !view.editsWholeRoute
        view.editsWholeRoute = parent.editsWholeRoute
        view.paragraph.isHidden = !parent.editsWholeRoute
        view.collection.isHidden = parent.editsWholeRoute; view.field.isHidden = parent.editsWholeRoute
        view.paragraph.font = RouteChipCell.font
        if parent.editsWholeRoute, view.paragraph.text != parent.text { view.paragraph.text = parent.text }
        if entering { view.paragraph.selectedRange = NSRange(location: view.paragraph.text.utf16.count, length: 0) }
        if wasWriting && !parent.editsWholeRoute { view.field.becomeFirstResponder() }
    }
    func textViewDidChange(_ textView: UITextView) { parent.text = textView.text }
    func textView(_ textView: UITextView, shouldChangeTextIn range: NSRange, replacementText text: String) -> Bool {
        if text == "\n" { parent.submit(); return false }
        guard let normalized = RouteIdentifierInput.normalized(text), let range = Range(range, in: textView.text),
              textView.text.replacingCharacters(in: range, with: normalized).utf8.count <= 4096 else {
            parent.invalidInput(); return false
        }
        return true
    }
    func indirectScribbleInteraction(_ interaction: UIInteraction, requestElementsIn rect: CGRect,
                                    completion: @escaping ([String]) -> Void) {
        completion(writingElements.filter { $0.frame.intersects(rect) }.map(\.id))
    }
    func indirectScribbleInteraction(_ interaction: UIInteraction, frameForElement elementIdentifier: String) -> CGRect {
        writingElements.first { $0.id == elementIdentifier }?.frame ?? .zero
    }
    func indirectScribbleInteraction(_ interaction: UIInteraction, isElementFocused elementIdentifier: String) -> Bool {
        if elementIdentifier == "route" { return view?.paragraph.isFirstResponder == true }
        return view?.field.isFirstResponder == true && elementIdentifier == (writingFocusID ?? "entry")
    }
    func indirectScribbleInteraction(_ interaction: UIInteraction, shouldDelayFocusForElement elementIdentifier: String) -> Bool {
        elementIdentifier != "entry" && elementIdentifier != "route"
    }
    func indirectScribbleInteraction(_ interaction: UIInteraction, focusElementIfNeeded elementIdentifier: String,
                                    referencePoint: CGPoint, completion: @escaping ((UIResponder & UITextInput)?) -> Void) {
        guard let view else { completion(nil); return }
        if elementIdentifier == "route", view.editsWholeRoute {
            if !view.paragraph.isFirstResponder {
                view.paragraph.layoutIfNeeded()
                let point = view.convert(referencePoint, to: view.paragraph)
                let position = view.paragraph.closestPosition(to: point) ?? view.paragraph.endOfDocument
                view.paragraph.becomeFirstResponder()
                view.paragraph.selectedTextRange = view.paragraph.textRange(from: position, to: position)
            }
            completion(view.paragraph); return
        }
        if view.field.isFirstResponder && elementIdentifier == (writingFocusID ?? "entry") {
            completion(view.field); return
        }
        if elementIdentifier != "entry" {
            guard let index = tokens.firstIndex(where: { "waypoint:" + $0.id == elementIdentifier }),
                  view.layout.frames.indices.contains(index) else { completion(nil); return }
            let token = tokens[index], frame = view.layout.frames[index]
            parent.select(token)
            view.field.text = RouteIdentifierInput.normalized(token.label) ?? ""
            view.field.frame = frame
        }
        writingFocusID = elementIdentifier
        view.field.becomeFirstResponder()
        view.field.layoutIfNeeded()
        let point = view.convert(referencePoint, to: view.field)
        let position = view.field.closestPosition(to: point) ?? view.field.endOfDocument
        view.field.selectedTextRange = view.field.textRange(from: position, to: position)
        completion(view.field)
    }

    private var writingElements: [(id: String, frame: CGRect)] {
        guard let view else { return [] }
        if view.editsWholeRoute { return [("route", view.paragraph.frame)] }
        let fieldID = writingFocusID ?? "entry"
        var elements = [(id: fieldID, frame: view.field.frame)]
        for (index, token) in tokens.enumerated() where view.layout.frames.indices.contains(index) {
            let id = "waypoint:" + token.id
            if id != fieldID { elements.append((id, view.layout.frames[index])) }
        }
        return elements
    }
}
