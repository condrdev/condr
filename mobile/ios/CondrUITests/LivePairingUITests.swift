import XCTest

/// Against a real Server: `TEST_RUNNER_CONDR_INVITE=tcp://… TEST_RUNNER_CONDR_DEVICE_NAME=<its
/// hostname> xcodebuild test` pairs this simulator with it by pasting the invite, opens its
/// first Pane and types a command into it, then forgets it again. Skipped without an invite.
final class LivePairingUITests: XCTestCase {
    @MainActor func testPairingOpeningAPaneAndForgetting() throws {
        let environment = ProcessInfo.processInfo.environment
        let invite = try XCTUnwrap(
            environment["CONDR_INVITE"], "set TEST_RUNNER_CONDR_INVITE to run")
        let deviceName = environment["CONDR_DEVICE_NAME"] ?? ""
        let app = XCUIApplication()
        app.launch()

        // First launch pairs from the welcome; a paired app pairs another from Devices.
        let welcome = app.buttons["Pair with Your Computer"]
        if welcome.waitForExistence(timeout: 10) {
            welcome.tap()
        } else {
            app.tabBars.buttons["Devices"].tap()
            app.buttons["Pair a Computer"].tap()
        }
        app.buttons["Paste a Link Instead"].tap()
        UIPasteboard.general.string = invite
        let field =
            app.textViews.firstMatch.waitForExistence(timeout: 5)
            ? app.textViews.firstMatch : app.textFields.firstMatch
        field.tap()
        field.press(forDuration: 1.2)
        let paste = app.menuItems["Paste"]
        XCTAssertTrue(paste.waitForExistence(timeout: 5), "the edit menu offers Paste")
        paste.tap()
        let pasted = field.value as? String ?? ""
        XCTAssertTrue(pasted.contains("@"), "the link is in the field: \(pasted)")
        app.buttons["Continue"].tap()

        let name = app.textFields["Name"]
        XCTAssertTrue(name.waitForExistence(timeout: 5))
        if app.buttons["Clear"].exists { app.buttons["Clear"].tap() }
        name.typeText("Live Test iPhone")
        app.buttons["Continue"].tap()
        XCTAssertTrue(
            app.staticTexts["Connect directly on your Wi-Fi"].waitForExistence(timeout: 5))
        app.buttons["Continue"].tap()

        let paired = app.staticTexts.matching(
            NSPredicate(format: "label BEGINSWITH %@", "Paired with")
        )
        .firstMatch
        if !paired.waitForExistence(timeout: 30) {
            let texts = app.staticTexts.allElementsBoundByIndex.map { $0.label }
            let scalars = pasted.unicodeScalars.prefix(16).map { String($0.value, radix: 16) }
            XCTFail(
                "the handshake did not complete for \(pasted.debugDescription) \(scalars); the screen says \(texts)"
            )
            return
        }
        if !deviceName.isEmpty {
            XCTAssertTrue(
                paired.label.contains(deviceName), "named after the Server: \(paired.label)")
        }
        let done =
            app.buttons["Show My Agents"].exists
            ? app.buttons["Show My Agents"] : app.buttons["Done"]
        done.tap()

        app.tabBars.buttons["Devices"].tap()
        let device = app.buttons.matching(NSPredicate(format: "label CONTAINS %@", deviceName))
            .firstMatch
        XCTAssertTrue(device.waitForExistence(timeout: 10), "the Device is listed")
        device.tap()
        let pane = app.buttons.matching(NSPredicate(format: "label CONTAINS %@", "Shell"))
            .firstMatch
        XCTAssertTrue(pane.waitForExistence(timeout: 15), "the Workspace's Pane is listed")
        pane.tap()

        XCTAssertTrue(app.buttons["esc"].waitForExistence(timeout: 10))
        // The keyboard changes the grid, and the page says so for a moment.
        let input = app.textFields.firstMatch
        input.tap()
        XCTAssertTrue(
            app.descendants(matching: .any)
                .matching(NSPredicate(format: "label CONTAINS[c] %@", "fitted to this iPhone"))
                .firstMatch.waitForExistence(timeout: 10), "the phone sized the Pane")
        // What the shell sees of the phone's grid, then a word to find.
        input.typeText("tput cols")
        app.buttons["sendReply"].tap()
        input.typeText("echo CONDR_PHONE_OK")
        app.buttons["sendReply"].tap()
        sleep(2)

        app.navigationBars.buttons.firstMatch.tap()
        app.navigationBars.buttons["More"].firstMatch.tap()
        app.buttons.matching(NSPredicate(format: "label BEGINSWITH %@", "Forget")).firstMatch.tap()
        app.buttons.matching(NSPredicate(format: "label BEGINSWITH %@", "Forget")).firstMatch.tap()
        XCTAssertTrue(
            app.buttons["Pair a Computer"].waitForExistence(timeout: 10)
                || app.buttons["Pair with Your Computer"].waitForExistence(timeout: 10),
            "the Device is gone")
    }
}
