import XCTest

final class PairingFlowUITests: XCTestCase {
    @MainActor func testTheWelcomeOffersPairing() {
        let app = XCUIApplication()
        app.launchArguments = ["-scene", "welcome", "-static", "YES"]
        app.launch()
        XCTAssertTrue(app.buttons["Pair with Your Computer"].waitForExistence(timeout: 15))
        attach(app, "Welcome")
    }

    @MainActor func testThePairingPageOffersADemoAndReturnsWithoutPairing() {
        let app = XCUIApplication()
        app.launchArguments = ["-scene", "pair", "-static", "YES"]
        app.launch()
        let demo = app.buttons["tryDemo"]
        XCTAssertTrue(demo.waitForExistence(timeout: 15))
        XCTAssertTrue(demo.isHittable, "the demo is visible beside the pairing actions")
        attach(app, "Pairing with demo entry")
        demo.tap()

        XCTAssertTrue(app.staticTexts["Demo Mode"].waitForExistence(timeout: 5))
        XCTAssertTrue(app.staticTexts["Needs you"].exists)
        attach(app, "Demo home")
        app.staticTexts["Implement statistics page"].firstMatch.tap()
        XCTAssertTrue(app.buttons["Escape"].waitForExistence(timeout: 5))
        XCTAssertTrue(app.buttons["exitDemo"].isHittable, "the terminal also offers an exit")
        app.buttons["exitDemo"].tap()

        XCTAssertTrue(app.buttons["Scan Code"].waitForExistence(timeout: 5))
        XCTAssertTrue(app.buttons["Paste a Link Instead"].exists)
        XCTAssertFalse(app.tabBars.buttons["Agents"].exists, "sample Devices were not paired")

        demo.tap()
        XCTAssertTrue(app.staticTexts["Needs you"].waitForExistence(timeout: 5))
        app.tabBars.buttons["Devices"].tap()
        app.buttons["Pair a Computer"].tap()
        XCTAssertTrue(app.buttons["Scan Code"].waitForExistence(timeout: 5))
        XCTAssertTrue(app.buttons["Paste a Link Instead"].exists)
        XCTAssertFalse(app.staticTexts["Demo Mode"].exists)
        XCTAssertFalse(app.buttons["exitDemo"].exists)
        XCTAssertFalse(app.tabBars.buttons["Devices"].exists, "sample Devices were dismissed")
    }

    @MainActor func testAddingAComputerDoesNotOfferADemo() {
        let app = XCUIApplication()
        app.launchArguments = ["-scene", "devices", "-static", "YES"]
        app.launch()
        XCTAssertTrue(app.buttons["Pair a Computer"].waitForExistence(timeout: 15))
        app.buttons["Pair a Computer"].tap()
        XCTAssertTrue(app.buttons["Scan Code"].waitForExistence(timeout: 5))
        XCTAssertTrue(app.buttons["Paste a Link Instead"].exists)
        XCTAssertFalse(app.buttons["tryDemo"].exists)
        attach(app, "Additional pairing without demo")
        XCTAssertTrue(app.buttons["Cancel"].waitForExistence(timeout: 5))
        app.buttons["Cancel"].tap()
        XCTAssertTrue(app.buttons["Pair a Computer"].waitForExistence(timeout: 5))
        XCTAssertTrue(app.tabBars.buttons["Devices"].isSelected)
    }

    @MainActor private func attach(_ app: XCUIApplication, _ name: String) {
        let attachment = XCTAttachment(screenshot: app.screenshot())
        attachment.name = name
        attachment.lifetime = .keepAlways
        add(attachment)
    }

}
