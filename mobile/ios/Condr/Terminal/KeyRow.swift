import CondrKit
import SwiftUI

/// The keys the soft keyboard lacks (ADR 0041): Esc, Tab, a sticky Ctrl and the arrows first,
/// which is what a permission prompt or a numbered question needs, then, a swipe along,
/// Shift-Tab, Paste, Home, End and the page keys; Return stays put at the end.
struct KeyRow: View {
    @Binding var control: Bool
    let press: (Key) -> Void
    /// Text from the clipboard, for the terminal as a paste.
    let paste: (String) -> Void

    var body: some View {
        HStack(spacing: 6) {
            ScrollView(.horizontal) {
                GlassEffectContainer(spacing: 6) {
                    HStack(spacing: 6) {
                        key("esc", "Escape") { press(.escape) }
                        key("tab", "Tab") { press(.tab) }
                        controlKey
                        HStack(spacing: 0) {
                            arrow("arrow.left", .left, "Left")
                            arrow("arrow.up", .up, "Up")
                            arrow("arrow.down", .down, "Down")
                            arrow("arrow.right", .right, "Right")
                        }
                        .glassEffect(.regular.interactive(), in: .capsule)
                        key("⇧tab", "Shift-Tab") { press(.backTab) }
                        PasteButton(payloadType: String.self) { strings in
                            paste(strings.joined(separator: "\n"))
                        }
                        .labelStyle(.iconOnly)
                        .buttonBorderShape(.capsule)
                        .tint(Color.paper.opacity(0.18))
                        .frame(minHeight: 44)
                        key("home", "Home") { press(.home) }
                        key("end", "End") { press(.end) }
                        key("pgup", "Page Up") { press(.pageUp) }
                        key("pgdn", "Page Down") { press(.pageDown) }
                    }
                }
            }
            .scrollIndicators(.hidden)
            Button {
                press(.enter)
            } label: {
                Image(systemName: "return")
                    .font(.system(size: 15, weight: .semibold))
                    .frame(minWidth: 46, minHeight: 44)
            }
            .buttonStyle(.plain)
            .glassEffect(.regular.interactive(), in: .capsule)
            .accessibilityLabel("Return")
        }
    }

    private var controlKey: some View {
        Button {
            control.toggle()
        } label: {
            Text("ctrl")
                .font(.system(size: 14, weight: .semibold, design: .monospaced))
                .frame(minWidth: 46, minHeight: 44)
                .foregroundStyle(control ? Color.ink : Color.primary)
        }
        .buttonStyle(.plain)
        .glassEffect(
            control ? .regular.tint(.paper).interactive() : .regular.interactive(),
            in: .capsule
        )
        .accessibilityLabel("Control")
        .accessibilityAddTraits(control ? .isSelected : [])
    }

    private func key(
        _ title: String, _ label: LocalizedStringResource, action: @escaping () -> Void
    ) -> some View {
        Button(action: action) {
            Text(title)
                .font(.system(size: 14, weight: .semibold, design: .monospaced))
                .padding(.horizontal, 6)
                .frame(minWidth: 46, minHeight: 44)
        }
        .buttonStyle(.plain)
        .glassEffect(.regular.interactive(), in: .capsule)
        .accessibilityLabel(label)
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
