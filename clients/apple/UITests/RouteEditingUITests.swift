import XCTest

final class RouteEditingUITests: XCTestCase {
    @MainActor
    func testInlineKeyboardEditsAndRejectsUnresolvedRoutes() throws {
        continueAfterFailure = false
        XCUIDevice.shared.orientation = .landscapeLeft
        defer { XCUIDevice.shared.orientation = .portrait }
        let app = launch()
        let entry = app.textFields["route-entry"]
        entry.tap()
        entry.typeText("kttn KEWR KJFK\n")
        XCTAssertTrue(chip("KJFK", in: app).waitForExistence(timeout: 30))
        app.buttons["Show route on map"].tap()
        XCTAssertEqual(entry.frame.midY, chip("KJFK", in: app).frame.midY, accuracy: 2)
        entry.tap()
        entry.typeText(XCUIKeyboardKey.delete.rawValue)
        XCTAssertFalse(chip("KJFK", in: app).exists)
        entry.typeText("KMGJ\n")
        XCTAssertTrue(chip("KMGJ", in: app).waitForExistence(timeout: 20))
        XCTAssertFalse(chip("KJFK", in: app).exists)
        chip("KEWR", in: app).tap()
        XCTAssertEqual(entry.value as? String, "KEWR")
        entry.typeText(String(repeating: XCUIKeyboardKey.delete.rawValue, count: 4) + "KJFK\n")
        XCTAssertTrue(chip("KJFK", in: app).waitForExistence(timeout: 20))
        XCTAssertLessThan(chip("KJFK", in: app).frame.minX, chip("KMGJ", in: app).frame.minX)
        entry.tap()
        entry.typeText("!")
        XCTAssertTrue(app.staticTexts["Use A–Z, 0–9, and spaces. Use search for names or coordinates."].waitForExistence(timeout: 5))
        XCTAssertFalse((entry.value as? String ?? "").contains("!"))
        entry.typeText("KEWR ZZZZZ\n")
        XCTAssertTrue(app.staticTexts["No exact match for ZZZZZ. Choose a match or change the identifier."].waitForExistence(timeout: 20))
        XCTAssertFalse(chip("KEWR", in: app).exists)
        XCTAssertEqual(chips(in: app).count, 3)
        app.buttons["Cancel edit"].tap()
        XCTAssertEqual(chips(in: app).count, 3)
    }

    @MainActor
    func testReturnContinuesEntryAndBackspaceDeletesRepeatedlyWithoutTaps() throws {
        continueAfterFailure = false
        let app = launch()
        let entry = app.textFields["route-entry"]
        entry.tap()
        entry.typeText("KTTN\nKEWR\nKJFK\n")
        XCTAssertTrue(chip("KJFK", in: app).waitForExistence(timeout: 30))
        XCTAssertEqual(chips(in: app).count, 3)
        entry.typeText(String(repeating: XCUIKeyboardKey.delete.rawValue, count: 2))
        XCTAssertEqual(chips(in: app).count, 1)
        XCTAssertTrue(chip("KTTN", in: app).exists)
        entry.typeText("KMGJ\n")
        XCTAssertTrue(chip("KMGJ", in: app).waitForExistence(timeout: 20))
        XCTAssertEqual(chips(in: app).count, 2)
    }

    @MainActor
    func testWholeRouteUsesNativeTextSelectionAndReturnsToContinuousEntry() throws {
        continueAfterFailure = false
        let app = launch()
        let entry = app.textFields["route-entry"]
        entry.tap(); entry.typeText("KTTN KEWR KJFK\n")
        XCTAssertTrue(chip("KJFK", in: app).waitForExistence(timeout: 30))
        app.buttons["Route actions"].tap()
        app.buttons["Edit route text"].tap()
        let writing = app.textViews["route-writing"]
        XCTAssertTrue(writing.waitForExistence(timeout: 10))
        XCTAssertEqual(writing.value as? String, "KTTN KEWR KJFK")
        writing.typeText(String(repeating: XCUIKeyboardKey.delete.rawValue, count: 9) + "KMGJ KEWR\n")
        XCTAssertTrue(chip("KMGJ", in: app).waitForExistence(timeout: 20))
        XCTAssertFalse(chip("KJFK", in: app).exists)
        XCTAssertEqual(chips(in: app).count, 3)
        entry.typeText("KJFK\n")
        XCTAssertTrue(chip("KJFK", in: app).waitForExistence(timeout: 20))
        XCTAssertEqual(chips(in: app).count, 4)
    }

    @MainActor
    func testFlightAmendmentCanUseNearbyMatchAndRequiresApply() throws {
        continueAfterFailure = false
        XCUIDevice.shared.orientation = .landscapeLeft
        defer { XCUIDevice.shared.orientation = .portrait }
        let app = launch()
        let entry = app.textFields["route-entry"]
        entry.tap()
        entry.typeText("KTTN KEWR KJFK\n")
        XCTAssertTrue(chip("KJFK", in: app).waitForExistence(timeout: 30))
        app.buttons["Show route on map"].tap()
        app.buttons["Navigate on this iPad"].tap()
        XCTAssertTrue(app.staticTexts["Current: KEWR"].waitForExistence(timeout: 10))
        chip("KJFK", in: app).tap()
        entry.typeText(String(repeating: XCUIKeyboardKey.delete.rawValue, count: 4) + "KMG")
        XCTAssertTrue(app.buttons["route-match-KMGJ"].waitForExistence(timeout: 20))
        XCTAssertTrue(app.staticTexts["Matches · distance from KJFK"].exists)
        app.buttons["route-match-KMGJ"].tap()
        XCTAssertTrue(app.navigationBars["Review route update"].waitForExistence(timeout: 10))
        app.buttons["Cancel"].tap()
        XCTAssertTrue(chip("KJFK", in: app).waitForExistence(timeout: 10))
        XCTAssertFalse(chip("KMGJ", in: app).exists)
        chip("KJFK", in: app).tap()
        entry.typeText(String(repeating: XCUIKeyboardKey.delete.rawValue, count: 4) + "KMGJ\n")
        XCTAssertTrue(app.buttons["Apply update"].waitForExistence(timeout: 15))
        app.buttons["Apply update"].tap()
        XCTAssertTrue(chip("KMGJ", in: app).waitForExistence(timeout: 10))
        XCTAssertTrue(app.staticTexts["Current: KEWR"].exists)
        XCTAssertFalse(chip("KJFK", in: app).exists)
        app.buttons["Mark reached"].tap()
        XCTAssertTrue(app.staticTexts["Current: KMGJ"].waitForExistence(timeout: 10))
        app.buttons["NavLog"].tap()
        XCTAssertTrue(app.staticTexts["Past"].waitForExistence(timeout: 10))
        XCTAssertTrue(app.staticTexts["Current"].exists)
    }

    @MainActor private func launch() -> XCUIApplication {
        let app = XCUIApplication()
        app.launchArguments = ["-OpenMap", "-OpenMission", "-MissionTestDraft", UUID().uuidString]
        app.launch()
        XCTAssertTrue(app.textFields["route-entry"].waitForExistence(timeout: 30))
        return app
    }
    @MainActor private func chips(in app: XCUIApplication) -> XCUIElementQuery {
        app.cells.matching(NSPredicate(format: "identifier BEGINSWITH 'route-chip-'"))
    }
    @MainActor private func chip(_ id: String, in app: XCUIApplication) -> XCUIElement {
        chips(in: app).matching(NSPredicate(format: "label == %@", id)).firstMatch
    }
}
