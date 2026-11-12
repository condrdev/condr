//! The window shortcuts (ADR 0063): each action's keys on this platform, what
//! `[client.keybindings]` changes about them, and the keymap that results.

use super::*;

/// One window action a person can rebind, by the name `[client.keybindings]` knows it by.
pub(super) struct Shortcut {
    pub name: &'static str,
    pub label: &'static str,
    pub action: &'static dyn Action,
    /// Its keys on this platform while nothing overrides them; tooltips show the first.
    pub defaults: &'static [&'static str],
}

const MAC: bool = cfg!(target_os = "macos");

const OS: OperatingSystem = if MAC {
    OperatingSystem::Mac
} else if cfg!(windows) {
    OperatingSystem::Windows
} else {
    OperatingSystem::Linux
};

/// The key context the window shortcuts are bound in.
pub(in crate::app) const SHORTCUT_CONTEXT: &str = "Condr";

/// Held with a digit to pick the Nth Tab: Cmd on macOS and Alt elsewhere.
#[cfg(test)]
pub(super) const TAB_MODIFIER: &str = if MAC { "cmd" } else { "alt" };

const fn on(
    mac: &'static [&'static str],
    other: &'static [&'static str],
) -> &'static [&'static str] {
    if MAC { mac } else { other }
}

const fn shortcut(
    name: &'static str,
    label: &'static str,
    action: &'static dyn Action,
    defaults: &'static [&'static str],
) -> Shortcut {
    Shortcut {
        name,
        label,
        action,
        defaults,
    }
}

/// Every window shortcut, grouped as the Shortcuts page shows them. The order is also
/// the precedence when two entries of `[client.keybindings]` name the same key.
pub(super) const GROUPS: &[(&str, &[Shortcut])] = &[
    (
        "Application",
        &[
            shortcut(
                "open-settings",
                "Open Settings",
                &OpenSettings,
                on(&["cmd-,"], &["ctrl-,"]),
            ),
            // VS Code's primary and secondary sidebar chords; elsewhere Ctrl+B is the tmux
            // prefix and readline's backward-char, so Shift joins it.
            shortcut(
                "toggle-sidebar",
                "Toggle sidebar",
                &ToggleSidebar,
                on(&["cmd-b"], &["ctrl-shift-b"]),
            ),
            shortcut(
                "toggle-changes",
                "Toggle Changes & Files",
                &ToggleChanges,
                on(&["cmd-alt-b"], &["ctrl-alt-b"]),
            ),
        ],
    ),
    (
        "Workspaces",
        // Workspaces stack vertically in the sidebar, so Up and Down walk them.
        &[
            shortcut(
                "next-workspace",
                "Next workspace",
                &NextWorkspace,
                on(&["cmd-shift-down"], &["ctrl-shift-down"]),
            ),
            shortcut(
                "previous-workspace",
                "Previous workspace",
                &PreviousWorkspace,
                on(&["cmd-shift-up"], &["ctrl-shift-up"]),
            ),
        ],
    ),
    (
        "Tabs",
        &[
            shortcut(
                "new-tab",
                "New tab",
                &NewTab,
                on(&["cmd-t"], &["ctrl-shift-t"]),
            ),
            shortcut(
                "next-tab",
                "Next tab",
                &NextTab,
                on(&["cmd-}", "ctrl-tab"], &["ctrl-tab"]),
            ),
            shortcut(
                "previous-tab",
                "Previous tab",
                &PreviousTab,
                on(&["cmd-{", "ctrl-shift-tab"], &["ctrl-shift-tab"]),
            ),
            shortcut(
                "go-to-tab-1",
                "Go to tab 1",
                &ActivateTab { index: 0 },
                on(&["cmd-1"], &["alt-1"]),
            ),
            shortcut(
                "go-to-tab-2",
                "Go to tab 2",
                &ActivateTab { index: 1 },
                on(&["cmd-2"], &["alt-2"]),
            ),
            shortcut(
                "go-to-tab-3",
                "Go to tab 3",
                &ActivateTab { index: 2 },
                on(&["cmd-3"], &["alt-3"]),
            ),
            shortcut(
                "go-to-tab-4",
                "Go to tab 4",
                &ActivateTab { index: 3 },
                on(&["cmd-4"], &["alt-4"]),
            ),
            shortcut(
                "go-to-tab-5",
                "Go to tab 5",
                &ActivateTab { index: 4 },
                on(&["cmd-5"], &["alt-5"]),
            ),
            shortcut(
                "go-to-tab-6",
                "Go to tab 6",
                &ActivateTab { index: 5 },
                on(&["cmd-6"], &["alt-6"]),
            ),
            shortcut(
                "go-to-tab-7",
                "Go to tab 7",
                &ActivateTab { index: 6 },
                on(&["cmd-7"], &["alt-7"]),
            ),
            shortcut(
                "go-to-tab-8",
                "Go to tab 8",
                &ActivateTab { index: 7 },
                on(&["cmd-8"], &["alt-8"]),
            ),
            shortcut(
                "go-to-tab-9",
                "Go to tab 9",
                &ActivateTab { index: 8 },
                on(&["cmd-9"], &["alt-9"]),
            ),
        ],
    ),
    (
        "Panes",
        &[
            shortcut(
                "split-right",
                "Split right",
                &SplitRight,
                on(&["cmd-d"], &["alt-shift-="]),
            ),
            shortcut(
                "split-down",
                "Split down",
                &SplitDown,
                on(&["cmd-shift-d"], &["alt-shift--"]),
            ),
            shortcut(
                "close-pane",
                "Close pane",
                &ClosePane,
                on(&["cmd-w"], &["ctrl-shift-w"]),
            ),
            shortcut(
                "toggle-zoom",
                "Toggle zoom",
                &ToggleZoom,
                on(&["cmd-shift-enter"], &["alt-shift-enter"]),
            ),
            shortcut(
                "focus-left",
                "Focus left",
                &FocusLeft,
                on(&["cmd-alt-left"], &["alt-left"]),
            ),
            shortcut(
                "focus-right",
                "Focus right",
                &FocusRight,
                on(&["cmd-alt-right"], &["alt-right"]),
            ),
            shortcut(
                "focus-up",
                "Focus up",
                &FocusUp,
                on(&["cmd-alt-up"], &["alt-up"]),
            ),
            shortcut(
                "focus-down",
                "Focus down",
                &FocusDown,
                on(&["cmd-alt-down"], &["alt-down"]),
            ),
            shortcut(
                "resize-left",
                "Resize left",
                &ResizeLeft,
                on(&["cmd-ctrl-left"], &["alt-shift-left"]),
            ),
            shortcut(
                "resize-right",
                "Resize right",
                &ResizeRight,
                on(&["cmd-ctrl-right"], &["alt-shift-right"]),
            ),
            shortcut(
                "resize-up",
                "Resize up",
                &ResizeUp,
                on(&["cmd-ctrl-up"], &["alt-shift-up"]),
            ),
            shortcut(
                "resize-down",
                "Resize down",
                &ResizeDown,
                on(&["cmd-ctrl-down"], &["alt-shift-down"]),
            ),
        ],
    ),
];

/// The chords every macOS app answers, which the app menu shows. No setting moves them,
/// and no shortcut may take them.
const SYSTEM: &[(&str, &dyn Action)] = if MAC {
    &[
        ("cmd-q", &QuitApp),
        ("cmd-h", &HideApp),
        ("cmd-alt-h", &HideOtherApps),
        ("cmd-m", &MinimizeWindow),
        ("cmd-ctrl-f", &ToggleFullScreen),
    ]
} else {
    &[]
};

/// Every shortcut, in `GROUPS` order; a shortcut's index is its place here.
pub(super) fn all() -> impl Iterator<Item = &'static Shortcut> + Clone {
    GROUPS.iter().flat_map(|(_, shortcuts)| shortcuts.iter())
}

fn parse(source: &str) -> Option<Keystroke> {
    let mut key = Keystroke::parse_with_os(source, OS).ok()?;
    key.key_char = None;
    Some(key)
}

fn default_keys(shortcut: &Shortcut) -> impl Iterator<Item = Keystroke> + '_ {
    shortcut.defaults.iter().filter_map(|key| parse(key))
}

/// How `key` is written in `[client.keybindings]`.
pub(super) fn written(key: &Keystroke) -> String {
    key.unparse(OS)
}

/// Why `key` cannot be a window shortcut, if it cannot: one with no Cmd, Ctrl or Alt
/// would take a key the terminal needs for typing (F-keys aside).
pub(super) fn refusal(key: &Keystroke) -> Option<&'static str> {
    let modifiers = key.modifiers;
    let function_key = key
        .key
        .strip_prefix('f')
        .and_then(|number| number.parse::<u8>().ok())
        .is_some_and(|number| (1..=24).contains(&number));
    if !(modifiers.control || modifiers.alt || modifiers.platform || function_key) {
        return Some(if MAC {
            "needs Cmd, Ctrl or Option"
        } else {
            "needs Ctrl or Alt"
        });
    }
    SYSTEM
        .iter()
        .any(|(system, _)| parse(system).as_ref() == Some(key))
        .then_some("belongs to macOS")
}

/// What a typed key is as a shortcut: the platform's way of reporting a shifted symbol
/// made the same as the default bindings write it, and the produced text left out.
pub(super) fn normalize(stroke: &Keystroke) -> Keystroke {
    let mut normalized = stroke.clone();
    let modifiers = stroke.modifiers;
    if !MAC && !modifiers.control && !modifiers.platform && modifiers.alt {
        // Windows GPUI turns Shift+= / Shift+- into + / _ and clears Shift.
        let key_char = stroke.key_char.as_deref();
        if stroke.key == "+" || key_char == Some("+") {
            normalized.key = "=".into();
            normalized.modifiers.shift = true;
        } else if stroke.key == "_" || key_char == Some("_") {
            normalized.key = "-".into();
            normalized.modifiers.shift = true;
        }
    }
    if MAC && modifiers.platform && !modifiers.control && !modifiers.alt {
        // GPUI may fold Cmd+Shift+] / [ into Cmd+} / {.
        match stroke.key.as_str() {
            "]" if modifiers.shift => normalized.key = "}".into(),
            "[" if modifiers.shift => normalized.key = "{".into(),
            _ => {}
        }
        if matches!(normalized.key.as_str(), "{" | "}") {
            normalized.modifiers.shift = false;
        }
    }
    // IME text is handled before this; only the chord counts, so text a chord produced
    // cannot strip Ctrl or Alt into another shortcut.
    normalized.key_char = None;
    normalized
}

/// Each shortcut's keys once `[client.keybindings]` has had its say, and what of it was
/// left out and why.
#[derive(Clone, Debug)]
pub(super) struct Keymap {
    /// Indexed like `all()`.
    pub keys: Vec<Vec<Keystroke>>,
    /// Indexed like `all()`: why a key `[client.keybindings]` gave that shortcut, or the
    /// entry itself, was left out.
    pub errors: Vec<Vec<String>>,
    /// Entries of `[client.keybindings]` that name no shortcut.
    pub unknown: Vec<String>,
}

impl Default for Keymap {
    fn default() -> Self {
        Self::resolve(&toml::Table::new())
    }
}

impl Keymap {
    /// An entry replaces its shortcut's default keys; `[]` leaves it none. The keys an
    /// entry names are taken from any default that has them, and from any later entry in
    /// `all()` order, which is told so.
    pub(super) fn resolve(table: &toml::Table) -> Self {
        let shortcuts = all().collect::<Vec<_>>();
        let mut errors = vec![Vec::new(); shortcuts.len()];
        let mut overrides: Vec<Option<Vec<Keystroke>>> = vec![None; shortcuts.len()];
        let mut unknown = Vec::new();
        for (name, value) in table {
            let Some(index) = shortcuts.iter().position(|shortcut| shortcut.name == name) else {
                unknown.push(name.clone());
                continue;
            };
            let Some(list) = value.as_array() else {
                errors[index].push(format!("Takes a list of keys, not {value}."));
                continue;
            };
            let mut keys = Vec::new();
            for item in list {
                let Some(source) = item.as_str() else {
                    errors[index].push(format!("{item} is not a key."));
                    continue;
                };
                let Some(key) = parse(source) else {
                    errors[index].push(format!("{source} is not a key."));
                    continue;
                };
                if let Some(why) = refusal(&key) {
                    errors[index].push(format!("{source} {why}."));
                } else if !keys.contains(&key) {
                    keys.push(key);
                }
            }
            overrides[index] = Some(keys);
        }
        let mut taken: Vec<(Keystroke, usize)> = Vec::new();
        for (index, keys) in overrides.iter_mut().enumerate() {
            let Some(keys) = keys else { continue };
            keys.retain(|key| match taken.iter().find(|(owned, _)| owned == key) {
                Some((_, owner)) => {
                    errors[index].push(format!(
                        "{} is already {}.",
                        written(key),
                        shortcuts[*owner].label
                    ));
                    false
                }
                None => {
                    taken.push((key.clone(), index));
                    true
                }
            });
        }
        let keys = overrides
            .into_iter()
            .zip(&shortcuts)
            .map(|(keys, shortcut)| {
                keys.unwrap_or_else(|| {
                    default_keys(shortcut)
                        .filter(|key| !taken.iter().any(|(owned, _)| owned == key))
                        .collect()
                })
            })
            .collect();
        Self {
            keys,
            errors,
            unknown,
        }
    }

    /// The window action a key typed into a terminal stands for. The terminal sees keys
    /// before any binding does, so it asks here first.
    pub(super) fn action_for(&self, stroke: &Keystroke) -> Option<Box<dyn Action>> {
        if stroke.modifiers.function {
            return None;
        }
        let stroke = normalize(stroke);
        all()
            .zip(&self.keys)
            .find(|(_, keys)| keys.contains(&stroke))
            .map(|(shortcut, _)| shortcut.action)
            .or_else(|| {
                SYSTEM
                    .iter()
                    .find(|(key, _)| parse(key).as_ref() == Some(&stroke))
                    .map(|(_, action)| *action)
            })
            .map(|action| action.boxed_clone())
    }
}

/// Marks the bindings `install` puts in the keymap, so the next one replaces exactly
/// those and keeps Kit's and every other binding.
const WINDOW_BINDING: KeyBindingMetaIndex = KeyBindingMetaIndex(u32::MAX);

/// Makes `keymap` the window shortcuts every window answers, and the app menu's.
pub(super) fn install(keymap: &Keymap, cx: &mut App) {
    let context = Some(Rc::new(
        KeyBindingContextPredicate::parse(SHORTCUT_CONTEXT).expect("a valid key context"),
    ));
    // The last binding added for an action is the one tooltips and the menu show.
    let window_bindings = all()
        .zip(&keymap.keys)
        .flat_map(|(shortcut, keys)| keys.iter().rev().map(|key| (written(key), shortcut.action)))
        .chain(
            SYSTEM
                .iter()
                .map(|(key, action)| ((*key).to_owned(), *action)),
        )
        .map(|(key, action)| {
            KeyBinding::load(&key, action.boxed_clone(), context.clone(), false, None, cx)
                .expect("a key parsed once parses again")
                .with_meta(WINDOW_BINDING)
        })
        .collect::<Vec<_>>();
    let kept = cx
        .key_bindings()
        .borrow()
        .bindings()
        .filter(|binding| binding.meta() != Some(WINDOW_BINDING))
        .cloned()
        .collect::<Vec<_>>();
    cx.clear_key_bindings();
    cx.bind_keys(kept.into_iter().chain(window_bindings));
    // A menu item keeps the key it was made with, and macOS still answers that key.
    super::startup::set_app_menus(cx);
}

impl Condr {
    pub(in crate::app) fn set_option_as_meta(
        &mut self,
        option: OptionAsMeta,
        cx: &mut Context<Self>,
    ) {
        if self.option_as_meta == option {
            return;
        }
        self.option_as_meta = option;
        self.save_option_as_meta(cx);
        cx.notify();
    }

    pub(super) fn apply_keymap(&mut self, cx: &mut Context<Self>) {
        self.keymap = Keymap::resolve(&self.keybindings);
        install(&self.keymap, cx);
    }

    /// Gives shortcut `index` the key `key` as well, taking it from whichever shortcut
    /// has it.
    pub(super) fn add_shortcut_key(
        &mut self,
        index: usize,
        key: Keystroke,
        cx: &mut Context<Self>,
    ) {
        let mut changes = Vec::new();
        if let Some(owner) = self.keymap.keys.iter().position(|keys| keys.contains(&key)) {
            if owner == index {
                return;
            }
            changes.push((owner, without(&self.keymap.keys[owner], &key)));
        }
        let mut keys = self.keymap.keys[index].clone();
        keys.push(key);
        changes.push((index, keys));
        self.override_shortcuts(changes, cx);
    }

    pub(super) fn remove_shortcut_key(
        &mut self,
        index: usize,
        key: &Keystroke,
        cx: &mut Context<Self>,
    ) {
        let keys = without(&self.keymap.keys[index], key);
        self.override_shortcuts(vec![(index, keys)], cx);
    }

    /// Back to the default keys, less any another shortcut's entry has taken.
    pub(super) fn reset_shortcut(&mut self, index: usize, cx: &mut Context<Self>) {
        let name = all().nth(index).expect("a shortcut").name;
        self.keybindings.remove(name);
        self.apply_keymap(cx);
        self.save_keybindings(vec![(name, None)], cx);
    }

    /// Writes each shortcut's new keys as its entry, or drops the entry when they are the
    /// defaults, so the file only says what differs.
    fn override_shortcuts(
        &mut self,
        changes: Vec<(usize, Vec<Keystroke>)>,
        cx: &mut Context<Self>,
    ) {
        let mut entries = Vec::new();
        for (index, keys) in changes {
            let shortcut = all().nth(index).expect("a shortcut");
            let keys = (!keys.iter().cloned().eq(default_keys(shortcut)))
                .then(|| keys.iter().map(written).collect::<Vec<_>>());
            match &keys {
                Some(keys) => {
                    let list = keys.iter().cloned().map(toml::Value::String).collect();
                    self.keybindings
                        .insert(shortcut.name.to_owned(), toml::Value::Array(list));
                }
                None => {
                    self.keybindings.remove(shortcut.name);
                }
            }
            entries.push((shortcut.name, keys));
        }
        self.apply_keymap(cx);
        self.save_keybindings(entries, cx);
    }
}

fn without(keys: &[Keystroke], key: &Keystroke) -> Vec<Keystroke> {
    keys.iter().filter(|kept| *kept != key).cloned().collect()
}

#[cfg(test)]
mod tests {
    // Not `use super::*`: it would glob in `gpui_kit::test` and shadow the built-in attribute.
    use super::{Keymap, Keystroke, MAC, QuitApp, SplitRight, all, parse, refusal, written};

    fn table(source: &str) -> toml::Table {
        source.parse().expect("valid TOML")
    }

    fn keys_of(keymap: &Keymap, name: &str) -> Vec<String> {
        let index = all().position(|shortcut| shortcut.name == name).unwrap();
        keymap.keys[index].iter().map(written).collect()
    }

    fn errors_of(keymap: &Keymap, name: &str) -> Vec<String> {
        let index = all().position(|shortcut| shortcut.name == name).unwrap();
        keymap.errors[index].clone()
    }

    /// The modifier this platform's defaults lean on, as `[client.keybindings]` writes it.
    const PRIMARY: &str = if MAC { "cmd" } else { "ctrl" };

    #[test]
    fn every_default_is_a_distinct_shortcut_key_and_names_are_unique() {
        let mut seen: Vec<Keystroke> = Vec::new();
        for shortcut in all() {
            assert!(
                !shortcut.defaults.is_empty(),
                "{} has no default",
                shortcut.name
            );
            for source in shortcut.defaults {
                let key = parse(source).unwrap_or_else(|| panic!("{source} must parse"));
                assert_eq!(refusal(&key), None, "{source}");
                assert!(!seen.contains(&key), "{source} is a default twice");
                seen.push(key);
            }
            assert_eq!(all().filter(|other| other.name == shortcut.name).count(), 1);
        }
        assert!(Keymap::default().errors.iter().all(Vec::is_empty));
    }

    #[test]
    fn an_entry_replaces_its_defaults_and_an_empty_one_removes_them() {
        let keymap = Keymap::resolve(&table(&format!(
            "split-right = [\"{PRIMARY}-k\", \"{PRIMARY}-shift-k\"]\ntoggle-zoom = []"
        )));
        assert_eq!(
            keys_of(&keymap, "split-right"),
            [format!("{PRIMARY}-k"), format!("{PRIMARY}-shift-k")]
        );
        assert!(keys_of(&keymap, "toggle-zoom").is_empty());
        assert_eq!(
            keys_of(&keymap, "split-down"),
            keys_of(&Keymap::default(), "split-down")
        );
        let action = keymap
            .action_for(&crate::app::tests::keystroke(&format!("{PRIMARY}-k")))
            .expect("the new key is the shortcut's");
        assert!(action.as_any().is::<SplitRight>());
        let old = &Keymap::default().keys[all().position(|s| s.name == "split-right").unwrap()][0];
        assert!(
            keymap.action_for(old).is_none(),
            "the replaced default is gone"
        );
    }

    #[test]
    fn an_entry_takes_its_keys_from_the_defaults_and_from_later_entries() {
        let new_tab = keys_of(&Keymap::default(), "new-tab").remove(0);
        let keymap = Keymap::resolve(&table(&format!(
            "toggle-zoom = [\"{new_tab}\"]\nsplit-right = [\"{new_tab}\", \"{PRIMARY}-k\"]"
        )));
        // split-right comes first in `all()`, so it keeps the key and toggle-zoom is told.
        assert_eq!(
            keys_of(&keymap, "split-right"),
            [new_tab.clone(), format!("{PRIMARY}-k")]
        );
        assert!(keys_of(&keymap, "toggle-zoom").is_empty());
        assert_eq!(
            errors_of(&keymap, "toggle-zoom"),
            [format!("{new_tab} is already Split right.")]
        );
        assert!(
            keys_of(&keymap, "new-tab").is_empty(),
            "the default lost its key"
        );
        assert!(errors_of(&keymap, "new-tab").is_empty(), "silently");
    }

    #[test]
    fn malformed_entries_are_left_out_one_by_one() {
        let keymap = Keymap::resolve(&table(&format!(
            "split-right = [\"{PRIMARY}-k\", \"nonsense-key-\", 3, \"k\"]\n\
             split-down = \"{PRIMARY}-j\"\n\
             no-such-action = [\"{PRIMARY}-l\"]"
        )));
        assert_eq!(keys_of(&keymap, "split-right"), [format!("{PRIMARY}-k")]);
        let modifier_error = if MAC {
            "k needs Cmd, Ctrl or Option."
        } else {
            "k needs Ctrl or Alt."
        };
        assert_eq!(
            errors_of(&keymap, "split-right"),
            [
                "nonsense-key- is not a key.",
                "3 is not a key.",
                modifier_error
            ]
        );
        assert_eq!(
            keys_of(&keymap, "split-down"),
            keys_of(&Keymap::default(), "split-down"),
            "a value that is not a list keeps the defaults"
        );
        assert_eq!(errors_of(&keymap, "split-down").len(), 1);
        assert_eq!(keymap.unknown, ["no-such-action"]);
        // An F-key needs no modifier.
        let keymap = Keymap::resolve(&table("split-right = [\"f6\"]"));
        assert_eq!(keys_of(&keymap, "split-right"), ["f6"]);
    }

    #[test]
    fn the_macos_system_chords_cannot_be_taken() {
        let keymap = Keymap::resolve(&table("split-right = [\"cmd-q\"]"));
        if MAC {
            assert!(keys_of(&keymap, "split-right").is_empty());
            assert_eq!(
                errors_of(&keymap, "split-right"),
                ["cmd-q belongs to macOS."]
            );
            let quit = keymap
                .action_for(&crate::app::tests::keystroke("cmd-q"))
                .unwrap();
            assert!(quit.as_any().is::<QuitApp>());
        } else {
            // Win/Super+Q is an ordinary key there.
            assert_eq!(keys_of(&keymap, "split-right").len(), 1);
        }
    }
}
