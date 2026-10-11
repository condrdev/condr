import Foundation

// What each demo Pane's program draws, as a function of the grid width, so a pinch that
// changes the columns redraws it as Claude Code or Codex would after a Resize.

struct Style: Equatable {
    var fg: UInt32 = Palette.foreground
    var bg: UInt32?
    var bold = false
    var dim = false
    var italic = false
}

struct Span: Equatable {
    var text: String
    var style: Style
}

typealias Line = [Span]

func t(_ text: String, _ fg: UInt32 = Palette.foreground, bold: Bool = false, dim: Bool = false,
       italic: Bool = false, bg: UInt32? = nil) -> Span {
    Span(text: text, style: Style(fg: fg, bg: bg, bold: bold, dim: dim, italic: italic))
}

private let dimGray: UInt32 = 0x8b949e
private let ruleGray: UInt32 = 0x30363d
private let permission: UInt32 = 0xb1b9f9
private let addBg: UInt32 = 0x12261e
private let removeBg: UInt32 = 0x2d1517
private let promptBg: UInt32 = 0x161b22

struct Screen {
    let columns: Int
    var lines: [Line] = []

    mutating func add(_ spans: Span...) { lines.append(spans) }
    mutating func add(_ spans: Line) { lines.append(spans) }
    mutating func blank() { lines.append([]) }

    /// Word-wraps `text` after `lead`, indenting the following lines under it.
    mutating func wrap(_ text: String, lead: Span? = nil, indent: Int? = nil, _ fg: UInt32 = Palette.foreground,
                       bold: Bool = false, dim: Bool = false, bg: UInt32? = nil) {
        let leadWidth = lead?.text.count ?? 0
        let pad = indent ?? leadWidth
        for (index, row) in Screen.wrap(text, width: max(8, columns - max(leadWidth, pad))).enumerated() {
            var line: Line = []
            if index == 0, let lead { line.append(lead) } else if pad > 0 { line.append(t(String(repeating: " ", count: pad), bg: bg)) }
            line.append(t(row, fg, bold: bold, dim: dim, bg: bg))
            if let bg { line.append(t(String(repeating: " ", count: max(0, columns - Screen.width(line))), bg: bg)) }
            lines.append(line)
        }
    }

    /// Breaks `text` at the last column, as a terminal does for a shell's output.
    mutating func hard(_ text: String, _ fg: UInt32 = Palette.foreground) {
        var rest = Substring(text)
        repeat {
            lines.append([t(String(rest.prefix(columns)), fg)])
            rest = rest.dropFirst(columns)
        } while !rest.isEmpty
    }

    mutating func rule(_ color: UInt32 = ruleGray) {
        add(t(String(repeating: "─", count: columns), color))
    }

    /// A rounded box the width of the grid around what `build` draws inside it.
    mutating func box(_ color: UInt32, _ build: (inout Screen) -> Void) {
        var inner = Screen(columns: columns - 4)
        build(&inner)
        add(t("╭" + String(repeating: "─", count: columns - 2) + "╮", color))
        for line in inner.lines {
            let fill = String(repeating: " ", count: max(0, columns - 4 - Screen.width(line)))
            add([t("│ ", color)] + line + [t(fill + " │", color)])
        }
        add(t("╰" + String(repeating: "─", count: columns - 2) + "╯", color))
    }

    /// A numbered choice as Claude Code draws one, highlighted when selected.
    mutating func option(_ number: Int, _ label: String, note: String? = nil, selected: Bool) {
        let color = selected ? permission : Palette.foreground
        wrap("\(number). \(label)", lead: t(selected ? "❯ " : "  ", permission), indent: 5, color)
        if let note { wrap(note, indent: 5, dimGray) }
    }

    /// A diff row: its line number, sign and code on the change's tint, to the edge.
    mutating func diff(_ number: Int, _ sign: Character, _ code: String) {
        let bg = sign == "+" ? addBg : removeBg
        let text = String("  \(number) \(sign) \(code)".prefix(columns))
        add(t("  \(number) ", dimGray, bg: bg), t("\(sign) ", sign == "+" ? Palette.green : Palette.red, bg: bg),
            t(String(text.dropFirst("  \(number) \(sign) ".count)), bg: bg),
            t(String(repeating: " ", count: max(0, columns - text.count)), bg: bg))
    }

    static func width(_ line: Line) -> Int { line.reduce(0) { $0 + $1.text.count } }

    static func wrap(_ text: String, width: Int) -> [String] {
        var rows: [String] = []
        var row = ""
        for piece in text.split(separator: " ", omittingEmptySubsequences: false) {
            var word = String(piece)
            while word.count > width {
                if !row.isEmpty { rows.append(row); row = "" }
                rows.append(String(word.prefix(width)))
                word = String(word.dropFirst(width))
            }
            if row.isEmpty {
                row = word
            } else if row.count + 1 + word.count <= width {
                row += " " + word
            } else {
                rows.append(row)
                row = word
            }
        }
        rows.append(row)
        return rows
    }
}

enum Script: Equatable {
    case claudeStats, claudeChart, claudePasskeys, codexSidebar, codexIdle, opencodeRime, shell

    /// How many choices its permission or question menu offers, the last one declining.
    /// The phase a script goes to when Esc or Ctrl-C stops it.
    static let interrupted = 4

    var menu: Int? {
        switch self {
        case .claudeStats, .claudeChart, .claudePasskeys: 3
        default: nil
        }
    }

    func render(_ pane: Pane, columns: Int, tick: Int, now: Date) -> [Line] {
        var screen = Screen(columns: columns)
        let seconds = max(1, Int(now.timeIntervalSince(pane.since)))
        switch self {
        case .claudeStats: claudeStats(&screen, pane, seconds, tick)
        case .claudeChart: claudeChart(&screen, pane, seconds, tick)
        case .claudePasskeys: claudePasskeys(&screen, pane, seconds, tick)
        case .codexSidebar: codexSidebar(&screen, pane, seconds, tick)
        case .codexIdle: codexIdle(&screen, pane, seconds, tick)
        case .opencodeRime: opencode(&screen, pane)
        case .shell: shell(&screen, pane)
        }
        return screen.lines
    }

    // MARK: Claude Code

    private func claudeUser(_ s: inout Screen, _ text: String) {
        s.wrap(text, lead: t("> ", dimGray, bg: promptBg), indent: 2, Palette.foreground, bg: promptBg)
        s.blank()
    }

    private func claudeSays(_ s: inout Screen, _ text: String) {
        s.wrap(text, lead: t("● ", Palette.brightWhite), indent: 2)
        s.blank()
    }

    private func claudeTool(_ s: inout Screen, _ name: String, _ argument: String, done: Bool?, _ result: [Span]) {
        let bullet: UInt32 = switch done { case true?: Palette.green; case false?: Palette.red; case nil: dimGray }
        s.wrap("\(name)(\(argument))", lead: t("● ", bullet), indent: 2, Palette.brightWhite, bold: true)
        s.add([t("  ⎿  ", dimGray)] + result)
    }

    private func claudeSpinner(_ s: inout Screen, _ verb: String, _ seconds: Int, _ tick: Int) {
        let frames = ["·", "✢", "✳", "✶", "✻", "✽", "✻", "✶", "✳", "✢"]
        s.blank()
        s.add(t(frames[tick % frames.count] + " ", Palette.claude), t(verb + "… ", Palette.claude),
              t("(\(seconds)s · esc to interrupt)", dimGray))
    }

    private func claudeInput(_ s: inout Screen, cursor: Bool) {
        s.blank()
        s.rule()
        s.add(t("> ", Palette.foreground), cursor ? t(" ", bg: Palette.cursor) : t(" "))
        s.rule()
        s.add(t("  Fable 5.1", Palette.cyan), t(" | ", dimGray), t("high", Palette.cyan), t(" | ", dimGray), t("Ctx 12%", dimGray))
        s.add(t("  ⏵⏵ accept edits on", 0xd2a8ff), t(" (shift+tab to cycle)", dimGray))
    }

    private func claudeConversation(_ s: inout Screen, _ pane: Pane, _ seconds: Int, _ tick: Int) {
        for (index, message) in pane.sent.enumerated() {
            claudeUser(&s, message)
            if index == pane.sent.count - 1, pane.replying {
                claudeSpinner(&s, "Thinking", seconds, tick)
                claudeInput(&s, cursor: false)
                return
            }
            if index == pane.sent.count - 1, pane.interrupted {
                s.add(t("  ⎿  ", dimGray), t("Interrupted · What should Claude do instead?", Palette.red))
                s.blank()
            } else {
                claudeSays(&s, "On it. I'll keep the change small and run the tests before I say it's done.")
            }
        }
        claudeInput(&s, cursor: true)
    }

    private func permissionBox(_ s: inout Screen, _ pane: Pane, title: String, command: String, why: String, again: String) {
        s.blank()
        s.box(permission) { b in
            b.add(t(title, permission, bold: true))
            b.blank()
            b.wrap(command, indent: 2, Palette.brightWhite)
            b.wrap(why, indent: 2, dimGray)
            b.blank()
            b.add(t("Do you want to proceed?"))
            b.option(1, "Yes", selected: pane.selection == 0)
            b.option(2, again, selected: pane.selection == 1)
            b.option(3, "No, and tell Claude what to do differently (esc)", selected: pane.selection == 2)
        }
    }

    private func claudeStats(_ s: inout Screen, _ pane: Pane, _ seconds: Int, _ tick: Int) {
        claudeUser(&s, "Add a statistics page to the session model: count Agents per state so the GUI can show it.")
        claudeSays(&s, "I'll add an AgentStats summary to SessionModel and cover it with tests.")
        claudeTool(&s, "Read", "crates/condr-client/src/model.rs", done: true, [t("Read 412 lines", dimGray)])
        s.blank()
        claudeTool(&s, "Update", "crates/condr-client/src/model.rs", done: true, [t("Updated ", dimGray), t("model.rs", Palette.brightWhite, bold: true), t(" with 38 additions", dimGray)])
        s.diff(212, "+", "pub fn agent_stats(&self) -> AgentStats {")
        s.diff(213, "+", "    let mut stats = AgentStats::default();")
        s.diff(214, "+", "    for tracker in self.agent_trackers.values() {")
        s.diff(215, "+", "        stats.count(tracker.display_state());")
        s.diff(216, "+", "    }")
        s.blank()
        let command = "cargo test -p condr-client"
        switch pane.phase {
        case 0:
            claudeTool(&s, "Bash", command, done: nil, [t("Waiting for permission…", dimGray)])
            permissionBox(&s, pane, title: "Bash command", command: command, why: "Run the client crate's tests",
                          again: "Yes, and don't ask again for cargo test commands in condr")
        case 1:
            claudeTool(&s, "Bash", command, done: nil, [t("Running…", dimGray)])
            claudeSpinner(&s, "Simmering", seconds, tick)
            claudeInput(&s, cursor: false)
        case 2:
            claudeTool(&s, "Bash", command, done: true, [t("running 46 tests", dimGray)])
            s.add(t("     test result: ", dimGray), t("ok", Palette.green, bold: true), t(". 46 passed; 0 failed", dimGray))
            s.blank()
            claudeSays(&s, "All 46 client tests pass. SessionModel now has agent_stats(), one count per state. Want me to show it in the sidebar next?")
            claudeConversation(&s, pane, seconds, tick)
        default:
            claudeTool(&s, "Bash", command, done: false, [t(pane.phase == Script.interrupted ? "Interrupted by user" : "No (tell Claude what to do differently)", Palette.red)])
            s.blank()
            claudeConversation(&s, pane, seconds, tick)
        }
    }

    private func claudeChart(_ s: inout Screen, _ pane: Pane, _ seconds: Int, _ tick: Int) {
        claudeUser(&s, "Add a usage chart to the dashboard home.")
        claudeSays(&s, "The dashboard has no chart library yet, so I'll ask before adding one.")
        let libraries = ["Recharts", "ECharts", "something else"]
        switch pane.phase {
        case 0:
            s.rule(permission)
            s.add(t(" ☐ Chart library ", Palette.background, bold: true, bg: permission))
            s.blank()
            s.wrap("Which chart library should the dashboard use?", Palette.brightWhite, bold: true)
            s.blank()
            s.option(1, "Recharts", note: "Composable React charts, smallest bundle", selected: pane.selection == 0)
            s.option(2, "ECharts", note: "Rich interactions, larger bundle", selected: pane.selection == 1)
            s.option(3, "Type something.", selected: pane.selection == 2)
            s.rule(permission)
            s.wrap("Enter to select · ↑/↓ to navigate · Esc to cancel", dimGray)
        case 1, 2:
            s.wrap("User answered Claude's questions:", lead: t("● ", Palette.brightWhite), indent: 2)
            s.wrap("· Which chart library should the dashboard use? → \(libraries[pane.selection])", lead: t("  ⎿  ", dimGray), indent: 5, dimGray)
            s.blank()
            if pane.phase == 1 {
                claudeTool(&s, "Write", "src/components/UsageChart.tsx", done: nil, [t("Writing…", dimGray)])
                claudeSpinner(&s, "Charting", seconds, tick)
                claudeInput(&s, cursor: false)
            } else {
                claudeTool(&s, "Write", "src/components/UsageChart.tsx", done: true, [t("Wrote 64 lines", dimGray)])
                s.blank()
                claudeSays(&s, "The home page now shows daily usage for the last 30 days with \(libraries[pane.selection]). Run pnpm dev to see it.")
                claudeConversation(&s, pane, seconds, tick)
            }
        default:
            s.add(t("  ⎿  ", dimGray), t("Interrupted · What should Claude do instead?", Palette.red))
            s.blank()
            claudeConversation(&s, pane, seconds, tick)
        }
    }

    private func claudePasskeys(_ s: inout Screen, _ pane: Pane, _ seconds: Int, _ tick: Int) {
        claudeUser(&s, "Migrate sign-in to passkeys and keep passwords as a fallback for a month.")
        claudeSays(&s, "I'll add a Passkey model, the WebAuthn routes and a migration.")
        claudeTool(&s, "Write", "prisma/schema.prisma", done: true, [t("Updated with 21 additions", dimGray)])
        s.blank()
        claudeTool(&s, "Write", "src/auth/passkeys.ts", done: true, [t("Wrote 148 lines", dimGray)])
        s.blank()
        let command = "pnpm prisma migrate dev --name passkeys"
        switch pane.phase {
        case 0:
            claudeTool(&s, "Write", "src/routes/webauthn.ts", done: nil, [t("Writing…", dimGray)])
            claudeSpinner(&s, "Migrating", seconds, tick)
            claudeInput(&s, cursor: false)
        case 1 where pane.agent?.status == .blocked:
            claudeTool(&s, "Bash", command, done: nil, [t("Waiting for permission…", dimGray)])
            permissionBox(&s, pane, title: "Bash command", command: command, why: "Create and apply the passkeys migration",
                          again: "Yes, and don't ask again for pnpm prisma commands")
        case 1:
            claudeTool(&s, "Bash", command, done: nil, [t("Running…", dimGray)])
            claudeSpinner(&s, "Migrating", seconds, tick)
            claudeInput(&s, cursor: false)
        case 2:
            claudeTool(&s, "Bash", command, done: true, [t("Applied migration 20261010_passkeys", dimGray)])
            s.blank()
            claudeSays(&s, "Passkeys are live behind the sign-in page; passwords still work until November 10.")
            claudeConversation(&s, pane, seconds, tick)
        default:
            claudeTool(&s, "Bash", command, done: false, [t(pane.phase == Script.interrupted ? "Interrupted by user" : "No (tell Claude what to do differently)", Palette.red)])
            s.blank()
            claudeConversation(&s, pane, seconds, tick)
        }
    }

    // MARK: Codex

    private func codexUser(_ s: inout Screen, _ text: String) {
        s.wrap(text, lead: t("› ", Palette.brightWhite, bold: true, bg: promptBg), indent: 2, Palette.brightWhite, bg: promptBg)
        s.blank()
    }

    private func codexSays(_ s: inout Screen, _ text: String) {
        s.wrap(text, lead: t("• ", Palette.brightWhite), indent: 2)
        s.blank()
    }

    private func codexInput(_ s: inout Screen, working: (Int, Int)?) {
        if let (seconds, tick) = working {
            let frames = ["◦", "•"]
            s.add(t(frames[(tick / 3) % 2] + " ", Palette.brightWhite), t("Working", Palette.brightWhite, bold: true),
                  t(" (\(seconds)s • esc to interrupt)", dimGray))
            s.blank()
        }
        s.add(t("› ", Palette.brightWhite, bold: true, bg: promptBg), working == nil ? t(" ", bg: Palette.cursor) : t(" ", bg: promptBg),
              t("Ask Codex to do anything", dimGray, bg: promptBg),
              t(String(repeating: " ", count: max(0, s.columns - 27)), bg: promptBg))
        s.blank()
        s.wrap("gpt-6-astra xhigh · ~/Developer/project/condr", indent: 2, dimGray)
    }

    private func codexConversation(_ s: inout Screen, _ pane: Pane, _ seconds: Int, _ tick: Int) {
        for (index, message) in pane.sent.enumerated() {
            codexUser(&s, message)
            if index == pane.sent.count - 1, pane.replying {
                codexInput(&s, working: (seconds, tick))
                return
            }
            if !(index == pane.sent.count - 1 && pane.interrupted) {
                codexSays(&s, "Got it. I'll make that change and run the GUI tests again.")
            }
        }
        codexInput(&s, working: nil)
    }

    private func codexSidebar(_ s: inout Screen, _ pane: Pane, _ seconds: Int, _ tick: Int) {
        codexUser(&s, "Fix the sidebar flicker when an Agent flips between Working and Idle quickly.")
        codexSays(&s, "I'll let the status glyph settle for a beat, so a quick Working → Idle → Working doesn't repaint the row.")
        s.add(t("• ", Palette.brightWhite), t("Explored", Palette.brightWhite, bold: true))
        s.wrap("Read icon.rs, sidebar.rs, tracker.rs", lead: t("  └ ", dimGray), indent: 4, dimGray)
        s.blank()
        s.wrap("crates/condr-gui/src/app/sidebar/icon.rs (+14 -3)", lead: t("• Edited ", Palette.brightWhite, bold: true), indent: 2)
        s.diff(88, "-", "let visual = agent_sidebar_status(state);")
        s.diff(88, "+", "let visual = self.settled(state, now);")
        s.blank()
        if pane.phase == 0 {
            codexInput(&s, working: (seconds, tick))
            return
        }
        if pane.phase == Script.interrupted {
            s.wrap("Conversation interrupted - tell the model what to do differently.", lead: t("■ ", Palette.red), indent: 2, Palette.red)
            s.blank()
            codexConversation(&s, pane, seconds, tick)
            return
        }
        s.add(t("• ", Palette.brightWhite), t("Ran ", Palette.brightWhite, bold: true), t("cargo test -p condr-gui"))
        s.add(t("  └ ", dimGray), t("test result: ", dimGray), t("ok", Palette.green), t(". 215 passed", dimGray))
        s.blank()
        let label = "─ Worked for 1m 12s "
        s.add(t(label + String(repeating: "─", count: max(0, s.columns - label.count)), dimGray))
        s.blank()
        codexSays(&s, "The glyph now waits 300 ms before it changes, so a flip shorter than that never repaints. Tests pass.")
        codexConversation(&s, pane, seconds, tick)
    }

    private func codexIdle(_ s: inout Screen, _ pane: Pane, _ seconds: Int, _ tick: Int) {
        codexUser(&s, "Tighten the copy on the pairing screens.")
        codexSays(&s, "Shortened every pairing screen to one sentence and one action. The Local Network screen now says why before iOS asks.")
        codexConversation(&s, pane, seconds, tick)
    }

    // MARK: OpenCode

    private func opencode(_ s: inout Screen, _ pane: Pane) {
        let accent: UInt32 = 0xfab283
        s.blank()
        s.wrap("Add a double pinyin schema and make it the default.", lead: t("┃  ", accent), indent: 0, Palette.brightWhite)
        s.blank()
        s.wrap("I added double_pinyin_flypy.schema.yaml and set it as the default in default.custom.yaml. Redeploy Rime to pick it up.", lead: t("   "), indent: 3)
        s.blank()
        s.add(t("   ▣ ", accent), t("Build", Palette.brightWhite), t(" · fable-5-1 · 48s", dimGray))
        for message in pane.sent {
            s.blank()
            s.wrap(message, lead: t("┃  ", accent), indent: 0, Palette.brightWhite)
        }
        s.blank()
        s.add(t("┃", accent))
        s.add(t("┃  ", accent), t(" ", bg: Palette.cursor))
        s.add(t("┃", accent))
        s.add(t("   ctrl+p commands", dimGray), t(String(repeating: " ", count: max(1, s.columns - 26))), t("build", accent))
    }

    // MARK: Shell

    private func shell(_ s: inout Screen, _ pane: Pane) {
        let prompt = [t("rocky@macmini ", Palette.green), t("condr", Palette.blue, bold: true), t(" % ")]
        s.hard("Last login: Fri Oct 10 09:12:44 on ttys004", dimGray)
        s.add(prompt + [t("git log --oneline -5")])
        let log = [
            ("5b1fc2e", "test: fix the Linux and Windows builds of tests the client split touched"),
            ("321c6e1", "docs: describe condr-client, condr-mobile and mobile/ in AGENTS.md"),
            ("574365c", "chore: add condr-mobile, the Companion's one library"),
            ("824f67a", "refactor: share the GUI's connection threads through condr-client"),
            ("144eeb3", "refactor: move the Device key and Noise into condr-client"),
        ]
        for (hash, subject) in log {
            var rest = Substring(hash + " " + subject)
            var first = true
            repeat {
                let row = String(rest.prefix(s.columns))
                if first {
                    s.add(t(String(row.prefix(7)), Palette.yellow), t(String(row.dropFirst(7))))
                } else {
                    s.add(t(row))
                }
                first = false
                rest = rest.dropFirst(s.columns)
            } while !rest.isEmpty
        }
        for command in pane.sent {
            s.add(prompt + [t(command)])
            if command.hasPrefix("git status") {
                s.hard("On branch main")
                s.hard("Your branch is up to date with 'origin/main'.")
                s.blank()
                s.hard("nothing to commit, working tree clean")
            } else if command.hasPrefix("ls") {
                s.hard("AGENTS.md  Cargo.lock  Cargo.toml  CONTEXT.md  README.md  assets  crates  docs  mobile  script  website", Palette.brightBlue)
            }
        }
        s.add(prompt + [t(" ", bg: Palette.cursor)])
    }
}
