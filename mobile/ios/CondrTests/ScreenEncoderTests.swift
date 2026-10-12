import CondrKit
import SwiftTerm
import Testing

@testable import Condr

/// The encoder's bytes, parsed by SwiftTerm's own emulator, give back the Server's cells.
struct ScreenEncoderTests {
    nonisolated final class Silent: TerminalDelegate {
        func send(source: Terminal, data: ArraySlice<UInt8>) {}
        func notify(source: Terminal, title: String, body: String) {}
        func progressReport(source: Terminal, report: Terminal.ProgressReport) {}
        func programStatusChanged(source: Terminal, records: [TerminalProgramStatus]) {}
        func kittyClipboardRequestPermission(
            source: Terminal, request: KittyClipboardPermissionRequest,
            completion: @escaping @Sendable (KittyClipboardPermissionResult) -> Void
        ) {}
    }

    static let background: UInt32 = 0x0d1117
    static let foreground: UInt32 = 0xc9d1d9

    static func cell(
        _ text: String, fg: UInt32 = foreground, bg: UInt32 = background, bold: Bool = false,
        link: String? = nil
    ) -> ScreenCell {
        ScreenCell(
            text: text, foreground: fg, background: bg, bold: bold, italic: false, underline: false,
            link: link)
    }

    static let blank = cell(" ")

    /// A screen from rows of cells, each padded to `columns`.
    static func screen(
        _ rows: [[ScreenCell]], columns: Int, cursor: ScreenCursor? = nil, revision: UInt64 = 1,
        scrolled: UInt32 = 0
    ) -> TerminalScreen {
        let cells = rows.flatMap { row in row + Array(repeating: blank, count: columns - row.count)
        }
        return TerminalScreen(
            revision: revision, rows: UInt16(rows.count), columns: UInt16(columns),
            scrolledLines: scrolled,
            cells: cells, cursor: cursor, background: background, mouseTracking: false)
    }

    static func text(_ string: String, fg: UInt32 = foreground, bold: Bool = false) -> [ScreenCell]
    {
        string.map { cell(String($0), fg: fg, bold: bold) }
    }

    /// Feeds each screen's bytes into one emulator of `rows` × `columns`.
    static func render(_ screens: [TerminalScreen], rows: Int, columns: Int) -> (
        Terminal, [[UInt8]]
    ) {
        let delegate = Silent()
        let terminal = Terminal(
            delegate: delegate, options: TerminalOptions(cols: columns, rows: rows, scrollback: 0))
        var encoder = ScreenEncoder()
        var frames: [[UInt8]] = []
        for screen in screens {
            let bytes = encoder.encode(screen, rows: rows, columns: columns)
            terminal.feed(byteArray: bytes)
            frames.append(bytes)
        }
        return (terminal, frames)
    }

    static func textAt(_ terminal: Terminal, _ row: Int) -> String {
        (0..<terminal.cols).map { column in
            let text = terminal.getCharData(col: column, row: row)?.getText() ?? ""
            return text.unicodeScalars.allSatisfy({ $0.value < 0x20 }) ? " " : text
        }.joined()
    }

    @Test func aScreenComesBackCellForCell() {
        let screen = Self.screen(
            [
                Self.text("● ", fg: 0x3fb950) + Self.text("Bash", fg: 0xffffff, bold: true)
                    + Self.text("(ls)"),
                Self.text("  ⎿  ", fg: 0x6e7681) + Self.text("ok"),
            ], columns: 12, cursor: ScreenCursor(row: 1, column: 7, color: 0))
        let (terminal, _) = Self.render([screen], rows: 3, columns: 12)

        #expect(Self.textAt(terminal, 0) == "● Bash(ls)  ")
        #expect(Self.textAt(terminal, 1) == "  ⎿  ok     ")
        #expect(Self.textAt(terminal, 2) == "            ")
        let bash = terminal.getCharData(col: 2, row: 0)
        #expect(bash?.attribute.fg == .trueColor(red: 0xff, green: 0xff, blue: 0xff))
        #expect(bash?.attribute.style.contains(.bold) == true)
        let dot = terminal.getCharData(col: 0, row: 0)
        #expect(dot?.attribute.fg == .trueColor(red: 0x3f, green: 0xb9, blue: 0x50))
        #expect(dot?.attribute.style.contains(.bold) == false)
        #expect(terminal.buffer.x == 7)
        #expect(terminal.buffer.y == 1)
    }

    @Test func aWideCharacterKeepsWhatFollowsInPlace() {
        // The Server's grid: 漢 takes two cells, the second empty.
        let row = [Self.cell("漢"), Self.cell(""), Self.cell("x"), Self.cell("y")]
        let screen = Self.screen([row], columns: 6)
        let (terminal, _) = Self.render([screen], rows: 1, columns: 6)
        #expect(terminal.getCharData(col: 0, row: 0)?.getText() == "漢")
        #expect(terminal.getCharData(col: 2, row: 0)?.getText() == "x")
        #expect(terminal.getCharData(col: 3, row: 0)?.getText() == "y")
    }

    @Test func onlyTheChangedRowIsSentAgain() {
        let first = Self.screen(
            [Self.text("one"), Self.text("two"), Self.text("three")], columns: 8)
        let second = Self.screen(
            [Self.text("one"), Self.text("TWO"), Self.text("three")], columns: 8,
            cursor: ScreenCursor(row: 2, column: 5, color: 0), revision: 2)
        let (terminal, frames) = Self.render([first, second], rows: 3, columns: 8)

        #expect(Self.textAt(terminal, 1) == "TWO     ")
        #expect(Self.textAt(terminal, 0) == "one     ")
        let bytes = String(decoding: frames[1], as: UTF8.self)
        #expect(
            bytes.components(separatedBy: ";1H").count == 2,
            "one row was positioned: \(bytes.debugDescription)")
        #expect(!bytes.contains("\u{1b}[2J"))
        #expect(bytes.hasSuffix("\u{1b}[3;6H\u{1b}[?25h"))
    }

    @Test func theSameScreenAgainSendsOnlyTheCursor() {
        let screen = Self.screen(
            [Self.text("same")], columns: 8, cursor: ScreenCursor(row: 0, column: 4, color: 0))
        let (_, frames) = Self.render([screen, screen], rows: 2, columns: 8)
        #expect(
            String(decoding: frames[1], as: UTF8.self)
                == "\u{1b}[?25l\u{1b}[0m\u{1b}[1;5H\u{1b}[?25h")
    }

    @Test func aNewSizeRedrawsEverything() {
        let small = Self.screen([Self.text("ab")], columns: 4)
        let wide = Self.screen([Self.text("abcdef")], columns: 8, revision: 2)
        let (terminal, frames) = Self.render([small, wide], rows: 2, columns: 8)
        #expect(String(decoding: frames[1], as: UTF8.self).contains("\u{1b}[2J"))
        #expect(Self.textAt(terminal, 0) == "abcdef  ")
    }

    @Test func aScreenWiderThanTheRendererNeverWraps() {
        // Frames at the Server's size arrive before the phone's Resize lands.
        let screen = Self.screen([Self.text("0123456789"), Self.text("second")], columns: 10)
        let (terminal, _) = Self.render([screen], rows: 2, columns: 6)
        #expect(Self.textAt(terminal, 0) == "012345")
        #expect(Self.textAt(terminal, 1) == "second")
    }

    @Test func aHiddenCursorStaysHidden() {
        let screen = Self.screen([Self.text("x")], columns: 4, cursor: nil)
        let (_, frames) = Self.render([screen], rows: 1, columns: 4)
        let bytes = String(decoding: frames[0], as: UTF8.self)
        #expect(bytes.hasPrefix("\u{1b}[?25l"))
        #expect(!bytes.contains("\u{1b}[?25h"))
    }

    @Test func aLinkTravelsWithItsCells() {
        let row =
            Self.text("see ") + "condr.dev".map { Self.cell(String($0), link: "https://condr.dev") }
        let screen = Self.screen([row], columns: 16)
        let (terminal, frames) = Self.render([screen], rows: 1, columns: 16)
        #expect(terminal.getCharData(col: 4, row: 0)?.hasPayload == true)
        #expect(terminal.getCharData(col: 0, row: 0)?.hasPayload == false)
        #expect(
            String(decoding: frames[0], as: UTF8.self).contains(
                "\u{1b}]8;;https://condr.dev\u{1b}\\"))
    }
}
