import SwiftUI
import UIKit
import XCTest
@testable import Pilotage

@MainActor
final class RouteEntryTests: XCTestCase {
    func testIdentifiersAllowOnlyASCIIWordsAndSeparators() {
        XCTAssertEqual(RouteIdentifierInput.normalized("kttn\t12n\r\nDCT kewr"), "KTTN 12N  DCT KEWR")
        for input in ["KJ!FK", "KÉWR", "ＫＴＴＮ", "KTTN🙂", "40.2,-74.0", "A/B", "A\u{202e}B"] {
            XCTAssertNil(RouteIdentifierInput.normalized(input), input)
        }
        let field = RouteInputField()
        var rejected = false, emptyBackspace = false
        field.invalidInput = { rejected = true }
        field.emptyBackspace = { emptyBackspace = true }
        field.insertText("kttn")
        XCTAssertEqual(field.text, "KTTN")
        field.insertText("💥")
        XCTAssertTrue(rejected)
        XCTAssertEqual(field.text, "KTTN")
        field.deleteBackward()
        XCTAssertEqual(field.text, "KTT")
        field.text = ""
        field.deleteBackward()
        XCTAssertTrue(emptyBackspace)
    }

    func testCaretSharesTheChipRowAndWrapsWithoutOverlap() {
        let wide = RouteInputGeometry(width: 700, itemWidths: [65, 65, 65], insertion: 3, textWidth: 0, rowHeight: 44)
        XCTAssertEqual(wide.height, 44)
        XCTAssertEqual(wide.field.midY, wide.items[2].midY)
        XCTAssertGreaterThan(wide.field.minX, wide.items[2].maxX)
        let narrow = RouteInputGeometry(width: 200, itemWidths: [65, 65, 65], insertion: 1, textWidth: 80, rowHeight: 72)
        for frame in narrow.items { XCTAssertFalse(frame.intersects(narrow.field)); XCTAssertLessThanOrEqual(frame.maxX, 200) }
        XCTAssertGreaterThan(narrow.height, 72)
        let unknown = RouteInputGeometry(width: .infinity, itemWidths: [65], insertion: 0, textWidth: 0, rowHeight: 44)
        XCTAssertTrue(unknown.field.width.isFinite)
    }

    func testLiftAndDropPreviewExcludeTheRectangleAndShadow() throws {
        let preview = RouteChipCell.preview(in: CGRect(x: 0, y: 0, width: 70, height: 44))
        let path = try XCTUnwrap(preview.visiblePath)
        XCTAssertTrue(path.contains(CGPoint(x: 35, y: 22)))
        XCTAssertFalse(path.contains(CGPoint(x: 1, y: 1)))
        XCTAssertFalse(path.contains(CGPoint(x: 1, y: 8)))
        XCTAssertEqual(preview.backgroundColor, .clear)
        XCTAssertTrue(try XCTUnwrap(preview.shadowPath).isEmpty)
    }

    func testBackspaceDeletesWholeWaypointsAndKeepsInsertionPosition() throws {
        var tokens = [token("KEWR"), token("KTTN"), token("KJFK")]
        var draft = RouteEntryDraft()
        for labels in [["KEWR", "KTTN"], ["KEWR"], []] {
            let deletion = try XCTUnwrap(draft.deletingBackward(in: tokens))
            tokens = deletion.tokens
            draft.reset(); draft.beforeID = deletion.beforeID
            XCTAssertEqual(tokens.map(\.label), labels)
            XCTAssertEqual(draft.text, "")
        }
        XCTAssertNil(draft.deletingBackward(in: tokens))
        tokens = [token("KEWR"), token("KTTN"), token("KJFK")]
        draft.insert(before: tokens[2].id)
        let deletion = try XCTUnwrap(draft.deletingBackward(in: tokens))
        draft.reset(); draft.beforeID = deletion.beforeID
        XCTAssertEqual(draft.applying([token("KMGJ")], to: deletion.tokens)?.map(\.label), ["KEWR", "KMGJ", "KJFK"])
    }

    func testMiddleReplacementPreservesRouteIdentityUntilCommit() throws {
        let original = [token("KEWR"), token("KTTN"), token("KJFK")]
        var draft = RouteEntryDraft()
        draft.edit(original[1])
        let changed = try XCTUnwrap(draft.applying([token("KMGJ")], to: original))
        XCTAssertEqual(changed.map(\.label), ["KEWR", "KMGJ", "KJFK"])
        XCTAssertEqual(changed[1].id, original[1].id)
        draft.insert(before: original[1].id)
        XCTAssertEqual(draft.applying([token("ARD")], to: original)?.map(\.label), ["KEWR", "ARD", "KTTN", "KJFK"])
        XCTAssertNil(draft.applying([token("ARD")], to: [original[0], original[2]]))
    }

    func testTypingDuringLookupRetainsTheFollowingIdentifiers() {
        var submitted = RouteEntryDraft(); submitted.text = "KTTN "
        var continued = submitted; continued.text += "KEWR KJFK "
        XCTAssertEqual(continued.remainder(after: submitted), "KEWR KJFK ")
        continued.text = "KMGJ "
        XCTAssertNil(continued.remainder(after: submitted))
        continued = submitted; continued.beforeID = "another-position"
        XCTAssertNil(continued.remainder(after: submitted))
        submitted.text = "KTTN"; continued = submitted; continued.text += "2"
        XCTAssertNil(continued.remainder(after: submitted))
    }

    func testWholeRouteEditRetainsRepeatedOccurrencesAndRejectsAStaleDraft() throws {
        let original = [token("KEWR"), token("KTTN"), token("KEWR")]
        var draft = RouteEntryDraft(); draft.editRoute(original)
        XCTAssertEqual(draft.text, "KEWR KTTN KEWR")
        let result = try XCTUnwrap(draft.applying([token("KEWR"), token("KMGJ"), token("KEWR")], to: original))
        XCTAssertEqual(result.first?.id, original.first?.id)
        XCTAssertEqual(result.last?.id, original.last?.id)
        XCTAssertNil(draft.applying(result, to: Array(original.dropLast())))
        var changed = draft; changed.text += " KJFK"
        XCTAssertNil(changed.remainder(after: draft))
    }

    func testNativeWritingSupportsSelectionReplacementAndMultipleIdentifiers() throws {
        let view = RouteWritingView()
        view.insertText("kttn kewr kjfk")
        XCTAssertEqual(view.text, "KTTN KEWR KJFK")
        let start = try XCTUnwrap(view.position(from: view.beginningOfDocument, offset: 5))
        let end = try XCTUnwrap(view.position(from: start, offset: 9))
        let range = try XCTUnwrap(view.textRange(from: start, to: end))
        view.replace(range, withText: "kmgj\n12n")
        XCTAssertEqual(view.text, "KTTN KMGJ 12N")
        var rejected = false
        view.invalidInput = { rejected = true }
        view.insertText("!")
        XCTAssertTrue(rejected)
        XCTAssertEqual(view.text, "KTTN KMGJ 12N")
        view.selectedRange = NSRange(location: 0, length: view.text.utf16.count)
        view.deleteBackward()
        XCTAssertEqual(view.text, "")
    }

    func testScribbleTargetsTheEntryAndEachWaypointSeparately() throws {
        let tokens = [token("KTTN"), token("KEWR"), token("KJFK")]
        var text = "", selected: RouteToken?
        let flow = RouteFlowLayout(tokens: tokens, text: Binding(get: { text }, set: { text = $0 }),
            insertion: 3, focusRequest: 0, endEditingRequest: 0, allowsReorder: true,
            reorder: { _ in true }, select: { selected = $0 }, backspace: {}, submit: {}, invalidInput: {})
        let coordinator = flow.makeCoordinator()
        let view = RouteInputView(frame: CGRect(x: 30, y: 80, width: 700, height: 44))
        let geometry = RouteInputGeometry(width: 700, itemWidths: tokens.map(RouteChipCell.width),
            insertion: 3, textWidth: 0, rowHeight: 44)
        view.measure = { _ in geometry }; view.layoutSubviews()
        coordinator.installWriting(in: view)
        let interaction = try XCTUnwrap(view.interactions.first)
        let tail = CGRect(x: geometry.field.midX, y: geometry.field.midY, width: 1, height: 1)
        coordinator.indirectScribbleInteraction(interaction, requestElementsIn: tail) { XCTAssertEqual($0, ["entry"]) }
        coordinator.indirectScribbleInteraction(interaction, focusElementIfNeeded: "entry", referencePoint: tail.origin) {
            XCTAssertTrue($0 === view.field)
        }
        XCTAssertNil(selected)
        view.field.insertText("KMGJ 12N")
        XCTAssertEqual(view.field.text, "KMGJ 12N")
        XCTAssertFalse(view.editsWholeRoute)
        let middle = geometry.items[1]
        let middleID = "waypoint:" + tokens[1].id
        coordinator.indirectScribbleInteraction(interaction, requestElementsIn: middle.insetBy(dx: 1, dy: 1)) {
            XCTAssertEqual($0, [middleID])
        }
        coordinator.indirectScribbleInteraction(interaction, focusElementIfNeeded: middleID,
            referencePoint: CGPoint(x: middle.minX, y: middle.midY)) { XCTAssertTrue($0 === view.field) }
        XCTAssertEqual(selected?.id, tokens[1].id)
        XCTAssertEqual(view.field.text, "KEWR")
        XCTAssertEqual(view.field.frame, middle)
        XCTAssertFalse(view.editsWholeRoute)
        coordinator.parent.editingID = tokens[1].id
        coordinator.tokens = [tokens[0], tokens[2]]
        coordinator.updateWriting(in: view)
        let reflowed = RouteInputGeometry(width: 200, itemWidths: [65, 65], insertion: 1,
            textWidth: 65, rowHeight: 44)
        view.measure = { _ in reflowed }; view.layoutSubviews()
        XCTAssertEqual(coordinator.indirectScribbleInteraction(interaction, frameForElement: middleID), reflowed.field)
        coordinator.indirectScribbleInteraction(interaction, requestElementsIn: reflowed.field) {
            XCTAssertEqual($0, [middleID])
        }
        coordinator.parent.editingID = tokens[2].id
        coordinator.tokens = [tokens[0], tokens[1]]
        coordinator.updateWriting(in: view)
        coordinator.indirectScribbleInteraction(interaction, requestElementsIn: reflowed.field) {
            XCTAssertEqual($0, ["waypoint:" + tokens[2].id])
        }
    }

    func testScribbleReferencePointUsesTheEntryPositionAfterWrappedBadges() throws {
        let tokens = [token("KTTN"), token("KEWR"), token("KJFK")]
        var text = "KMGJ"
        let flow = RouteFlowLayout(tokens: tokens, text: Binding(get: { text }, set: { text = $0 }),
            insertion: 3, focusRequest: 0, endEditingRequest: 0, allowsReorder: true,
            reorder: { _ in true }, select: { _ in XCTFail("The entry must not select a badge") },
            backspace: {}, submit: {}, invalidInput: {})
        let coordinator = flow.makeCoordinator()
        let view = RouteInputView(frame: CGRect(x: 30, y: 80, width: 200, height: 160))
        let geometry = RouteInputGeometry(width: 200, itemWidths: [65, 65, 65],
            insertion: 3, textWidth: 65, rowHeight: 44)
        view.measure = { _ in geometry }; view.layoutSubviews()
        view.field.font = RouteChipCell.font; view.field.text = text
        coordinator.installWriting(in: view)
        let interaction = try XCTUnwrap(view.interactions.first)
        XCTAssertGreaterThan(view.field.frame.minY, 0)
        coordinator.indirectScribbleInteraction(interaction, focusElementIfNeeded: "entry",
            referencePoint: CGPoint(x: view.field.frame.maxX - 1, y: view.field.frame.midY)) {
                XCTAssertTrue($0 === view.field)
            }
        let selection = try XCTUnwrap(view.field.selectedTextRange)
        XCTAssertEqual(view.field.offset(from: view.field.beginningOfDocument, to: selection.start), 4)
        view.field.insertText(" 12N")
        XCTAssertEqual(view.field.text, "KMGJ 12N")
    }

    func testScribbleKeepsAnExistingTextSelection() throws {
        let scene = try XCTUnwrap(UIApplication.shared.connectedScenes.compactMap { $0 as? UIWindowScene }.first)
        let window = UIWindow(windowScene: scene)
        window.rootViewController = UIViewController()
        window.makeKeyAndVisible()
        defer { window.isHidden = true }
        var text = "KTTN KEWR KJFK"
        let flow = RouteFlowLayout(tokens: [], text: Binding(get: { text }, set: { text = $0 }),
            insertion: 0, focusRequest: 0, endEditingRequest: 0, allowsReorder: true,
            reorder: { _ in true }, select: { _ in XCTFail("The entry must not select a badge") },
            backspace: {}, submit: {}, invalidInput: {})
        let coordinator = flow.makeCoordinator()
        let view = RouteInputView(frame: CGRect(x: 30, y: 80, width: 700, height: 44))
        view.field.frame = view.bounds; view.field.text = text
        window.rootViewController?.view.addSubview(view)
        coordinator.installWriting(in: view)
        XCTAssertTrue(view.field.becomeFirstResponder())
        let start = try XCTUnwrap(view.field.position(from: view.field.beginningOfDocument, offset: 5))
        let end = try XCTUnwrap(view.field.position(from: start, offset: 9))
        view.field.selectedTextRange = view.field.textRange(from: start, to: end)
        let interaction = try XCTUnwrap(view.interactions.first)
        coordinator.indirectScribbleInteraction(interaction, focusElementIfNeeded: "entry",
            referencePoint: CGPoint(x: 0, y: 0)) { XCTAssertTrue($0 === view.field) }
        let selection = try XCTUnwrap(view.field.selectedTextRange)
        XCTAssertEqual(view.field.offset(from: view.field.beginningOfDocument, to: selection.start), 5)
        XCTAssertEqual(view.field.offset(from: selection.start, to: selection.end), 9)
        view.field.replace(selection, withText: "KMGJ 12N")
        XCTAssertEqual(view.field.text, "KTTN KMGJ 12N")
    }

    func testAmendmentKeepsCurrentRouteUntilAppliedAndCanBeUndone() async throws {
        let directory = FileManager.default.temporaryDirectory.appendingPathComponent(UUID().uuidString)
        defer { try? FileManager.default.removeItem(at: directory) }
        let plan = MissionPlanModel()
        await plan.start(url: directory.appendingPathComponent("mission.json"))
        let original = [token("KEWR"), token("KTTN"), token("KJFK")]
        plan.proposeRoute(original)
        plan.setCurrentLeg(to: original[1].id)
        let changed = [original[0], original[1], token("KMGJ")]
        plan.proposeRoute(changed)
        XCTAssertEqual(plan.tokens, original)
        XCTAssertEqual(plan.currentProgress?.destination?.id, original[1].id)
        let proposal = try XCTUnwrap(plan.routeUpdate)
        plan.routeUpdate = nil
        XCTAssertEqual(plan.tokens, original)
        plan.applyRoute(proposal, destinationID: original[1].id)
        XCTAssertEqual(plan.tokens, changed)
        XCTAssertEqual(plan.currentProgress?.destination?.id, original[1].id)
        plan.undoRouteEdit()
        XCTAssertEqual(plan.routeUpdate?.after, original)
        XCTAssertEqual(plan.tokens, changed)
        await plan.flush()
    }

    func testNearbyOrderDoesNotReplaceAnExactIdentifier() {
        func result(_ id: String, _ longitude: Double) -> NavigationMatch {
            NavigationMatch(point: NavigationPoint(key: id, identifier: id, kind: "airport", name: "", region: "",
                latitudeDeg: 40, longitudeDeg: longitude), source: NavigationSource(releaseId: id, authority: "test", edition: "test",
                    sourceDigest: String(repeating: "a", count: 64), effectiveAt: 0, expiresAt: Int64.max))
        }
        let near = result("KABC", -74), far = result("KABD", -124), exact = result("KAB", -100)
        let sorted = NavigationMatchOrder.sorted([far, exact, near], query: "KAB", near: near.point)
        XCTAssertEqual(sorted.map(\.point.identifier), ["KAB", "KABC", "KABD"])
    }

    func testOverviewFramesTheShortArcAcrossTheDateLine() throws {
        let overview = try XCTUnwrap(RouteOverview([
            CLLocationCoordinate2D(latitude: 40, longitude: 179), CLLocationCoordinate2D(latitude: 41, longitude: -179)
        ]))
        XCTAssertEqual(abs(overview.center.longitude), 180, accuracy: 0.001)
        XCTAssertLessThan(overview.radiusMeters, 200_000)
        XCTAssertNil(RouteOverview([]))
    }

    private func token(_ id: String) -> RouteToken {
        RouteToken(id: UUID().uuidString, point: NavigationPoint(key: id, identifier: id, kind: "airport", name: "", region: "",
            latitudeDeg: 40, longitudeDeg: -74), source: nil)
    }
}
