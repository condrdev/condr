import CondrKit
import Foundation
import Synchronization
import Testing
import UserNotifications

@testable import Condr

/// Condr Cloud as a test sees it: the requests it got, answered HTTP 200 with `status` as `R.code`.
nonisolated final class FakeCloud: Sendable {
    let status: Mutex<Int>
    let requests = Mutex<[URLRequest]>([])
    /// How long a `DELETE` takes to answer.
    let deleteTakes = Mutex(Duration.zero)
    /// The methods in the order their answers came back.
    let answered = Mutex<[String]>([])

    init(status: Int = 200) {
        self.status = Mutex(status)
    }

    func send(_ request: URLRequest) async throws -> (Data, URLResponse) {
        requests.withLock { $0.append(request) }
        let delay = deleteTakes.withLock { $0 }
        if request.httpMethod == "DELETE", delay > .zero { try await Task.sleep(for: delay) }
        answered.withLock { $0.append(request.httpMethod ?? "") }
        let status = self.status.withLock { $0 }
        guard let url = request.url,
            let response = HTTPURLResponse(
                url: url, statusCode: 200, httpVersion: nil, headerFields: nil)
        else { throw URLError(.badURL) }
        return (Data("{\"code\":\"\(status)\"}".utf8), response)
    }

    var methods: [String] { requests.withLock { $0.compactMap(\.httpMethod) } }
}

@MainActor
struct PushTests {
    let client = FakeClient()
    let cloud = FakeCloud()
    let defaults = Fixture.defaults()

    func push() -> Push {
        Push(
            client: client, cloud: "https://cloud.test", defaults: defaults,
            send: cloud.send)
    }

    @Test func theRegistrationNamesThePhoneItsTokenAndEveryServer() async throws {
        let push = push()
        await push.update(status: .authorized, servers: ["mac-id", "box-id"])
        #expect(cloud.methods.isEmpty, "nothing goes out before APNs gives a token")
        await push.received(token: Data([0x0a, 0xff, 0x00]))

        let request = try #require(cloud.requests.withLock { $0.first })
        #expect(request.httpMethod == "PUT")
        #expect(request.url?.absoluteString == "https://cloud.test/api/push/device")
        #expect(request.value(forHTTPHeaderField: "API-Version") == "1")
        #expect(request.value(forHTTPHeaderField: "Authorization") == "Condr signed")
        #expect(request.value(forHTTPHeaderField: "Content-Type") == "application/json")
        let body = try #require(request.httpBody)
        let json = try #require(try JSONSerialization.jsonObject(with: body) as? [String: Any])
        #expect(json["platform"] as? String == "APNS")
        #expect(json["environment"] as? String == "SANDBOX", "a Debug build's token is sandbox")
        #expect(json["token"] as? String == "0aff00")
        #expect(json["servers"] as? [String] == ["mac-id", "box-id"])
        #expect(json.count == 4)
        #expect(push.on)
    }

    @Test func theFlagIsKeptAndSaidBeforeTheNextLaunchConnects() async {
        let first = push()
        await first.update(status: .authorized, servers: ["mac-id"])
        await first.received(token: Data([1]))
        #expect(client.state.withLock { $0.pushes } == [false, true])

        let next = push()
        #expect(next.on)
        #expect(client.state.withLock { $0.pushes } == [false, true, true])
    }

    @Test func aFailedRegistrationTurnsTheFlagOffUntilOneSucceeds() async {
        let push = push()
        await push.received(token: Data([1]))
        cloud.status.withLock { $0 = 500 }
        await push.update(status: .authorized, servers: ["mac-id"])
        #expect(!push.on)

        cloud.status.withLock { $0 = 200 }
        await push.update(status: .authorized, servers: ["mac-id", "box-id"])
        #expect(push.on)
        #expect(cloud.methods == ["PUT", "PUT"])
    }

    @Test func aFailedRegistrationIsTriedAgainEvenWhenNothingChanged() async {
        let push = push()
        await push.received(token: Data([1]))
        cloud.status.withLock { $0 = 500 }
        await push.update(status: .authorized, servers: ["mac-id"])
        #expect(!push.on)

        cloud.status.withLock { $0 = 200 }
        await push.update(status: .authorized, servers: ["mac-id"])
        #expect(push.on, "the next return to the foreground registers again")
        #expect(cloud.methods == ["PUT", "PUT"])
    }

    @Test func losingPermissionOrTheLastDeviceRemovesTheRegistration() async {
        let push = push()
        await push.received(token: Data([1]))
        await push.update(status: .authorized, servers: ["mac-id"])
        #expect(push.on)

        await push.update(status: .authorized, servers: [])
        #expect(!push.on)
        await push.update(status: .authorized, servers: ["mac-id"])
        await push.update(status: .denied, servers: ["mac-id"])
        #expect(!push.on)
        #expect(cloud.methods == ["PUT", "DELETE", "PUT", "DELETE"])
        let delete = cloud.requests.withLock { $0.last }
        #expect(delete?.url?.absoluteString == "https://cloud.test/api/push/device")
        #expect(delete?.value(forHTTPHeaderField: "Authorization") == "Condr signed")
        #expect(delete?.httpBody == nil)

        await push.update(status: .authorized, servers: [])
        await self.push().update(status: .denied, servers: ["mac-id"])
        #expect(cloud.methods.count == 4, "a removed registration is not removed again")
    }

    @Test func aFailedRemovalIsTriedAgainNextLaunch() async {
        let push = push()
        await push.received(token: Data([1]))
        await push.update(status: .authorized, servers: ["mac-id"])
        cloud.status.withLock { $0 = 500 }
        await push.update(status: .denied, servers: ["mac-id"])

        cloud.status.withLock { $0 = 200 }
        await self.push().update(status: .denied, servers: ["mac-id"])
        await self.push().update(status: .denied, servers: ["mac-id"])
        #expect(cloud.methods == ["PUT", "DELETE", "DELETE"])
    }

    @Test func aPhoneThatDeclinedOrNeverPairedSendsNothing() async {
        for (status, servers) in [
            (UNAuthorizationStatus.notDetermined, ["mac-id"]), (.denied, ["mac-id"]),
            (.authorized, []),
        ] {
            let push = push()
            await push.received(token: Data([1]))
            await push.update(status: status, servers: servers)
            #expect(!push.on)
        }
        #expect(cloud.methods.isEmpty)
    }

    @Test func aRemovalAndTheRegistrationAfterItLandInOrder() async {
        let push = push()
        await push.received(token: Data([1]))
        await push.update(status: .authorized, servers: ["mac-id"])
        cloud.deleteTakes.withLock { $0 = .milliseconds(200) }

        let forget = Task { await push.update(status: .authorized, servers: []) }
        let pair = Task { await push.update(status: .authorized, servers: ["box-id"]) }
        await forget.value
        await pair.value
        #expect(cloud.answered.withLock { $0 } == ["PUT", "DELETE", "PUT"])
        #expect(push.on)

        await push.update(status: .denied, servers: ["box-id"])
        #expect(cloud.methods.last == "DELETE", "the last registration is removed too")
    }

    @Test func nothingChangedSendsNothingAgain() async {
        let push = push()
        await push.update(status: .authorized, servers: ["mac-id"])
        await push.received(token: Data([1]))
        await push.received(token: Data([1]))
        await push.update(status: .authorized, servers: ["mac-id"])
        #expect(cloud.methods == ["PUT"])

        await push.received(token: Data([2]))
        #expect(cloud.methods == ["PUT", "PUT"], "a new token registers again")
    }

    @Test func noTokenFromAPNsTurnsTheFlagOff() async {
        let push = push()
        await push.update(status: .authorized, servers: ["mac-id"])
        await push.received(token: Data([1]))
        push.failed()
        #expect(!push.on)
    }
}

struct PushedPaneTests {
    @Test func aNotificationNamesItsDeviceAndPane() {
        let pushed = PushedPane(["device": "mac-id", "pane": "42", "aps": [:]])
        #expect(pushed?.device == "mac-id")
        #expect(pushed?.pane == 42)
        #expect(PushedPane(["device": "mac-id", "pane": "x"]) == nil)
        #expect(PushedPane(["pane": "42"]) == nil)
    }

    @Test func itOpensThePaneOnTheDeviceItsIdNames() {
        var state = FakeClient.State()
        state.devices = [
            DeviceView(address: "p2p://mac", deviceId: "mac-id", label: "mac", status: .connected),
            DeviceView(
                address: "tcp://other@10.0.0.2:7418", deviceId: "other", label: "box",
                status: .connected),
        ]
        let (session, router, _, _) = Fixture.session(FakeClient(state))

        let ref = session.ref(PushedPane(["device": "other", "pane": "3"])!)
        #expect(ref == PaneRef(address: "tcp://other@10.0.0.2:7418", pane: 3))
        #expect(session.ref(PushedPane(["device": "forgotten", "pane": "3"])!) == nil)

        router.openNotified(ref!)
        #expect(router.agentsPath == [.device(ref!.address), .pane(ref!)])
        router.close(ref!)
        #expect(
            router.agentsPath == [.device(ref!.address)], "a gone Pane leaves its Device's list")
    }

    @Test func theSystemBannerShowsOnlyForADeviceNotConnectedHere() {
        var state = FakeClient.State()
        state.devices = [
            DeviceView(address: "p2p://mac", deviceId: "mac-id", label: "mac", status: .connected),
            DeviceView(
                address: "p2p://box", deviceId: "box-id", label: "box", status: .connecting),
        ]
        let (session, _, _, _) = Fixture.session(FakeClient(state))

        #expect(
            !session.presents(PushedPane(["device": "mac-id", "pane": "3"])!),
            "a connected Device's change raises the in-app banner")
        #expect(session.presents(PushedPane(["device": "box-id", "pane": "3"])!))
        #expect(session.presents(PushedPane(["device": "forgotten", "pane": "3"])!))

        session.show(PaneRef(address: "p2p://box", pane: 3), rows: 10, columns: 40)
        #expect(
            !session.presents(PushedPane(["device": "box-id", "pane": "3"])!),
            "the Pane on screen gets none")
        #expect(session.presents(PushedPane(["device": "box-id", "pane": "4"])!))
    }
}
