import SwiftUI

/// An Agent elsewhere started to need you or finished, while you look at something else.
struct BannerView: View {
    @Environment(Session.self) private var session
    @Environment(Router.self) private var router
    @Environment(AppLock.self) private var lock
    @Environment(Alerts.self) private var alerts

    var body: some View {
        Group {
            if let banner = alerts.banner, !banner.quiet, session.shown != banner.ref, !lock.locked
            {
                Button {
                    alerts.dismissBanner()
                    router.open(banner.ref)
                } label: {
                    HStack(spacing: 12) {
                        AgentAvatar(
                            kind: banner.kind, status: banner.status, side: 38,
                            surface: Color(.systemBackground), ripple: true)
                        VStack(alignment: .leading, spacing: 1) {
                            Text(banner.title)
                                .font(.subheadline.weight(.semibold))
                            Text(banner.detail)
                                .font(.footnote)
                                .foregroundStyle(.secondary)
                                .lineLimit(1)
                        }
                        Spacer(minLength: 0)
                        Text("Open")
                            .font(.footnote.weight(.semibold))
                            .foregroundStyle(banner.status.tint)
                    }
                    .padding(.leading, 12)
                    .padding(.trailing, 18)
                    .padding(.vertical, 11)
                    .contentShape(.rect)
                }
                .buttonStyle(.plain)
                .accessibilityIdentifier("banner")
                .glassEffect(
                    .regular.interactive(), in: .rect(cornerRadius: 28, style: .continuous)
                )
                .padding(.horizontal, 10)
                .gesture(
                    DragGesture(minimumDistance: 8).onEnded { value in
                        if value.translation.height < 0 { alerts.dismissBanner() }
                    }
                )
                .transition(.move(edge: .top).combined(with: .opacity))
                .accessibilityHint("Opens its terminal")
            }
        }
        .animation(.spring(duration: 0.5, bounce: 0.25), value: alerts.banner)
    }
}

/// A failed operation as one message to read, copy and dismiss (ADR 0020).
struct ErrorToast: View {
    @Environment(Alerts.self) private var alerts

    var body: some View {
        Group {
            if let message = alerts.error {
                HStack(alignment: .firstTextBaseline, spacing: 12) {
                    Image(systemName: "exclamationmark.triangle.fill")
                        .foregroundStyle(.red)
                    Text(message)
                        .font(.subheadline)
                        .fixedSize(horizontal: false, vertical: true)
                    Spacer(minLength: 0)
                    Button("Copy") { UIPasteboard.general.string = message }
                        .font(.subheadline.weight(.semibold))
                        .frame(minWidth: 44, minHeight: 44)
                        .contentShape(.rect)
                    Button("Dismiss", systemImage: "xmark") { alerts.dismissError() }
                        .labelStyle(.iconOnly)
                        .font(.subheadline.weight(.semibold))
                        .foregroundStyle(.secondary)
                        .frame(width: 44, height: 44)
                        .contentShape(.rect)
                }
                .padding(.leading, 16)
                .padding(.trailing, 6)
                .padding(.vertical, 4)
                .glassEffect(.regular, in: .rect(cornerRadius: 24, style: .continuous))
                .padding(.horizontal, 12)
                .transition(.move(edge: .bottom).combined(with: .opacity))
            }
        }
        .animation(.spring(duration: 0.45, bounce: 0.2), value: alerts.error)
    }
}

/// A passing word, such as "Copied to clipboard".
struct NoticeToast: View {
    @Environment(Alerts.self) private var alerts

    var body: some View {
        Group {
            if let notice = alerts.notice {
                Text(notice)
                    .font(.subheadline.weight(.medium))
                    .padding(.horizontal, 16)
                    .padding(.vertical, 10)
                    .glassEffect(.regular, in: .capsule)
                    .transition(.move(edge: .bottom).combined(with: .opacity))
            }
        }
        .animation(.spring(duration: 0.45, bounce: 0.2), value: alerts.notice)
    }
}
