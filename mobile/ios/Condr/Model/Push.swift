import CondrKit
import Foundation
import Observation
import UserNotifications

/// This phone's registration with Condr Cloud, and the one `push` flag every `Hello` says
/// (ADR 0047): on while notifications are allowed and the last registration succeeded.
/// Each launch, a new APNs token and a Device paired or forgotten register again; losing
/// permission or the last Device removes a registration once made. A phone that never
/// registered, having declined or never paired, sends nothing. A request that fails waits
/// for the next of those; requests go out one at a time, in the order they were made.
@Observable
@MainActor
final class Push {
    typealias Send = (URLRequest) async throws -> (Data, URLResponse)

    /// The APNs environment the build's signing gives its token.
    #if DEBUG
        nonisolated static let environment = "SANDBOX"
    #else
        nonisolated static let environment = "PRODUCTION"
    #endif
    private static let key = "push.on"
    /// Set before the first `PUT` and cleared by a `DELETE` that succeeded: whether Condr
    /// Cloud may hold a registration to remove.
    private static let registeredKey = "push.registered"

    /// What every `Hello` says, kept for the next launch's first connections.
    private(set) var on: Bool

    private let client: any CompanionProtocol
    private let cloud: String
    private let defaults: UserDefaults
    private let send: Send
    private var status: UNAuthorizationStatus?
    private var servers: [String] = []
    private var token: Data?
    /// Bumped by every registration, so only the latest one's answer sets the flag.
    private var attempt = 0
    /// The last request, which the next one waits for.
    private var last: Task<Bool, Never>?

    /// Says the last known flag to the library at once, before the first `resume()`.
    init(
        client: any CompanionProtocol, cloud: String = cloudUrl(),
        defaults: UserDefaults = .standard,
        send: @escaping Send = { try await URLSession.shared.data(for: $0) }
    ) {
        self.client = client
        self.cloud = cloud
        self.defaults = defaults
        self.send = send
        on = defaults.bool(forKey: Self.key)
        client.setPush(push: on)
    }

    /// The notification permission and the Devices paired now, as the app last found them.
    /// A registration that failed, or whose answer never came, is tried again on the next
    /// call even when nothing changed, so the flag does not stay off until something does.
    func update(status: UNAuthorizationStatus, servers: [String]) async {
        guard status != self.status || servers != self.servers || !on else { return }
        self.status = status
        self.servers = servers
        await register()
    }

    /// APNs gave this launch's token, or a new one.
    func received(token: Data) async {
        guard token != self.token else { return }
        self.token = token
        await register()
    }

    /// APNs gave no token: nothing can reach this phone.
    func failed() {
        token = nil
        attempt += 1
        set(false)
    }

    private func register() async {
        attempt += 1
        let current = attempt
        guard let status else { return }
        if status == .authorized, !servers.isEmpty {
            // Without a token yet, its arrival registers.
            guard let token else { return }
            let body = Registration(token: token, servers: servers)
            let registered = await inOrder {
                // A `PUT` whose answer is lost may still have registered.
                self.defaults.set(true, forKey: Self.registeredKey)
                return await self.succeeds(self.request("PUT", body: body))
            }
            if current == attempt { set(registered) }
        } else {
            set(false)
            _ = await inOrder {
                guard self.defaults.bool(forKey: Self.registeredKey) else { return true }
                let removed = await self.succeeds(self.request("DELETE"))
                if removed { self.defaults.removeObject(forKey: Self.registeredKey) }
                return removed
            }
        }
    }

    /// Runs `request` once every request made before it has its answer, so a `DELETE` and
    /// the `PUT` after it reach Condr Cloud in that order.
    private func inOrder(_ request: @escaping () async -> Bool) async -> Bool {
        let previous = last
        let next = Task {
            _ = await previous?.value
            return await request()
        }
        last = next
        return await next.value
    }

    /// Signed as it goes out, since a token is good for five minutes.
    private func request(_ method: String, body: Registration? = nil) -> URLRequest {
        Self.request(
            method, cloud: cloud, authorization: client.cloudToken(), body: body)
    }

    private func succeeds(_ request: URLRequest) async -> Bool {
        guard let (data, response) = try? await send(request),
            (response as? HTTPURLResponse).map({ (200..<300).contains($0.statusCode) }) == true
        else { return false }
        return (try? JSONDecoder().decode(Reply.self, from: data))?.code == "200"
    }

    private func set(_ on: Bool) {
        self.on = on
        defaults.set(on, forKey: Self.key)
        client.setPush(push: on)
    }

    /// `PUT` or `DELETE /api/push/device`, signed by the phone's key, which also names the phone.
    static func request(
        _ method: String, cloud: String, authorization: String,
        body: Registration? = nil
    ) -> URLRequest {
        guard let base = URL(string: cloud) else { preconditionFailure("Bad Condr Cloud URL") }
        var request = URLRequest(url: base.appending(path: "api/push/device"))
        request.httpMethod = method
        // A request whose answer is lost holds up every one queued after it.
        request.timeoutInterval = 15
        request.setValue("1", forHTTPHeaderField: "API-Version")
        request.setValue("Condr \(authorization)", forHTTPHeaderField: "Authorization")
        if let body {
            request.setValue("application/json", forHTTPHeaderField: "Content-Type")
            request.httpBody = try? JSONEncoder().encode(body)
        }
        return request
    }

    /// Condr Cloud answers HTTP 200 with the outcome in `code`.
    nonisolated struct Reply: Decodable {
        let code: String
    }

    /// What `PUT /api/push/device` registers: the APNs token and the Devices that may push to it.
    nonisolated struct Registration: Encodable {
        let platform = "APNS"
        let environment = Push.environment
        let token: String
        let servers: [String]

        init(token: Data, servers: [String]) {
            self.token = token.map { String(format: "%02x", $0) }.joined()
            self.servers = servers
        }
    }
}

/// The Pane a notification names: its Server's Device id and the Pane id (ADR 0047).
nonisolated struct PushedPane: Equatable, Sendable {
    let device: String
    let pane: UInt64

    init?(_ userInfo: [AnyHashable: Any]) {
        guard let device = userInfo["device"] as? String,
            let pane = (userInfo["pane"] as? String).flatMap({ UInt64($0) })
        else { return nil }
        self.device = device
        self.pane = pane
    }
}
