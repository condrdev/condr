import CondrKit
import SwiftUI

/// The keys the soft keyboard lacks (ADR 0041): Esc, Tab, a sticky Ctrl, the arrows and
/// Return, which is what a permission prompt or a numbered question needs.
struct KeyRow: View {
    @Binding var control: Bool
    let press: (Key) -> Void

    var body: some View {
        GlassEffectContainer(spacing: 6) {
            HStack(spacing: 6) {
                key("esc") { press(.escape) }
                key("tab") { press(.tab) }
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
                Spacer(minLength: 0)
                HStack(spacing: 0) {
                    arrow("arrow.left", .left, "Left")
                    arrow("arrow.up", .up, "Up")
                    arrow("arrow.down", .down, "Down")
                    arrow("arrow.right", .right, "Right")
                }
                .glassEffect(.regular.interactive(), in: .capsule)
                key(symbol: "return", label: "Return") { press(.enter) }
            }
        }
    }

    private func key(_ title: String, action: @escaping () -> Void) -> some View {
        Button(action: action) {
            Text(title)
                .font(.system(size: 14, weight: .semibold, design: .monospaced))
                .frame(minWidth: 46, minHeight: 44)
        }
        .buttonStyle(.plain)
        .glassEffect(.regular.interactive(), in: .capsule)
    }

    private func key(symbol: String, label: LocalizedStringResource, action: @escaping () -> Void)
        -> some View
    {
        Button(action: action) {
            Image(systemName: symbol)
                .font(.system(size: 15, weight: .semibold))
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
