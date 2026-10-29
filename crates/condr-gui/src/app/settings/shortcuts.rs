use super::*;

/// A labelled action, as one row of the Shortcuts page.
pub(super) type Shortcut = (&'static str, &'static dyn Action);

/// The fixed shortcuts, grouped as the page shows them. The keys themselves are read
/// from the keymap at render time, so this list can never disagree with `bind_keys`.
pub(in crate::app) const SHORTCUTS: &[(&str, &[Shortcut])] = &[
    (
        "Application",
        &[
            ("Open Settings", &OpenSettings),
            ("Toggle sidebar", &ToggleSidebar),
            ("Toggle Changes & Files", &ToggleChanges),
        ],
    ),
    (
        "Workspaces",
        &[
            ("Next workspace", &NextWorkspace),
            ("Previous workspace", &PreviousWorkspace),
        ],
    ),
    (
        "Tabs",
        &[
            ("New tab", &NewTab),
            ("Next tab", &NextTab),
            ("Previous tab", &PreviousTab),
            ("Go to tab 1", &ActivateTab { index: 0 }),
            ("Go to tab 2", &ActivateTab { index: 1 }),
            ("Go to tab 3", &ActivateTab { index: 2 }),
            ("Go to tab 4", &ActivateTab { index: 3 }),
            ("Go to tab 5", &ActivateTab { index: 4 }),
            ("Go to tab 6", &ActivateTab { index: 5 }),
            ("Go to tab 7", &ActivateTab { index: 6 }),
            ("Go to tab 8", &ActivateTab { index: 7 }),
            ("Go to tab 9", &ActivateTab { index: 8 }),
        ],
    ),
    (
        "Panes",
        &[
            ("Split right", &SplitRight),
            ("Split down", &SplitDown),
            ("Close pane", &ClosePane),
            ("Toggle zoom", &ToggleZoom),
            ("Focus left", &FocusLeft),
            ("Focus right", &FocusRight),
            ("Focus up", &FocusUp),
            ("Focus down", &FocusDown),
            ("Resize left", &ResizeLeft),
            ("Resize right", &ResizeRight),
            ("Resize up", &ResizeUp),
            ("Resize down", &ResizeDown),
        ],
    ),
];

/// The keys a focused terminal takes for the clipboard before any action sees them, so
/// the keymap cannot show them; a test holds them to `terminal_clipboard_shortcut`.
pub(super) const TERMINAL_COPY_KEYS: &[&str] = if cfg!(target_os = "macos") {
    &["cmd-c", "ctrl-insert"]
} else if cfg!(windows) {
    &["ctrl-c", "ctrl-shift-c", "ctrl-insert"]
} else {
    &["ctrl-shift-c", "ctrl-insert"]
};
pub(super) const TERMINAL_PASTE_KEYS: &[&str] = if cfg!(target_os = "macos") {
    &["cmd-v", "shift-insert"]
} else if cfg!(windows) {
    &["ctrl-v", "ctrl-shift-v", "shift-insert"]
} else {
    &["ctrl-shift-v", "shift-insert"]
};

/// What the Server takes as a jump between shell prompts while the shell waits at one
/// (ADR 0055); it accepts Cmd and Ctrl alike, so these are the ones each platform shows.
const PREVIOUS_PROMPT_KEYS: &[&str] = if cfg!(target_os = "macos") {
    &["cmd-up"]
} else {
    &["ctrl-up"]
};
const NEXT_PROMPT_KEYS: &[&str] = if cfg!(target_os = "macos") {
    &["cmd-down"]
} else {
    &["ctrl-down"]
};

/// The key context `bind_keys` registers the shortcuts under.
pub(in crate::app) const SHORTCUT_CONTEXT: &str = "Condr";

/// Read-only: shortcuts are fixed per platform, so this page only shows them.
pub(super) fn shortcuts_page() -> SettingPage {
    SettingPage::new("Shortcuts")
        .icon(IconName::LayoutDashboard)
        .groups(
            SHORTCUTS
                .iter()
                .map(|(title, rows)| {
                    rows.iter().fold(
                        SettingGroup::new().title(*title),
                        |group, (label, action)| {
                            group.item(SettingItem::new(
                                *label,
                                SettingField::render(move |_, window, _| {
                                    div().children(
                                        Kbd::binding_for_action(
                                            *action,
                                            Some(SHORTCUT_CONTEXT),
                                            window,
                                        )
                                        .map(|kbd| kbd.outline()),
                                    )
                                }),
                            ))
                        },
                    )
                })
                .chain([terminal_group()]),
        )
}

/// The terminal's clipboard keys, every key that does each, the image paste that only a
/// remote Pane takes (ADR 0012), and the prompt jumps.
fn terminal_group() -> SettingGroup {
    let rows: [(&str, &'static [&'static str]); 5] = [
        ("Copy selection", TERMINAL_COPY_KEYS),
        ("Paste text", TERMINAL_PASTE_KEYS),
        (
            "Paste image to a remote Device",
            &[crate::app::terminal_input::IMAGE_PASTE_KEY],
        ),
        ("Previous prompt", PREVIOUS_PROMPT_KEYS),
        ("Next prompt", NEXT_PROMPT_KEYS),
    ];
    rows.into_iter().fold(
        SettingGroup::new().title("Terminal"),
        |group, (label, keys)| {
            group.item(SettingItem::new(
                label,
                SettingField::render(move |_, _, _| {
                    h_flex().gap_1().children(keys.iter().map(|key| {
                        Kbd::new(Keystroke::parse(key).expect("a valid keystroke")).outline()
                    }))
                }),
            ))
        },
    )
}
