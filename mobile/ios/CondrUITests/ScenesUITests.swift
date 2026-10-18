import XCTest

/// Every screen the app has, opened on the demo world through `-scene`, shows what it is
/// for; and the terminal page takes a reply.
final class ScenesUITests: XCTestCase {
    private func launch(_ scene: String, _ arguments: [String] = []) -> XCUIApplication {
        let app = XCUIApplication()
        app.launchArguments = ["-scene", scene] + arguments
        app.launch()
        return app
    }

    @MainActor private func assertLandmark(
        _ scene: String, _ query: (XCUIApplication) -> XCUIElement
    ) {
        let app = launch(scene)
        if !query(app).waitForExistence(timeout: 15) {
            let texts = app.staticTexts.allElementsBoundByIndex.map { $0.label }
            XCTFail("\(scene) shows its landmark; the screen says \(texts)")
        }
    }

    @MainActor func testTheWelcomeOffersPairing() {
        assertLandmark("welcome") { $0.buttons["Pair with Your Computer"] }
    }

    @MainActor func testTheInviteStepExplainsTheCode() {
        assertLandmark("pair") { $0.staticTexts["Show the pairing code on your computer"] }
    }

    @MainActor func testTheNameStepPreviewsThePairedRow() {
        assertLandmark("name") { $0.staticTexts["Name this iPhone"] }
    }

    @MainActor func testPairedShowsTheComputer() {
        assertLandmark("paired") { $0.staticTexts["Paired with macmini"] }
    }

    @MainActor func testTheHomeListsWhatNeedsYou() {
        let app = launch("devices")
        XCTAssertTrue(app.buttons["Pair a Computer"].waitForExistence(timeout: 15))
        app.tabBars.buttons["Agents"].tap()
        XCTAssertTrue(app.staticTexts["Needs you"].waitForExistence(timeout: 5))
        XCTAssertTrue(app.staticTexts["Implement statistics page"].exists)
    }

    @MainActor func testAnUnreachableComputerOffersARetry() {
        assertLandmark("offline") { $0.buttons["Try Again"] }
    }

    @MainActor func testTheLockAsksToUnlock() {
        assertLandmark("lock") { $0.staticTexts["Condr is locked"] }
    }

    @MainActor func testAnErrorCanBeDismissed() {
        let app = launch("error")
        let dismiss = app.buttons["Dismiss"]
        XCTAssertTrue(dismiss.waitForExistence(timeout: 15))
        dismiss.tap()
        XCTAssertFalse(dismiss.waitForExistence(timeout: 2))
    }

    @MainActor func testABannerOpensItsTerminal() {
        let app = launch("banner")
        let banner = app.buttons["banner"]
        XCTAssertTrue(banner.waitForExistence(timeout: 15))
        banner.tap()
        XCTAssertTrue(app.staticTexts["Migrate auth to passkeys"].waitForExistence(timeout: 10))
        XCTAssertTrue(app.buttons["Escape"].exists)
    }

    @MainActor func testTheTerminalTakesAReply() {
        let app = launch("terminal", ["-focus", "YES"])
        XCTAssertTrue(app.buttons["Escape"].waitForExistence(timeout: 15))
        app.buttons["Down"].tap()
        app.buttons["terminalReturn"].tap()
        let field = app.descendants(matching: .any).matching(identifier: "draft").firstMatch
        XCTAssertTrue(field.waitForExistence(timeout: 5))
        let send = app.buttons["sendReply"]
        XCTAssertFalse(send.isEnabled, "an empty draft has nothing to send")
        // Return in the field is a line break; the terminal's Return is on the key row.
        field.tap()
        field.typeText("Ship it\nplease")
        XCTAssertEqual(field.value as? String, "Ship it\nplease")
        XCTAssertTrue(send.isEnabled)
        // The terminal's Backspace, tapped and held, touches neither the draft nor Send.
        let backspace = app.buttons["terminalBackspace"]
        backspace.tap()
        backspace.coordinate(withNormalizedOffset: CGVector(dx: 0.5, dy: 0.9)).tap()
        backspace.press(forDuration: 1.2)
        XCTAssertEqual(field.value as? String, "Ship it\nplease", "Backspace left the draft alone")
        XCTAssertTrue(send.isEnabled)
        send.tap()
        expectation(for: NSPredicate(format: "isEnabled == NO"), evaluatedWith: send)
        waitForExpectations(timeout: 2)
        app.buttons["Hide Keyboard"].tap()
        XCTAssertFalse(
            app.buttons["Hide Keyboard"].waitForExistence(timeout: 2), "the keyboard went")
        app.navigationBars.buttons.firstMatch.tap()
        XCTAssertTrue(app.staticTexts["Needs you"].waitForExistence(timeout: 10))
    }

    @MainActor func testSettingsShowsTheKey() {
        assertLandmark("settings") { $0.staticTexts["Key"] }
    }

    @MainActor func testACrowdedComputerStillFitsItsScore() {
        assertLandmark("crowd-device") { $0.staticTexts["buildbox"] }
    }

    /// A long press selects the word under it; Copy from the menu puts it on the clipboard
    /// and says so (ADR 0041).
    @MainActor func testALongPressSelectsAndCopies() {
        let app = launch("terminal")
        let terminal = app.descendants(matching: .any)["terminal"]
        XCTAssertTrue(terminal.waitForExistence(timeout: 15))
        // "Bash" starts the first row, two cells in.
        terminal.coordinate(withNormalizedOffset: .zero).withOffset(CGVector(dx: 28, dy: 8))
            .press(forDuration: 1)
        let copy =
            app.menuItems["Copy"].waitForExistence(timeout: 3)
            ? app.menuItems["Copy"] : app.buttons["Copy"]
        XCTAssertTrue(copy.waitForExistence(timeout: 3), "the edit menu offers Copy")
        copy.tap()
        XCTAssertTrue(app.staticTexts["Copied to clipboard"].waitForExistence(timeout: 5))
    }

    /// The key row goes on past the arrows: Shift-Tab, Paste and the page keys (ADR 0041).
    @MainActor func testTheKeyRowHasTheKeysAgentsUse() {
        let app = launch("terminal")
        XCTAssertTrue(app.buttons["Escape"].waitForExistence(timeout: 15))
        for key in [
            "Tab", "Control", "Up", "Down", "Shift-Tab", "Home", "End", "Page Up", "Page Down",
            "Return",
        ] {
            XCTAssertTrue(app.buttons[key].exists, "the key row has \(key)")
        }
    }

    /// The app lock is on unless turned off, and turning it off asks first (ADR 0041).
    @MainActor func testTheAppLockCanBeTurnedOff() {
        let app = launch("settings")
        let toggle = app.switches["Lock with Face ID"]
        XCTAssertTrue(toggle.waitForExistence(timeout: 15))
        XCTAssertEqual(toggle.value as? String, "1")
        let control = toggle.switches.firstMatch.exists ? toggle.switches.firstMatch : toggle
        control.tap()
        XCTAssertTrue(
            app.staticTexts[
                "Anyone who can open this iPhone can open Condr and reach your computers."
            ]
            .waitForExistence(timeout: 5))
        XCTAssertEqual(toggle.value as? String, "0")
    }
}
