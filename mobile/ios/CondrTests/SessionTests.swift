import CondrKit
import Testing
import UIKit

@testable import Condr

struct SessionTests {
    @Test func anAgentElsewhereComingToNeedYouRaisesABanner() {
        let client = FakeClient(Fixture.world(.working))
        let (session, _, alerts, _) = Fixture.session(client)
        #expect(alerts.banner == nil)

        client.update { state in
            state.workspaces[Fixture.address]?[0].panes[0].agent?.status = .blocked
            state.workspaces[Fixture.address]?[0].panes[0].agent?.blockedOn = "Bash: rm -rf build"
            state.changes = [.device(address: Fixture.address)]
        }
        session.refresh()

        #expect(alerts.banner?.ref == Fixture.ref)
        #expect(alerts.banner?.title == "Claude needs you")
        #expect(alerts.banner?.detail == "Fix the thing · condr · desk")
        #expect(session.agents(.blocked).count == 1)
    }

    @Test func anAgentFirstSeenBlockedRaisesNothing() {
        let client = FakeClient(Fixture.world(.blocked, blockedOn: "Bash: cargo test"))
        let (session, _, alerts, _) = Fixture.session(client)
        client.update { $0.changes = [.device(address: Fixture.address)] }
        session.refresh()
        #expect(alerts.banner == nil)
    }

    @Test func thePaneOnScreenRaisesNoBanner() {
        let client = FakeClient(Fixture.world(.working))
        let (session, _, alerts, _) = Fixture.session(client)
        session.show(Fixture.ref, rows: 20, columns: 50)

        client.update { state in
            state.workspaces[Fixture.address]?[0].panes[0].agent?.status = .done
            state.changes = [.device(address: Fixture.address)]
        }
        session.refresh()

        #expect(alerts.banner == nil)
    }

    @Test func showingAPaneSizesItAndLeavingGivesItBack() {
        let client = FakeClient(Fixture.world(.idle))
        let (session, _, _, _) = Fixture.session(client)
        session.show(Fixture.ref, rows: 0, columns: 0)
        session.show(Fixture.ref, rows: 18, columns: 44)
        session.hide(Fixture.ref)
        let shows = client.state.withLock { $0.shows }
        #expect(
            shows == [
                .init(address: Fixture.address, pane: 7, rows: 0, columns: 0),
                .init(address: Fixture.address, pane: 7, rows: 18, columns: 44),
                .init(address: nil, pane: 7, rows: 0, columns: 0),
            ])
        #expect(session.shown == nil)
    }

    @Test func theShownPanesScreenFollowsItsChanges() {
        let client = FakeClient(Fixture.world(.working))
        let (session, _, _, _) = Fixture.session(client)
        session.show(Fixture.ref, rows: 2, columns: 2)
        #expect(session.screen == nil)

        let screen = TerminalScreen(
            revision: 3, rows: 2, columns: 2, scrolledLines: 0, cells: [], cursor: nil,
            background: 0,
            mouseTracking: false)
        client.update { state in
            state.screens[Fixture.ref] = screen
            state.changes = [.terminal(address: Fixture.address, pane: 7)]
        }
        session.refresh()
        #expect(session.screen?.revision == 3)
    }

    @Test func aClipboardEventLandsOnThePasteboardWithANotice() {
        let client = FakeClient(Fixture.world(.working))
        let (session, _, alerts, _) = Fixture.session(client)
        client.update { $0.events = [.clipboard(text: "copied text")] }
        session.refresh()
        #expect(UIPasteboard.general.string == "copied text")
        #expect(alerts.notice == "Copied to clipboard")
    }

    @Test func anErrorEventBecomesTheErrorToast() {
        let client = FakeClient(Fixture.world(.working))
        let (session, _, alerts, _) = Fixture.session(client)
        client.update {
            $0.events = [.error(address: Fixture.address, message: "the connection closed")]
        }
        session.refresh()
        #expect(alerts.error == "the connection closed")
    }

    @Test func forgettingADeviceDropsItEverywhere() {
        let client = FakeClient(Fixture.world(.working))
        let store = SavedDeviceStore(defaults: Fixture.defaults())
        store.add(SavedDevice(address: Fixture.address, label: "desk"))
        let (session, router, _, _) = Fixture.session(client, saved: store)
        router.agentsPath = [.pane(Fixture.ref)]
        router.devicesPath = [.device(Fixture.address)]
        session.show(Fixture.ref, rows: 1, columns: 1)

        session.forget(Fixture.address)

        #expect(session.devices.isEmpty)
        #expect(session.workspaces[Fixture.address] == nil)
        #expect(session.shown == nil)
        #expect(store.devices.isEmpty)
        #expect(router.agentsPath.isEmpty)
        #expect(router.devicesPath.isEmpty)
        #expect(client.state.withLock { $0.forgotten } == [Fixture.address])
    }

    @Test func pairingKeepsTheDevice() async throws {
        let client = FakeClient()
        let (session, _, _, store) = Fixture.session(client)
        #expect(!session.paired)
        let device = try await session.pair(link: "tcp://key.invite@host:1", phoneName: "my iPhone")
        #expect(device.label == "desk", "the Server names the Device")
        #expect(session.paired)
        #expect(store.devices == [device])
        #expect(client.state.withLock { $0.names } == ["my iPhone"], "the phone's name went first")
        #expect(client.state.withLock { $0.pairs.map(\.1) } == [nil])
    }

    @Test func aBadLinkIsRefusedInTheLibrarysWords() async {
        let client = FakeClient()
        let (session, _, _, store) = Fixture.session(client)
        await #expect(throws: MobileError.self) {
            try await session.pair(link: "http://nope", phoneName: "x")
        }
        #expect(store.devices.isEmpty)
    }

    @Test func inputGoesToTheShownPane() {
        let client = FakeClient(Fixture.world(.blocked, blockedOn: "Bash: ls"))
        let (session, _, _, _) = Fixture.session(client)
        session.send(key: .enter)
        #expect(client.state.withLock { $0.keys.isEmpty }, "nothing is sent before a Pane is shown")
        session.show(Fixture.ref, rows: 10, columns: 40)
        session.send(key: .character(text: "c"), control: true)
        session.send(text: "yes")
        session.scroll(lines: 3)
        session.scrollToBottom()
        let state = client.state.withLock { $0 }
        #expect(state.keys.map(\.1) == [true])
        #expect(state.texts == ["yes"])
        #expect(state.scrolls == [3, 0])
    }
}
