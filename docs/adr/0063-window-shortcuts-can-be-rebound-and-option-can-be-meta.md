# Window Shortcuts Can Be Rebound and Option Can Be Meta

Status: accepted 2026-11-12.

Every window shortcut was fixed per platform, and on macOS Option always typed the character macOS composes, so Option+B gave `∫` and a shell's word movement had no key. Both came up together on the roadmap, and both are this machine's GUI preferences in `[client]`.

## Rebinding

The 31 actions on Settings > Shortcuts can be rebound: Open Settings, the two sidebars, the Workspace, Tab and Go to tab 1–9 shortcuts, and the Pane splits, close, zoom, focus and resize. Each has a kebab-case name, `split-right` or `go-to-tab-3`, in `[client.keybindings]`, whose value is a list of keys in GPUI's syntax:

```toml
[client.keybindings]
split-right = ["cmd-d", "cmd-\\"]
toggle-zoom = []
```

An entry replaces that action's defaults on this platform; `[]` leaves it no key; an action without one keeps its defaults. The file only holds what differs, so a default added in a later release still reaches someone who changed another shortcut. herdr-app writes the whole keymap on first run and freezes it from then on; this was chosen against that.

A key belongs to one action. An entry takes its keys from any action's defaults without saying so, so `toggle-zoom = ["cmd-d"]` needs no second line to take `cmd-d` from Split right. Two entries naming the same key keep it for the one earlier on the page and tell the later one so, as the herdr TUI does. Recording a key in Settings that another action has moves it, and writes both entries.

Each key must hold Cmd, Ctrl or Alt (Option), or be an F-key: anything else is a key the terminal needs for typing. On macOS Cmd+Q, Cmd+H, Cmd+Option+H, Cmd+M and Cmd+Ctrl+F belong to the system and the app menu, and no action can take them. One key per shortcut, no sequences: the terminal's own path matches keys one at a time, and a pending prefix would need a timeout and a way to give the prefix back to the program.

What stays fixed: the terminal's copy and paste keys, whose meaning depends on a selection and on whether the Pane is remote (ADR 0012); the prompt jumps the Server decides (ADR 0055); Save in the Preview Tab; and the keys of dialogs. A rebound key is taken before the terminal sees it, as a default is, whatever it is: someone who gives Ctrl+C to an action has chosen to.

A malformed part of an entry is left out on its own and the rest kept: an unknown key, one without a modifier, a reserved one, a value that is not a list (which keeps the defaults), or a key another entry has. Each reason shows under its row on the Shortcuts page and in the log; an entry naming no action is listed at the top of the page. Nothing toasts: ADR 0020 puts a validation failure under its field. The file is read when the GUI starts, like every other `[client]` key; a change made in Settings takes effect at once.

### Settings

Each row of the Shortcuts page shows its keys, each with a remove button, and a button that records one more: the next key pressed in the Settings window is taken before any binding or listener sees it, through GPUI's keystroke interceptor, so recording Cmd+W does not close anything and Escape cancels rather than closing the window. A row that differs from the defaults has Kit's reset button, which drops its entry.

### How it is installed

`condr-gui/src/app/keymap.rs` holds the table of actions and defaults and resolves `[client.keybindings]` into a `Keymap`. Installing one replaces exactly the bindings a previous install added, which carry their own `KeyBindingMetaIndex`, and keeps Kit's and every other binding. The terminal, which sees keys before the keymap, asks the same `Keymap`. The macOS menus are made again with each install: an item keeps the key equivalent it was made with, and macOS answers that key.

## Option as Meta

`[client.terminal] macos_option_as_meta` is `off`, `left`, `right` or `both`, set under Settings > Shortcuts on macOS. `off`, the default, is what Condr did before and what Terminal.app, iTerm2 and Ghostty do. The others send an Option chord from that key as Meta, Esc and the key, so Option+B moves back a word; the other Option key keeps typing é or `@` on layouts that put it there. Windows and Linux are unchanged, where Alt always is Meta. No `auto` that guesses from the keyboard layout, as herdr-app has: it is a rule a person cannot see.

GPUI's modifiers have one Alt, so the side comes from the window server: `CGEventSourceFlagsState` still carries `NX_DEVICELALTKEYMASK` and `NX_DEVICERALTKEYMASK`. That is the one call `objc2-core-graphics` is declared for, on the version Kit already builds for GPUI's macOS backend. herdr-app gets the side by patching GPUI's `Keystroke`; Condr declares only Kit's crates.

Where Option is Meta, the GUI ignores the character Option composed and sends the key it was pressed with, Alt held; the Server's encoder already treats Alt as Alt. GPUI hands an Option chord to the app before AppKit's text input whenever no IME composition is under way, and the terminal tells GPUI it does not want printable keys sent to an IME first, so the key never starts a dead-key composition: Option+E sends Esc e, not an accent waiting for its letter.
