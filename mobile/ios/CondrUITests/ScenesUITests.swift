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
        XCTAssertTrue(app.buttons["esc"].exists)
    }

    @MainActor func testTheTerminalTakesAReply() {
        let app = launch("terminal", ["-focus", "YES"])
        XCTAssertTrue(app.buttons["esc"].waitForExistence(timeout: 15))
        app.buttons["Down"].tap()
        app.buttons["Return"].tap()
        let field = app.textFields.firstMatch
        XCTAssertTrue(field.waitForExistence(timeout: 5))
        field.typeText("Ship it")
        XCTAssertTrue(app.buttons["sendReply"].waitForExistence(timeout: 3))
        app.buttons["sendReply"].tap()
        XCTAssertFalse(
            app.buttons["sendReply"].waitForExistence(timeout: 2), "the reply left the field")
        app.navigationBars.buttons.firstMatch.tap()
        XCTAssertTrue(app.staticTexts["Needs you"].waitForExistence(timeout: 10))
    }

    @MainActor func testSettingsShowsTheKey() {
        assertLandmark("settings") { $0.staticTexts["Key"] }
    }

    @MainActor func testACrowdedComputerStillFitsItsScore() {
        assertLandmark("crowd-device") { $0.staticTexts["buildbox"] }
    }
}
