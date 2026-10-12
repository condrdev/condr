import XCTest

final class PairingFlowUITests: XCTestCase {
    @MainActor func testTheWelcomeOffersPairing() {
        let app = XCUIApplication()
        app.launchArguments = ["-scene", "welcome", "-static", "YES"]
        app.launch()
        XCTAssertTrue(app.buttons["Pair with Your Computer"].waitForExistence(timeout: 15))
    }
}
