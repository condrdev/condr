import CondrKit
import SwiftTerm
import SwiftUI

/// SwiftTerm's view as a renderer only (ADR 0041): it draws the screens the Server sends
/// and offers its selection, Copy and links; its keyboard stays closed, nothing it would
/// send goes anywhere, and a swipe becomes the Pane's scroll.
struct TerminalHost: UIViewRepresentable {
    let screen: TerminalScreen?
    let fontSize: Double
    /// The grid the view fits at its size and font, whenever either changes.
    let grid: @MainActor (_ rows: Int, _ columns: Int) -> Void
    let scroll: @MainActor (_ lines: Int32) -> Void
    let open: @MainActor (URL) -> Void

    final class Surface: TerminalView {
        override var canBecomeFirstResponder: Bool { false }
    }

    func makeUIView(context: Context) -> Surface {
        let view = Surface(
            frame: .zero, font: Self.font(fontSize), options: TerminalOptions(scrollback: 0))
        view.allowMouseReporting = false
        view.linkReporting = .explicit
        view.nativeBackgroundColor = UIColor(Color(hex: Palette.background))
        view.isScrollEnabled = false
        view.terminalDelegate = context.coordinator
        let pan = UIPanGestureRecognizer(
            target: context.coordinator, action: #selector(Coordinator.pan(_:)))
        pan.maximumNumberOfTouches = 1
        view.addGestureRecognizer(pan)
        return view
    }

    func updateUIView(_ view: Surface, context: Context) {
        context.coordinator.host = self
        if view.font.pointSize != fontSize {
            view.font = Self.font(fontSize)
        }
        context.coordinator.show(screen, in: view)
    }

    func makeCoordinator() -> Coordinator {
        Coordinator(host: self)
    }

    static func font(_ size: Double) -> UIFont {
        .monospacedSystemFont(ofSize: size, weight: .regular)
    }

    @MainActor
    final class Coordinator: NSObject, TerminalViewDelegate {
        var host: TerminalHost
        private var encoder = ScreenEncoder()
        private var shown: TerminalScreen?
        private var lastGrid: (rows: Int, columns: Int)?
        /// Swipe distance not yet turned into lines, in points.
        private var dragged: CGFloat = 0

        init(host: TerminalHost) {
            self.host = host
        }

        func show(_ screen: TerminalScreen?, in view: Surface) {
            guard let screen, screen != shown else { return }
            let dimensions = view.terminalDimensions
            view.feed(
                byteArray: encoder.encode(screen, rows: dimensions.rows, columns: dimensions.cols)[
                    ...])
            shown = screen
        }

        nonisolated func sizeChanged(source: TerminalView, newCols: Int, newRows: Int) {
            Task { @MainActor in
                guard let view = source as? Surface else { return }
                encoder.reset()
                if let screen = shown {
                    view.feed(
                        byteArray: encoder.encode(screen, rows: newRows, columns: newCols)[...])
                }
                if lastGrid?.rows != newRows || lastGrid?.columns != newCols {
                    lastGrid = (newRows, newCols)
                    host.grid(newRows, newCols)
                }
            }
        }

        nonisolated func requestOpenLink(
            source: TerminalView, link: String, params: [String: String]
        ) {
            guard let url = URL(string: link) else { return }
            Task { @MainActor in host.open(url) }
        }

        // Nothing typed into the renderer goes anywhere: input is the page's key row and field.
        nonisolated func send(source: TerminalView, data: ArraySlice<UInt8>) {}
        nonisolated func setTerminalTitle(source: TerminalView, title: String) {}
        nonisolated func hostCurrentDirectoryUpdate(source: TerminalView, directory: String?) {}
        nonisolated func scrolled(source: TerminalView, position: Double) {}
        nonisolated func bell(source: TerminalView) {}
        nonisolated func rangeChanged(source: TerminalView, startY: Int, endY: Int) {}
        nonisolated func iTermContent(source: TerminalView, content: ArraySlice<UInt8>) {}
        nonisolated func clipboardCopy(source: TerminalView, content: Data) {}
        nonisolated func clipboardRead(source: TerminalView) -> Data? { nil }
        nonisolated func kittyClipboardCapabilities(source: TerminalView)
            -> KittyClipboardCapabilities
        { [] }

        /// A swipe scrolls the Pane for every viewer (ADR 0036): one line per cell height,
        /// a finger moving down showing what came before.
        @objc func pan(_ gesture: UIPanGestureRecognizer) {
            guard let view = gesture.view as? Surface else { return }
            switch gesture.state {
            case .began:
                dragged = 0
            case .changed:
                let translation = gesture.translation(in: view).y
                gesture.setTranslation(.zero, in: view)
                dragged += translation
                let line = view.font.lineHeight
                let lines = (dragged / line).rounded(.towardZero)
                if lines != 0 {
                    dragged -= lines * line
                    host.scroll(Int32(lines))
                }
            default:
                dragged = 0
            }
        }
    }
}
