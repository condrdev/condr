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

/// The key context `bind_keys` registers the shortcuts under.
pub(in crate::app) const SHORTCUT_CONTEXT: &str = "Condr";

/// Read-only: shortcuts are fixed per platform, so this page only shows them.
pub(super) fn shortcuts_page() -> SettingPage {
    SettingPage::new("Shortcuts")
        .icon(IconName::LayoutDashboard)
        .groups(SHORTCUTS.iter().map(|(title, rows)| {
            rows.iter().fold(
                SettingGroup::new().title(*title),
                |group, (label, action)| {
                    group.item(SettingItem::new(
                        *label,
                        SettingField::render(move |_, window, _| {
                            div().children(
                                Kbd::binding_for_action(*action, Some(SHORTCUT_CONTEXT), window)
                                    .map(|kbd| kbd.outline()),
                            )
                        }),
                    ))
                },
            )
        }))
}
