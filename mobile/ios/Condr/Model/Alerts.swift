import CondrKit
import Foundation
import Observation

/// An Agent elsewhere started to need you or finished, while you look at something else.
struct Banner: Identifiable, Equatable {
    let id = UUID()
    let ref: PaneRef
    let status: AgentStatus
    /// The Agent's kind id, for its mark.
    let kind: String?
    let title: String
    let detail: String
    /// Raised while the first screen is showing, which shows the change itself: felt, not shown.
    var quiet = false
}

/// What the app says over its screens: one banner, one error to read, copy and dismiss
/// (ADR 0020), and a passing notice such as "Copied to clipboard".
@Observable
@MainActor
final class Alerts {
    static let bannerLifetime: Duration = .seconds(6)
    static let noticeLifetime: Duration = .seconds(2)

    private(set) var banner: Banner?
    private(set) var error: String?
    private(set) var notice: String?

    func raise(_ banner: Banner) {
        self.banner = banner
        Task {
            try? await Task.sleep(for: Self.bannerLifetime)
            if self.banner?.id == banner.id { self.banner = nil }
        }
    }

    func dismissBanner() {
        banner = nil
    }

    /// Opening the Pane a banner points at answers it.
    func dismissBanner(for ref: PaneRef) {
        if banner?.ref == ref { banner = nil }
    }

    func report(_ message: String) {
        error = message
    }

    func dismissError() {
        error = nil
    }

    func notify(_ text: String) {
        notice = text
        Task {
            try? await Task.sleep(for: Self.noticeLifetime)
            if self.notice == text { self.notice = nil }
        }
    }
}
