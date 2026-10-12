import SwiftUI

/// A window over the app's own, for what must cover everything: the lock and the shield.
/// It takes the keyboard and VoiceOver from the window beneath, appears at once and only
/// fades away; showing the same thing again keeps it rather than building it anew.
@MainActor
final class Curtain {
    enum Kind { case lock, shield }

    private var window: UIWindow?
    private var kind: Kind?
    /// Bumped by every show and hide, so a fade that finishes late can't undo a newer show.
    private var generation = 0

    func show(_ kind: Kind, _ view: () -> AnyView) {
        generation += 1
        if let window, self.kind == kind {
            // A fade already under way would show the app for a moment before snapping back.
            window.layer.removeAllAnimations()
            window.alpha = 1
            window.isHidden = false
            return
        }
        guard
            let scene = UIApplication.shared.connectedScenes.compactMap({ $0 as? UIWindowScene })
                .first
        else { return }
        // Whatever had the keyboard gives it up, so no keystroke reaches the work behind.
        UIApplication.shared.sendAction(
            #selector(UIResponder.resignFirstResponder), to: nil, from: nil, for: nil)
        let window = window ?? UIWindow(windowScene: scene)
        window.windowLevel = .alert + 1
        window.rootViewController = UIHostingController(rootView: view())
        window.alpha = 1
        window.makeKeyAndVisible()
        // VoiceOver reads this window alone: the app's own windows drop out until it goes.
        window.accessibilityViewIsModal = true
        for other in scene.windows where other !== window {
            other.accessibilityElementsHidden = true
        }
        self.window = window
        self.kind = kind
    }

    func hide() {
        guard let window else { return }
        generation += 1
        let mine = generation
        UIView.animate(withDuration: 0.35, animations: { window.alpha = 0 }) { _ in
            guard self.generation == mine else { return }
            window.isHidden = true
            for other in window.windowScene?.windows ?? [] where other !== window {
                other.accessibilityElementsHidden = false
            }
            window.windowScene?.windows.first { $0 !== window }?.makeKey()
            self.window = nil
            self.kind = nil
        }
    }
}

/// What the app switcher and a returning glance see: the mark, and nothing of the work.
struct Shield: View {
    var body: some View {
        ZStack {
            Color(.systemBackground).ignoresSafeArea()
            AppMark(side: 80)
        }
    }
}

/// The owner's face, finger or passcode before anything shows (ADR 0041).
struct LockView: View {
    @Environment(AppLock.self) private var lock

    var body: some View {
        ZStack {
            Color(.systemBackground).ignoresSafeArea()
            VStack(spacing: 0) {
                Spacer()
                AppMark(side: 80)
                    .shadow(color: .black.opacity(0.15), radius: 16, y: 8)
                Text("Condr is locked")
                    .font(.title2.bold())
                    .padding(.top, 24)
                Text("Unlock to reach your computers.")
                    .foregroundStyle(.secondary)
                    .padding(.top, 6)
                Image(systemName: "faceid")
                    .font(.system(size: 44, weight: .light))
                    .foregroundStyle(lock.authenticating ? Color.accentColor : Color.secondary)
                    .symbolEffect(.pulse, isActive: lock.authenticating)
                    .padding(.top, 44)
                Spacer()
                PrimaryButton("Unlock") { Task { await lock.unlock() } }
                    .padding(.horizontal, 28)
                    .padding(.bottom, 12)
            }
        }
        .task { if lock.promptsOnAppear { await lock.unlock() } }
    }
}
