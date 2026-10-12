import Testing

@testable import Condr

struct BlockedOnTests {
    @Test func aPermissionSplitsIntoToolAndCommand() {
        let blocked = BlockedOn("Bash: rm -rf build")
        #expect(blocked.tool == "Bash")
        #expect(blocked.text == "rm -rf build")
    }

    @Test func aQuestionKeepsEverything() {
        let blocked = BlockedOn("Which option: A or B?")
        #expect(blocked.tool == nil)
        #expect(blocked.text == "Which option: A or B?")
    }
}
