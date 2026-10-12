import CondrKit

/// Turns the Server's screens into the bytes that bring a SwiftTerm buffer from one to the
/// next: only the rows that changed, each positioned, cleared and written as runs of one
/// style, with the cursor placed last. The renderer parses them back into the same cells.
nonisolated struct ScreenEncoder {
    private var previous: TerminalScreen?

    /// The next screen is drawn whole: the renderer's buffer no longer matches the last one.
    mutating func reset() {
        previous = nil
    }

    /// The bytes for `screen` on a renderer of `rows` × `columns`; cells past either are not
    /// sent, so a screen at the Server's old size never wraps.
    mutating func encode(_ screen: TerminalScreen, rows: Int, columns: Int) -> [UInt8] {
        var out = "\u{1b}[?25l"
        let width = Int(screen.columns)
        let visibleRows = min(Int(screen.rows), rows)
        let visibleColumns = min(width, columns)
        let whole =
            previous.map {
                $0.rows != screen.rows || $0.columns != screen.columns
                    || $0.cells.count != screen.cells.count
            }
            ?? true
        if whole { out += "\u{1b}[0m\u{1b}[2J" }
        for row in 0..<visibleRows {
            let range = (row * width)..<min((row + 1) * width, screen.cells.count)
            if !whole, let previous, previous.cells[range] == screen.cells[range] { continue }
            out += Self.row(
                screen.cells[range].prefix(visibleColumns), at: row, background: screen.background)
        }
        out += "\u{1b}[0m"
        if let cursor = screen.cursor, Int(cursor.row) < visibleRows,
            Int(cursor.column) < visibleColumns
        {
            out += "\u{1b}[\(cursor.row + 1);\(cursor.column + 1)H\u{1b}[?25h"
        }
        previous = screen
        return Array(out.utf8)
    }

    /// One row: cleared, then its cells in runs of one style. A run starts with a cursor
    /// move unless the previous run was plain ASCII, whose width the renderer agrees on;
    /// after anything wider the next run is placed again, so a wide character's empty
    /// second cell, which is skipped, never shifts what follows.
    static func row(_ cells: ArraySlice<ScreenCell>, at row: Int, background: UInt32) -> String {
        var out = "\u{1b}[\(row + 1);1H\u{1b}[0m\u{1b}[2K"
        var column = cells.startIndex
        var at: Int? = cells.startIndex
        var style: ScreenCell?
        var linked = false
        for cell in cells {
            defer { column += 1 }
            let blank = cell.text == " " && cell.background == background && !cell.underline
            if cell.text.isEmpty || blank {
                // Nothing is written, so the cursor stays where it was; after a wide
                // character's second cell it is somewhere past, which the next run resets.
                if style != nil { out += close(&style, &linked) }
                if cell.text.isEmpty { at = nil }
                continue
            }
            if at != column {
                if style != nil { out += close(&style, &linked) }
                out += "\u{1b}[\(row + 1);\(column - cells.startIndex + 1)H"
            }
            if style.map({ !sameStyle($0, cell) }) ?? true {
                out += close(&style, &linked)
                out += sgr(cell)
                if let link = cell.link {
                    out += "\u{1b}]8;;\(link)\u{1b}\\"
                    linked = true
                }
                style = cell
            }
            out += cell.text
            at = cell.text.utf8.count == 1 ? column + 1 : nil
        }
        out += close(&style, &linked)
        return out
    }

    private static func sameStyle(_ a: ScreenCell, _ b: ScreenCell) -> Bool {
        a.foreground == b.foreground && a.background == b.background && a.bold == b.bold
            && a.italic == b.italic
            && a.underline == b.underline && a.link == b.link
    }

    private static func sgr(_ cell: ScreenCell) -> String {
        var codes = ["0"]
        if cell.bold { codes.append("1") }
        if cell.italic { codes.append("3") }
        if cell.underline { codes.append("4") }
        codes.append("38;2;\(rgb(cell.foreground))")
        codes.append("48;2;\(rgb(cell.background))")
        return "\u{1b}[\(codes.joined(separator: ";"))m"
    }

    private static func rgb(_ color: UInt32) -> String {
        "\((color >> 16) & 0xff);\((color >> 8) & 0xff);\(color & 0xff)"
    }

    private static func close(_ style: inout ScreenCell?, _ linked: inout Bool) -> String {
        guard style != nil else { return "" }
        style = nil
        defer { linked = false }
        return (linked ? "\u{1b}]8;;\u{1b}\\" : "") + "\u{1b}[0m"
    }
}
