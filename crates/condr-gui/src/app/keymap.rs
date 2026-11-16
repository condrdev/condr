//! The window shortcuts (ADR 0063): each action's keys on this platform, what
//! `[client.keybindings]` changes about them, and the keymap that results.

use super::command_palette::{self, OpenCommandPalette, Page};
use super::*;
use rust_i18n::t;

/// One window action, by the name `[client.keybindings]` knows it by.
pub(super) struct Shortcut {
    pub name: &'static str,
    /// The message key of its label, which `t!` turns into text.
    pub label: &'static str,
    pub action: &'static dyn Action,
    /// What it acts on, for the Command Palette to grey it out without (ADR 0064).
    pub needs: Needs,
    /// Its keys on this platform while nothing overrides them; tooltips show the first.
    /// Some actions have none until someone gives them one.
    pub defaults: &'static [&'static str],
}

/// What a window action needs to do anything. Its handler checks for itself; this only
/// lets the Command Palette say so beforehand (ADR 0064).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Needs {
    Nothing,
    /// A Pane the window's actions apply to, on a connection that can change the Session.
    Pane,
    /// A Tab the window presents.
    Tab,
    /// A presented Tab on a connection that can change the Session.
    Change,
    /// A presented connection that can change the Session.
    Device,
    /// A Workspace on any Device.
    Workspaces,
    /// A presented Workspace whose right sidebar this window draws.
    Changes,
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
    needs: Needs,
    defaults: &'static [&'static str],
) -> Shortcut {
    Shortcut {
        name,
        label,
        action,
        needs,
        defaults,
    }
}

/// Every window action, grouped as the Shortcuts page and the Command Palette show them,
/// each group and label by its message key. The order is also the precedence when two entries of `[client.keybindings]` name the
/// same key.
pub(super) const GROUPS: &[(&str, &[Shortcut])] = &[
    (
        "keymap.group.application",
        &[
            shortcut(
                "open-settings",
                "keymap.action.open_settings",
                &OpenSettings,
                Needs::Nothing,
                on(&["cmd-,"], &["ctrl-,"]),
            ),
            // VS Code's primary and secondary sidebar chords; elsewhere Ctrl+B is the tmux
            // prefix and readline's backward-char, so Shift joins it.
            shortcut(
                "toggle-sidebar",
                "keymap.action.toggle_sidebar",
                &ToggleSidebar,
                Needs::Nothing,
                on(&["cmd-b"], &["ctrl-shift-b"]),
            ),
            shortcut(
                "toggle-changes",
                "keymap.action.toggle_changes",
                &ToggleChanges,
                Needs::Changes,
                on(&["cmd-alt-b"], &["ctrl-alt-b"]),
            ),
            // VS Code's Command Palette and Go to File; Ctrl+P stays the terminal's, and
            // herdr-app takes Ctrl+Shift+K for its Go to.
            shortcut(
                "command-palette",
                "keymap.action.command_palette",
                &OpenCommandPalette {
                    page: Page::Actions,
                },
                Needs::Nothing,
                on(&["cmd-shift-p"], &["ctrl-shift-p"]),
            ),
            shortcut(
                "go-to",
                "keymap.action.go_to",
                &OpenCommandPalette { page: Page::GoTo },
                Needs::Nothing,
                on(&["cmd-p"], &["ctrl-shift-k"]),
            ),
        ],
    ),
    (
        "keymap.group.devices",
        &[
            shortcut(
                "connect-remote-device",
                "keymap.action.connect_remote_device",
                &AddServer,
                Needs::Nothing,
                &[],
            ),
            shortcut(
                "reconnect-device",
                "keymap.action.reconnect_device",
                &ReconnectServer,
                Needs::Nothing,
                &[],
            ),
        ],
    ),
    (
        "keymap.group.workspaces",
        &[
            // Workspaces stack vertically in the sidebar, so Up and Down walk them.
            shortcut(
                "next-workspace",
                "keymap.action.next_workspace",
                &NextWorkspace,
                Needs::Workspaces,
                on(&["cmd-shift-down"], &["ctrl-shift-down"]),
            ),
            shortcut(
                "previous-workspace",
                "keymap.action.previous_workspace",
                &PreviousWorkspace,
                Needs::Workspaces,
                on(&["cmd-shift-up"], &["ctrl-shift-up"]),
            ),
            shortcut(
                "new-workspace",
                "keymap.action.new_workspace",
                &NewWorkspace,
                Needs::Device,
                &[],
            ),
            shortcut(
                "rename-workspace",
                "keymap.action.rename_workspace",
                &RenameWorkspace,
                Needs::Change,
                &[],
            ),
            shortcut(
                "close-workspace",
                "keymap.action.close_workspace",
                &CloseWorkspace,
                Needs::Change,
                &[],
            ),
        ],
    ),
    (
        "keymap.group.tabs",
        &[
            shortcut(
                "new-tab",
                "keymap.action.new_tab",
                &NewTab,
                Needs::Change,
                on(&["cmd-t"], &["ctrl-shift-t"]),
            ),
            shortcut(
                "next-tab",
                "keymap.action.next_tab",
                &NextTab,
                Needs::Tab,
                on(&["cmd-}", "ctrl-tab"], &["ctrl-tab"]),
            ),
            shortcut(
                "previous-tab",
                "keymap.action.previous_tab",
                &PreviousTab,
                Needs::Tab,
                on(&["cmd-{", "ctrl-shift-tab"], &["ctrl-shift-tab"]),
            ),
            shortcut(
                "rename-tab",
                "keymap.action.rename_tab",
                &RenameTab,
                Needs::Change,
                &[],
            ),
            shortcut(
                "close-tab",
                "keymap.action.close_tab",
                &CloseTab,
                Needs::Change,
                &[],
            ),
            shortcut(
                "go-to-tab-1",
                "keymap.action.go_to_tab_1",
                &ActivateTab { index: 0 },
                Needs::Tab,
                on(&["cmd-1"], &["alt-1"]),
            ),
            shortcut(
                "go-to-tab-2",
                "keymap.action.go_to_tab_2",
                &ActivateTab { index: 1 },
                Needs::Tab,
                on(&["cmd-2"], &["alt-2"]),
            ),
            shortcut(
                "go-to-tab-3",
                "keymap.action.go_to_tab_3",
                &ActivateTab { index: 2 },
                Needs::Tab,
                on(&["cmd-3"], &["alt-3"]),
            ),
            shortcut(
                "go-to-tab-4",
                "keymap.action.go_to_tab_4",
                &ActivateTab { index: 3 },
                Needs::Tab,
                on(&["cmd-4"], &["alt-4"]),
            ),
            shortcut(
                "go-to-tab-5",
                "keymap.action.go_to_tab_5",
                &ActivateTab { index: 4 },
                Needs::Tab,
                on(&["cmd-5"], &["alt-5"]),
            ),
            shortcut(
                "go-to-tab-6",
                "keymap.action.go_to_tab_6",
                &ActivateTab { index: 5 },
                Needs::Tab,
                on(&["cmd-6"], &["alt-6"]),
            ),
            shortcut(
                "go-to-tab-7",
                "keymap.action.go_to_tab_7",
                &ActivateTab { index: 6 },
                Needs::Tab,
                on(&["cmd-7"], &["alt-7"]),
            ),
            shortcut(
                "go-to-tab-8",
                "keymap.action.go_to_tab_8",
                &ActivateTab { index: 7 },
                Needs::Tab,
                on(&["cmd-8"], &["alt-8"]),
            ),
            shortcut(
                "go-to-tab-9",
                "keymap.action.go_to_tab_9",
                &ActivateTab { index: 8 },
                Needs::Tab,
                on(&["cmd-9"], &["alt-9"]),
            ),
        ],
    ),
    (
        "keymap.group.panes",
        &[
            shortcut(
                "split-right",
                "keymap.action.split_right",
                &SplitRight,
                Needs::Pane,
                on(&["cmd-d"], &["alt-shift-="]),
            ),
            shortcut(
                "split-down",
                "keymap.action.split_down",
                &SplitDown,
                Needs::Pane,
                on(&["cmd-shift-d"], &["alt-shift--"]),
            ),
            shortcut(
                "close-pane",
                "keymap.action.close_pane",
                &ClosePane,
                Needs::Pane,
                on(&["cmd-w"], &["ctrl-shift-w"]),
            ),
            shortcut(
                "toggle-zoom",
                "keymap.action.toggle_zoom",
                &ToggleZoom,
                Needs::Pane,
                on(&["cmd-shift-enter"], &["alt-shift-enter"]),
            ),
            shortcut(
                "focus-left",
                "keymap.action.focus_left",
                &FocusLeft,
                Needs::Pane,
                on(&["cmd-alt-left"], &["alt-left"]),
            ),
            shortcut(
                "focus-right",
                "keymap.action.focus_right",
                &FocusRight,
                Needs::Pane,
                on(&["cmd-alt-right"], &["alt-right"]),
            ),
            shortcut(
                "focus-up",
                "keymap.action.focus_up",
                &FocusUp,
                Needs::Pane,
                on(&["cmd-alt-up"], &["alt-up"]),
            ),
            shortcut(
                "focus-down",
                "keymap.action.focus_down",
                &FocusDown,
                Needs::Pane,
                on(&["cmd-alt-down"], &["alt-down"]),
            ),
            shortcut(
                "resize-left",
                "keymap.action.resize_left",
                &ResizeLeft,
                Needs::Pane,
                on(&["cmd-ctrl-left"], &["alt-shift-left"]),
            ),
            shortcut(
                "resize-right",
                "keymap.action.resize_right",
                &ResizeRight,
                Needs::Pane,
                on(&["cmd-ctrl-right"], &["alt-shift-right"]),
            ),
            shortcut(
                "resize-up",
                "keymap.action.resize_up",
                &ResizeUp,
                Needs::Pane,
                on(&["cmd-ctrl-up"], &["alt-shift-up"]),
            ),
            shortcut(
                "resize-down",
                "keymap.action.resize_down",
                &ResizeDown,
                Needs::Pane,
                on(&["cmd-ctrl-down"], &["alt-shift-down"]),
            ),
            shortcut(
                "swap-left",
                "keymap.action.swap_left",
                &SwapLeft,
                Needs::Pane,
                &[],
            ),
            shortcut(
                "swap-right",
                "keymap.action.swap_right",
                &SwapRight,
                Needs::Pane,
                &[],
            ),
            shortcut(
                "swap-up",
                "keymap.action.swap_up",
                &SwapUp,
                Needs::Pane,
                &[],
            ),
            shortcut(
                "swap-down",
                "keymap.action.swap_down",
                &SwapDown,
                Needs::Pane,
                &[],
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
/// would take a key the terminal needs for typing (F-keys aside). The reason is the key
/// of a message that names the key as `key`.
pub(super) fn refusal(key: &Keystroke) -> Option<&'static str> {
    let modifiers = key.modifiers;
    let function_key = key
        .key
        .strip_prefix('f')
        .and_then(|number| number.parse::<u8>().ok())
        .is_some_and(|number| (1..=24).contains(&number));
    if !(modifiers.control || modifiers.alt || modifiers.platform || function_key) {
        return Some(if MAC {
            "keymap.error.needs_modifier_mac"
        } else {
            "keymap.error.needs_modifier"
        });
    }
    SYSTEM
        .iter()
        .any(|(system, _)| parse(system).as_ref() == Some(key))
        .then_some("keymap.error.belongs_to_macos")
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
                errors[index].push(t!("keymap.error.not_a_list", value = value).into_owned());
                continue;
            };
            let mut keys = Vec::new();
            for item in list {
                let Some(source) = item.as_str() else {
                    errors[index].push(t!("keymap.error.not_a_key", key = item).into_owned());
                    continue;
                };
                let Some(key) = parse(source) else {
                    errors[index].push(t!("keymap.error.not_a_key", key = source).into_owned());
                    continue;
                };
                if let Some(why) = refusal(&key) {
                    errors[index].push(t!(why, key = source).into_owned());
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
                    errors[index].push(
                        t!(
                            "keymap.error.taken",
                            key = written(key),
                            action = t!(shortcuts[*owner].label)
                        )
                        .into_owned(),
                    );
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
    let context = |name| {
        Some(Rc::new(
            KeyBindingContextPredicate::parse(name).expect("a valid key context"),
        ))
    };
    let (window, palette) = (context(SHORTCUT_CONTEXT), context(command_palette::CONTEXT));
    // The last binding added for an action is the one tooltips and the menu show.
    let window_bindings = all()
        .zip(&keymap.keys)
        .flat_map(|(shortcut, keys)| keys.iter().rev().map(|key| (written(key), shortcut.action)))
        .chain(
            SYSTEM
                .iter()
                .map(|(key, action)| ((*key).to_owned(), *action)),
        )
        // The open Command Palette sits outside the window's context, and its own keys
        // close it there (ADR 0064).
        .flat_map(|(key, action)| {
            let in_palette = action.as_any().is::<OpenCommandPalette>();
            [Some(window.clone()), in_palette.then(|| palette.clone())]
                .into_iter()
                .flatten()
                .map(move |context| (key.clone(), action, context))
        })
        .map(|(key, action, context)| {
            KeyBinding::load(&key, action.boxed_clone(), context, false, None, cx)
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

    /// Makes `key` shortcut `index`'s only key, taking it from whichever shortcut has it.
    pub(super) fn set_shortcut_key(
        &mut self,
        index: usize,
        key: Keystroke,
        cx: &mut Context<Self>,
    ) {
        let mut changes = Vec::new();
        if let Some(owner) = self.keymap.keys.iter().position(|keys| keys.contains(&key))
            && owner != index
        {
            changes.push((owner, without(&self.keymap.keys[owner], &key)));
        }
        changes.push((index, vec![key]));
        self.override_shortcuts(changes, cx);
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
