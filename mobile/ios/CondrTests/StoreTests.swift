import CondrKit
import Foundation
import Testing

@testable import Condr

struct RouterTests {
    @Test func openingAPaneLandsOnTheAgentsTab() {
        let router = Router()
        router.tab = .devices
        router.open(Fixture.ref)
        #expect(router.tab == .agents)
        #expect(router.agentsPath == [.pane(Fixture.ref)])
        #expect(!router.showsHome)
    }

    @Test func forgettingADeviceDropsEveryRouteIntoIt() {
        let router = Router()
        let other = PaneRef(address: "p2p://other", pane: 1)
        router.agentsPath = [.device(Fixture.address), .pane(Fixture.ref)]
        router.devicesPath = [.device("p2p://other"), .pane(other)]
        router.forget(Fixture.address)
        #expect(router.agentsPath.isEmpty)
        #expect(router.devicesPath == [.device("p2p://other"), .pane(other)])
    }
}

struct AppLockTests {
    struct Owner: Authenticator {
        var available = true
        var promptsOnAppear = true
        var answer = true
        func authenticate() async -> Bool { answer }
    }

    @Test func aPairedPhoneStartsLockedAndTheOwnerOpensIt() async {
        let lock = AppLock(authenticator: Owner(), paired: true)
        #expect(lock.locked)
        await lock.unlock()
        #expect(!lock.locked)
    }

    @Test func fiveMinutesAwayLocksAgain() {
        var now = Date(timeIntervalSinceReferenceDate: 0)
        let lock = AppLock(authenticator: Owner(), paired: true, now: { now })
        lock.sceneBecameActive(paired: true)
        let unlocked = AppLock(authenticator: Owner(), paired: false, now: { now })
        #expect(!unlocked.locked)

        unlocked.sceneLeft()
        now += 100
        unlocked.sceneBecameActive(paired: true)
        #expect(!unlocked.locked, "a short absence keeps it open")

        unlocked.sceneLeft()
        now += AppLock.grace + 1
        unlocked.sceneBecameActive(paired: true)
        #expect(unlocked.locked)
        _ = lock
    }

    @Test func withoutAPasscodeThereIsNoLock() {
        let lock = AppLock(authenticator: Owner(available: false), paired: true)
        #expect(!lock.locked)
        lock.lock()
        #expect(!lock.locked)
    }

    @Test func aRefusedOwnerStaysLocked() async {
        let lock = AppLock(authenticator: Owner(answer: false), paired: true)
        await lock.unlock()
        #expect(lock.locked)
    }
}

struct PreferencesTests {
    @Test func whatIsSetIsKept() {
        let defaults = Fixture.defaults()
        let preferences = Preferences(defaults: defaults)
        #expect(preferences.fontSize == 12.5)
        #expect(preferences.phoneName == "iPhone")
        preferences.fontSize = 14
        preferences.phoneName = "Rocky's iPhone"
        let again = Preferences(defaults: defaults)
        #expect(again.fontSize == 14)
        #expect(again.phoneName == "Rocky's iPhone")
    }
}

struct SavedDeviceStoreTests {
    @Test func devicesSurviveARestartWithoutTheirInvite() {
        let defaults = Fixture.defaults()
        let store = SavedDeviceStore(defaults: defaults)
        store.add(SavedDevice(address: "p2p://abc", label: "desk"))
        store.add(SavedDevice(address: "tcp://key@host:1", label: "studio"))
        store.add(SavedDevice(address: "p2p://abc", label: "desk again"))
        let again = SavedDeviceStore(defaults: defaults)
        #expect(
            again.devices == [
                SavedDevice(address: "tcp://key@host:1", label: "studio"),
                SavedDevice(address: "p2p://abc", label: "desk again"),
            ])
        again.remove("p2p://abc")
        #expect(SavedDeviceStore(defaults: defaults).devices.map(\.label) == ["studio"])
    }
}

struct DeviceKeyTests {
    /// The Keychain hands back the seed it was given; a second start finds the first's.
    @Test func theSeedIsKeptBetweenStarts() throws {
        let first = try DeviceKey.load()
        let second = try DeviceKey.load()
        #expect(first.count == 32)
        #expect(first == second)
        #expect(try fingerprint(seed: first) == fingerprint(seed: second))
    }
}

struct DisplayTests {
    @Test func agentsSortMostUrgentFirstAndStableWithin() {
        let statuses: [AgentStatus] = [.idle, .working, .blocked, .done, .unknown, .working]
        #expect(statuses.byUrgency() == [.blocked, .done, .working, .working, .unknown, .idle])
    }

    @Test func aPaneIsNamedByItsTitleThenItsAgent() {
        let agent = AgentView(kind: "codex", label: "Codex", status: .idle, blockedOn: nil)
        #expect(
            PaneView(id: 1, title: "Fix it", agent: agent, attention: false, exited: false).name
                == "Fix it")
        #expect(
            PaneView(id: 1, title: nil, agent: agent, attention: false, exited: false).name
                == "Codex")
        #expect(
            PaneView(id: 1, title: nil, agent: nil, attention: false, exited: false).name == "Shell"
        )
    }

    @Test func marksFollowTheKindId() {
        #expect(AgentMarks.resource(for: "claude") == .agentClaude)
        #expect(AgentMarks.resource(for: "other") == nil)
        #expect(AgentMarks.resource(for: nil) == nil)
    }
}
