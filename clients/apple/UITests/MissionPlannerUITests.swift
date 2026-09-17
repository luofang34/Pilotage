import XCTest

final class MissionPlannerUITests: XCTestCase {
    @MainActor
    func testTypedRouteUsesCompactDraggableChipsAndReusableVehicleProfile() throws {
        continueAfterFailure = false
        let app = XCUIApplication()
        app.launchArguments = ["-OpenMap", "-OpenMission", "-MissionTestDraft", UUID().uuidString]
        app.launch()
        let entry = app.textFields["route-entry"]
        XCTAssertTrue(entry.waitForExistence(timeout: 30))
        entry.tap()
        entry.typeText("KTTN KEWR KJFK")
        XCTAssertEqual(entry.value as? String, "KTTN KEWR KJFK")
        app.buttons["Add route"].tap()
        let first = chip("KTTN", in: app), second = chip("KEWR", in: app), last = chip("KJFK", in: app)
        XCTAssertTrue(last.waitForExistence(timeout: 30))
        XCTAssertEqual(first.frame.midY, second.frame.midY, accuracy: 2)
        XCTAssertEqual(second.frame.midY, last.frame.midY, accuracy: 2)
        first.press(forDuration: 0.8, thenDragTo: last)
        let reordered = NSPredicate { _, _ in
            second.frame.minX < first.frame.minX && first.frame.minX < last.frame.minX
        }
        expectation(for: reordered, evaluatedWith: nil)
        waitForExpectations(timeout: 10)
        app.buttons["select-vehicle"].tap()
        XCTAssertTrue(app.buttons["New vehicle profile"].waitForExistence(timeout: 10))
        app.buttons["New vehicle profile"].tap()
        let name = app.textFields["Profile name"]
        XCTAssertTrue(name.waitForExistence(timeout: 10))
        name.tap()
        name.typeText("UI Cruise")
        let speed = app.textFields["Cruise true airspeed (kt)"]
        speed.tap()
        speed.typeText("120")
        XCTAssertEqual(speed.value as? String, "120")
        app.buttons["Save"].tap()
        XCTAssertTrue(app.buttons["select-vehicle"].waitForExistence(timeout: 10))
        XCTAssertTrue(app.buttons["select-vehicle"].label.contains("UI Cruise"))
        XCTAssertTrue(app.staticTexts["Still air"].exists)
        XCTAssertFalse(app.textFields["Ground speed (kt)"].exists)
        app.buttons["Details"].tap()
        XCTAssertTrue(app.staticTexts["UI Cruise · 120 KTAS"].waitForExistence(timeout: 10))
        let attachment = XCTAttachment(screenshot: app.screenshot())
        attachment.name = "Compact route and vehicle profile"
        attachment.lifetime = .keepAlways
        add(attachment)
        app.terminate()
        app.launch()
        XCTAssertTrue(chip("KJFK", in: app).waitForExistence(timeout: 30))
        XCTAssertLessThan(chip("KEWR", in: app).frame.minX, chip("KTTN", in: app).frame.minX)
        XCTAssertLessThan(chip("KTTN", in: app).frame.minX, chip("KJFK", in: app).frame.minX)
        XCTAssertTrue(app.buttons["select-vehicle"].label.contains("UI Cruise"))
    }

    @MainActor
    func testSearchAddsPublishedAirportAndShowsCoordinatedPlanner() throws {
        continueAfterFailure = false
        let app = XCUIApplication()
        app.launchArguments = ["-OpenMap", "-OpenMission", "-MissionTestDraft", UUID().uuidString]
        app.launch()
        let addWaypoint = app.buttons["Add waypoint"]
        XCTAssertTrue(addWaypoint.waitForExistence(timeout: 30))
        addWaypoint.tap()
        let search = app.searchFields.firstMatch
        XCTAssertTrue(search.waitForExistence(timeout: 10))
        search.tap()
        search.typeText("KTTN")
        let match = app.buttons.matching(NSPredicate(format: "label BEGINSWITH %@", "Add KTTN,")).firstMatch
        XCTAssertTrue(match.waitForExistence(timeout: 30))
        match.tap()
        XCTAssertTrue(chip("KTTN", in: app).waitForExistence(timeout: 10))
        app.buttons["Team & Timing"].tap()
        XCTAssertTrue(app.buttons["Add vehicle assignment"].waitForExistence(timeout: 10))
        app.buttons["Add vehicle assignment"].tap()
        XCTAssertTrue(app.staticTexts["Vehicle 2"].waitForExistence(timeout: 10))
        let attachment = XCTAttachment(screenshot: app.screenshot())
        attachment.name = "Coordinated planner"
        attachment.lifetime = .keepAlways
        add(attachment)
    }

    @MainActor
    func testRouteOpensItsInstalledApproachChart() throws {
        continueAfterFailure = false
        let app = XCUIApplication()
        app.launchArguments = ["-OpenMap", "-OpenMission", "-MissionTestDraft", UUID().uuidString]
        app.launch()
        let entry = app.textFields["route-entry"]
        XCTAssertTrue(entry.waitForExistence(timeout: 30))
        entry.tap(); entry.typeText("KEWR ARD ZUBAX COPIX KTTN\n")
        XCTAssertTrue(chip("KTTN", in: app).waitForExistence(timeout: 30), app.debugDescription)
        app.buttons["Show route on map"].tap()
        let procedures = app.buttons["mission-procedures"]
        let ready = NSPredicate(format: "enabled == true")
        expectation(for: ready, evaluatedWith: procedures)
        waitForExpectations(timeout: 30)
        procedures.tap()
        let chart = app.buttons["procedure-chart-KTTN-00982IL6.PDF"]
        XCTAssertTrue(chart.waitForExistence(timeout: 20))
        XCTAssertTrue(app.staticTexts["On this route"].exists)
        chart.tap()
        XCTAssertTrue(app.staticTexts["KTTN · 2609 · Current"].waitForExistence(timeout: 20))
        XCTAssertTrue(app.staticTexts["111.3"].waitForExistence(timeout: 10))
        let attachment = XCTAttachment(screenshot: app.screenshot())
        attachment.name = "Mission approach chart"
        attachment.lifetime = .keepAlways
        add(attachment)
    }

    @MainActor
    private func chip(_ identifier: String, in app: XCUIApplication) -> XCUIElement {
        app.descendants(matching: .any).matching(NSPredicate(format: "identifier BEGINSWITH 'route-chip-' AND label == %@", identifier)).firstMatch
    }
}
