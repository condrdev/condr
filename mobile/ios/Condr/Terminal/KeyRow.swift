import CondrKit
import SwiftUI

/// The keys the soft keyboard lacks (ADR 0041): Esc, Tab, a sticky Ctrl and the arrows first,
/// which is what a permission prompt or a numbered question needs, then, a swipe along,
/// Shift-Tab, Home, End and the page keys; a key cut off at the edge shows that the row
/// scrolls. Past a divider, Backspace and Return stay put, since a line already in the
/// program's composer is edited there, not in the field.
struct KeyRow: View {
    @Binding var control: Bool
    let press: (Key) -> Void

    var body: some View {
        HStack(spacing: 6) {
            ScrollView(.horizontal) {
                GlassEffectContainer(spacing: 6) {
                    HStack(spacing: 6) {
                        key("esc", "Escape") { press(.escape) }
                        key("tab", "Tab") { press(.tab) }
                        controlKey
                        ScrolledKey {
                            HStack(spacing: 0) {
                                arrow("arrow.left", .left, "Left")
                                arrow("arrow.up", .up, "Up")
                                arrow("arrow.down", .down, "Down")
                                arrow("arrow.right", .right, "Right")
                            }
                        }
                        key("⇧tab", "Shift-Tab") { press(.backTab) }
                        key("home", "Home") { press(.home) }
                        key("end", "End") { press(.end) }
                        key("pgup", "Page Up") { press(.pageUp) }
                        key("pgdn", "Page Down") { press(.pageDown) }
                    }
                }
            }
            .scrollIndicators(.hidden)
            Divider()
                .frame(height: 24)
                .padding(.horizontal, 6)
            // Backspace goes down on touch and repeats while held, as a hardware key does.
            Button {
            } label: {
                Image(systemName: "delete.left")
                    .font(.system(size: 15, weight: .semibold))
                    .frame(minWidth: 46, minHeight: 44)
            }
            .buttonStyle(KeyStyle(repeating: { press(.backspace) }))
            .glassEffect(.regular.interactive(), in: .capsule)
            .accessibilityLabel("Backspace")
            .accessibilityIdentifier("terminalBackspace")
            // Named apart from the soft keyboard's own Return key.
            fixedKey("return", .enter, "Return", id: "terminalReturn")
        }
    }

    private var controlKey: some View {
        ScrolledKey(glass: control ? .regular.tint(.paper) : .regular) {
            Button {
                control.toggle()
            } label: {
                Text("ctrl")
                    .font(.system(size: 14, weight: .semibold, design: .monospaced))
                    .frame(minWidth: 46, minHeight: 44)
                    .foregroundStyle(control ? Color.ink : Color.primary)
            }
            .buttonStyle(.plain)
            .accessibilityLabel("Control")
            .accessibilityAddTraits(control ? .isSelected : [])
        }
    }

    private func fixedKey(
        _ symbol: String, _ key: Key, _ label: LocalizedStringResource, id: String
    ) -> some View {
        Button {
            press(key)
        } label: {
            Image(systemName: symbol)
                .font(.system(size: 15, weight: .semibold))
                .frame(minWidth: 46, minHeight: 44)
        }
        .buttonStyle(.plain)
        .glassEffect(.regular.interactive(), in: .capsule)
        .accessibilityLabel(label)
        .accessibilityIdentifier(id)
    }

    private func key(
        _ title: String, _ label: LocalizedStringResource, action: @escaping () -> Void
    ) -> some View {
        ScrolledKey {
            Button(action: action) {
                Text(title)
                    .font(.system(size: 14, weight: .semibold, design: .monospaced))
                    .padding(.horizontal, 6)
                    .frame(minWidth: 46, minHeight: 44)
            }
            .buttonStyle(.plain)
            .accessibilityLabel(label)
        }
    }

    private func arrow(_ symbol: String, _ key: Key, _ label: LocalizedStringResource) -> some View
    {
        Button {
            press(key)
        } label: {
            Image(systemName: symbol)
                .font(.system(size: 14, weight: .semibold))
                .frame(width: 36, height: 44)
                .contentShape(.rect)
        }
        .buttonStyle(.plain)
        .accessibilityLabel(label)
    }
}

/// A key that goes down on touch and repeats while held, as a hardware key does; the
/// Button's own action stays empty.
private struct KeyStyle: ButtonStyle {
    var repeating: (() -> Void)?

    func makeBody(configuration: Configuration) -> some View {
        configuration.label
            .modifier(Repeating(down: configuration.isPressed, press: repeating))
    }
}

private struct Repeating: ViewModifier {
    let down: Bool
    let press: (() -> Void)?
    @State private var repeating: Task<Void, Never>?

    func body(content: Content) -> some View {
        content
            .onChange(of: down) { _, down in
                repeating?.cancel()
                guard down, let press else { return }
                press()
                repeating = Task {
                    try? await Task.sleep(for: .milliseconds(450))
                    while !Task.isCancelled {
                        press()
                        try? await Task.sleep(for: .milliseconds(70))
                    }
                }
            }
            .onDisappear { repeating?.cancel() }
    }
}

/// A key that scrolls. Interactive glass lights the glass shape under a touch by the shape's
/// whole geometry: the ScrollView masks what is scrolled out of view but leaves the shape, so
/// a key half out of view lit up under a touch beside or on the fixed keys, where its shape
/// still ran, while its Button, which is clipped, stayed quiet. So the glass is interactive
/// only while the key is wholly in view.
private struct ScrolledKey<Content: View>: View {
    var glass: Glass = .regular
    @ViewBuilder let content: Content
    @State private var whole = false

    var body: some View {
        content
            .glassEffect(glass.interactive(whole), in: .capsule)
            .onScrollVisibilityChange(threshold: 0.99) { whole = $0 }
    }
}
