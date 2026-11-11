# The Interface Takes Its Colours from the Terminal Colour Scheme

Status: accepted 2026-11-11. Amends ADR 0032's sentence on the code views' background for the Default Highlight theme, and adds a sending of ADR 0054's `TerminalColors` on every mode change.

The GUI had three colour sources that knew nothing of each other: GPUI Kit's default light or dark theme for the interface, a terminal colour scheme for the Panes, and a Highlight theme for the Preview and Diff Tabs. The default dark interface was near-black `#0a0a0a` around terminals in GitHub's navy `#0d1117`, and a light interface kept that navy terminal. The pane focus border was a fixed `#0078d4` whatever the scheme.

Now the terminal colour scheme is the one source. `condr-gui/src/interface_colors.rs` turns a scheme into a Kit `ThemeConfig`, and Kit derives every token it is not given (button hovers, list rows, charts) from the ones it is. The proportions follow herdr-app's `resolve()` (Apache-2.0):

- The interface's background is the terminals' background, so a Pane's header runs into its terminal.
- The sidebars, title bar, tab bar and status bar are the background mixed 32% towards black, or 60% towards a warm `#e8e8e2` in a light scheme.
- Menus and dialogs are 6% lighter in a dark scheme; secondary surfaces 9% lighter, or 3.5% darker in a light one.
- Borders, hovers and selected rows are a faint wash of white or black (about 9%, 5% and 9% alpha).
- Text is a neutral `#eeeeef` or `#1b1b1e`, not the scheme's foreground, which some schemes tint. Secondary text mixes the background 60% (66% light) of the way to it.
- The accent, used for the focus border, primary buttons, links and the caret, is the scheme's ANSI blue. Text on it is whichever of the background and the text colour contrasts more. Success, warning, danger and info come from the scheme's ANSI green, yellow, red and cyan, so each Agent state keeps its hue and takes the scheme's shade of it.

Text and secondary text keep WCAG 2's 4.5:1 on the background, the bars and the menus: when the mix falls short they are pushed towards white (black in a light scheme) until they reach it. A test holds every built-in scheme to it. Kit has no tertiary text colour, so nothing else needs a floor.

There is a scheme for each mode: `[client.terminal] dark_color_scheme` and `light_color_scheme`, replacing `color_scheme`, which is no longer read. Settings > Appearance offers Dark colors and Light colors, each listing only the built-in schemes of its lightness by the background's luminance: 94 dark, 39 light. Empty picks that mode's default: the built-in dark palette, which a Server answers with before any Client has sent its colours, and GitHub Light Default. A name of the wrong lightness or unknown counts as empty. Both derived themes are installed as Kit's `dark_theme` and `light_theme`, so switching modes, by hand or by following the system, shows the other scheme in the terminals and around them, and sends it to every connected Server as `TerminalColors` so programs asking hear the change. A program that asked before the first send still heard the built-in dark palette, as before.

The Default Highlight theme's Preview and Diff background is the interface's background. A named Highlight theme keeps its own, as ADR 0032 has it.

Not done: user theme files (Kit's `ThemeRegistry` already reads JSON themes, should anyone ask), the Companion, which draws its own colours, and Pane spacing, rounding and border width, which are layout rather than colour.
