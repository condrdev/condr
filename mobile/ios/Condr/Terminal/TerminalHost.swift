import CondrKit
import SwiftTerm
import SwiftUI

/// SwiftTerm's view as a renderer only (ADR 0041): it draws the screens the Server sends.
/// The touches on it are the Pane's, as the desktop's pointer is: a swipe is wheel motion,
/// a tap opens a link or, where the program tracks the mouse, clicks, and a long press
/// selects in the selection every viewer shares, with Copy to put it on the clipboard.
/// Its keyboard stays closed and nothing it would send goes anywhere.
struct TerminalHost: UIViewRepresentable {
    let screen: TerminalScreen?
    let fontSize: Double
    /// The grid the view fits at its size and font, whenever either changes.
    let grid: @MainActor (_ rows: Int, _ columns: Int) -> Void
    /// Where the touches go.
    let session: Session
    let open: @MainActor (URL) -> Void

    final class Surface: TerminalView {
        override var canBecomeFirstResponder: Bool { false }
    }

    func makeUIView(context: Context) -> Surface {
        let view = Surface(
            frame: .zero, font: Self.font(fontSize), options: TerminalOptions(scrollback: 0))
        view.allowMouseReporting = false
        // Links come marked in the cells, OSC 8 and plain URLs alike; the tap opens them.
        view.linkReporting = .none
        view.nativeBackgroundColor = UIColor(Color(hex: Palette.background))
        view.isScrollEnabled = false
        view.terminalDelegate = context.coordinator
        // SwiftTerm's own taps and long press work its own keyboard and selection, which
        // a renderer has neither of; these work the Pane.
        for recognizer in view.gestureRecognizers ?? []
        where recognizer !== view.panGestureRecognizer && recognizer !== view.pinchGestureRecognizer
        {
            view.removeGestureRecognizer(recognizer)
        }
        let coordinator = context.coordinator
        let pan = UIPanGestureRecognizer(
            target: coordinator, action: #selector(Coordinator.pan(_:)))
        pan.maximumNumberOfTouches = 1
        pan.delegate = coordinator
        view.addGestureRecognizer(pan)
        let tap = UITapGestureRecognizer(
            target: coordinator, action: #selector(Coordinator.tap(_:)))
        view.addGestureRecognizer(tap)
        let press = UILongPressGestureRecognizer(
            target: coordinator, action: #selector(Coordinator.press(_:)))
        press.minimumPressDuration = 0.4
        view.addGestureRecognizer(press)
        let menu = UIEditMenuInteraction(delegate: coordinator)
        view.addInteraction(menu)
        coordinator.menu = menu
        view.accessibilityIdentifier = "terminal"
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

    /// A cell's size as SwiftTerm lays its grid out: the width of "W" and the font's line,
    /// each snapped to the screen's pixels.
    static func cellSize(_ font: UIFont, scale: CGFloat) -> CGSize {
        let line = CTFontGetAscent(font) + CTFontGetDescent(font) + CTFontGetLeading(font)
        let width = ("W" as NSString).size(withAttributes: [.font: font]).width
        return CGSize(
            width: max(1, (width * scale).rounded() / scale),
            height: max(1, ceil(ceil(line) * scale) / scale))
    }

    @MainActor
    final class Coordinator: NSObject, TerminalViewDelegate, UIGestureRecognizerDelegate,
        UIEditMenuInteractionDelegate
    {
        var host: TerminalHost
        var menu: UIEditMenuInteraction?
        private var encoder = ScreenEncoder()
        private var shown: TerminalScreen?
        private var lastGrid: (rows: Int, columns: Int)?
        /// Swipe distance not yet turned into lines, in points.
        private var dragged: CGFloat = 0
        /// Where a long press began, while its finger still moves the selection's end.
        private var anchor: (row: UInt16, column: UInt16)?
        private var reached: (row: UInt16, column: UInt16)?

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

        /// The cell under `point`, on the screen as last drawn.
        private func cell(at point: CGPoint, in view: Surface) -> (row: UInt16, column: UInt16)? {
            guard let screen = shown, screen.rows > 0, screen.columns > 0 else { return nil }
            let size = TerminalHost.cellSize(view.font, scale: view.traitCollection.displayScale)
            let row = min(max(Int(point.y / size.height), 0), Int(screen.rows) - 1)
            let column = min(max(Int(point.x / size.width), 0), Int(screen.columns) - 1)
            return (UInt16(row), UInt16(column))
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

        // Nothing typed into the renderer goes anywhere: input is the page's key row and field.
        nonisolated func send(source: TerminalView, data: ArraySlice<UInt8>) {}
        nonisolated func requestOpenLink(
            source: TerminalView, link: String, params: [String: String]
        ) {}
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

        // MARK: Touches

        /// A swipe is wheel motion at the finger, a line per cell height, a finger moving
        /// down showing what came before.
        @objc func pan(_ gesture: UIPanGestureRecognizer) {
            guard let view = gesture.view as? Surface else { return }
            switch gesture.state {
            case .began:
                dragged = 0
                menu?.dismissMenu()
            case .changed:
                let translation = gesture.translation(in: view).y
                gesture.setTranslation(.zero, in: view)
                dragged += translation
                let line = TerminalHost.cellSize(
                    view.font, scale: view.traitCollection.displayScale
                ).height
                let lines = (dragged / line).rounded(.towardZero)
                if lines != 0, let at = cell(at: gesture.location(in: view), in: view) {
                    dragged -= lines * line
                    host.session.wheel(row: at.row, column: at.column, lines: Int32(lines))
                }
            default:
                dragged = 0
            }
        }

        /// No swipe while a long press drags the selection's end.
        func gestureRecognizerShouldBegin(_ gesture: UIGestureRecognizer) -> Bool {
            anchor == nil
        }

        /// A tap ends a selection, opens a link, or clicks where the program tracks the mouse.
        @objc func tap(_ gesture: UITapGestureRecognizer) {
            guard gesture.state == .ended, let view = gesture.view as? Surface,
                let screen = shown, let at = cell(at: gesture.location(in: view), in: view)
            else { return }
            menu?.dismissMenu()
            if screen.hasSelection {
                host.session.clearSelection()
                return
            }
            let index = Int(at.row) * Int(screen.columns) + Int(at.column)
            if screen.cells.indices.contains(index), let link = screen.cells[index].link,
                let url = URL(string: link)
            {
                host.open(url)
            } else if screen.mouseTracking {
                host.session.click(row: at.row, column: at.column)
            }
        }

        /// A long press selects the word under it; dragging on moves the selection's end,
        /// and letting go offers Copy.
        @objc func press(_ gesture: UILongPressGestureRecognizer) {
            guard let view = gesture.view as? Surface,
                let at = cell(at: gesture.location(in: view), in: view)
            else { return }
            switch gesture.state {
            case .began:
                menu?.dismissMenu()
                anchor = at
                reached = at
                host.session.selectWord(row: at.row, column: at.column)
                UISelectionFeedbackGenerator().selectionChanged()
            case .changed:
                guard let anchor, reached.map({ $0 != at }) ?? true else { return }
                reached = at
                host.session.selectRange(from: anchor, to: at)
            case .ended:
                anchor = nil
                menu?.presentEditMenu(
                    with: UIEditMenuConfiguration(
                        identifier: nil, sourcePoint: gesture.location(in: view)))
            default:
                anchor = nil
            }
        }

        func editMenuInteraction(
            _ interaction: UIEditMenuInteraction, menuFor configuration: UIEditMenuConfiguration,
            suggestedActions: [UIMenuElement]
        ) -> UIMenu? {
            UIMenu(children: [
                UIAction(title: String(localized: "Copy"), image: UIImage(systemName: "doc.on.doc"))
                {
                    [weak self] _ in
                    self?.host.session.copySelection()
                }
            ])
        }
    }
}
