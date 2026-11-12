use super::*;
use crate::app::keymap::{self, Shortcut};

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

/// The window shortcuts, each row editing its keys (ADR 0063); on macOS the Option key
/// first; and the Terminal's own keys, which the page only shows.
pub(super) fn shortcuts_page(
    owner: &WeakEntity<Condr>,
    settings: &Entity<SettingsWindow>,
    cx: &App,
) -> SettingPage {
    let mut page = SettingPage::new("Shortcuts").icon(IconName::LayoutDashboard);
    if cfg!(target_os = "macos") {
        page = page.group(option_group(owner));
    }
    let unknown = owner
        .upgrade()
        .map(|owner| owner.read(cx).keymap.unknown.clone())
        .unwrap_or_default();
    if !unknown.is_empty() {
        page = page.group(
            SettingGroup::new().item(SettingItem::render(move |_, _, cx| {
                div()
                    .debug_selector(|| "shortcuts-unknown".into())
                    .text_sm()
                    .text_color(cx.theme().danger)
                    .child(format!(
                        "[client.keybindings] names no shortcut: {}.",
                        unknown.join(", ")
                    ))
            })),
        );
    }
    let mut index = 0;
    for (title, shortcuts) in keymap::GROUPS {
        let mut group = SettingGroup::new().title(*title);
        for shortcut in *shortcuts {
            group = group.item(shortcut_row(index, shortcut, owner, settings));
            index += 1;
        }
        if *title == "Application" {
            // No key `[client.keybindings]` can write, so it only shows (ADR 0064).
            group = group.item(fixed_row("Command Palette", &["shift", "shift"]));
        }
        page = page.group(group);
    }
    page.group(terminal_group())
}

fn option_group(owner: &WeakEntity<Condr>) -> SettingGroup {
    let (get_owner, set_owner) = (owner.clone(), owner.clone());
    let options = OptionAsMeta::ALL
        .map(|option| (option.as_str().into(), option.label().into()))
        .to_vec();
    SettingGroup::new().title("Keyboard").item(
        SettingItem::new(
            "Option as Meta",
            SettingField::dropdown(
                options,
                move |cx| {
                    get_owner
                        .upgrade()
                        .map(|owner| owner.read(cx).option_as_meta)
                        .unwrap_or_default()
                        .as_str()
                        .into()
                },
                move |value: SharedString, cx| {
                    let option = OptionAsMeta::from_str(&value);
                    let _ = set_owner.update(cx, |owner, cx| owner.set_option_as_meta(option, cx));
                },
            )
            .default_value(OptionAsMeta::default().as_str()),
        )
        .description(
            "Sends Esc and the key, as a shell expects of Meta, instead of typing a character such as ∫.",
        ),
    )
}

/// One shortcut's keys, each removable, and a button that records one more; reset
/// drops its `[client.keybindings]` entry.
fn shortcut_row(
    index: usize,
    shortcut: &'static Shortcut,
    owner: &WeakEntity<Condr>,
    settings: &Entity<SettingsWindow>,
) -> SettingItem {
    let (field_owner, dirty_owner, reset_owner) = (owner.clone(), owner.clone(), owner.clone());
    let settings = settings.clone();
    SettingItem::new(
        shortcut.label,
        SettingField::render(move |_, _, cx| {
            shortcut_keys(index, shortcut.name, &field_owner, &settings, cx)
        })
        .on_reset(
            move |cx| {
                dirty_owner
                    .upgrade()
                    .is_some_and(|owner| owner.read(cx).keybindings.contains_key(shortcut.name))
            },
            move |_, cx| {
                let _ = reset_owner.update(cx, |owner, cx| owner.reset_shortcut(index, cx));
            },
        ),
    )
}

fn shortcut_keys(
    index: usize,
    name: &'static str,
    owner: &WeakEntity<Condr>,
    settings: &Entity<SettingsWindow>,
    cx: &App,
) -> impl IntoElement + use<> {
    let (keys, mut errors) = owner
        .upgrade()
        .map(|owner| {
            let keymap = &owner.read(cx).keymap;
            (keymap.keys[index].clone(), keymap.errors[index].clone())
        })
        .unwrap_or_default();
    let window = settings.read(cx);
    let recording = window.recording == Some(index);
    if let Some((row, why)) = &window.refusal
        && *row == index
    {
        errors.push(why.to_string());
    }
    let chips = keys.into_iter().enumerate().map(|(n, key)| {
        let owner = owner.clone();
        let removed = key.clone();
        h_flex().gap_0p5().child(Kbd::new(key).outline()).child(
            Button::new(SharedString::from(format!("shortcut-{name}-remove-{n}")))
                .debug_selector(move || format!("shortcut-{name}-remove-{n}"))
                .icon(IconName::Close)
                .ghost()
                .xsmall()
                .tooltip("Remove")
                .on_click(move |_, _, cx| {
                    let _ = owner.update(cx, |owner, cx| {
                        owner.remove_shortcut_key(index, &removed, cx)
                    });
                }),
        )
    });
    let settings = settings.clone();
    let record = if recording {
        Button::new(SharedString::from(format!("shortcut-{name}-recording")))
            .debug_selector(move || format!("shortcut-{name}-recording"))
            .label("Press a key…")
            .outline()
            .xsmall()
            .tooltip("Esc cancels")
            .on_click(move |_, _, cx| {
                settings.update(cx, |this, cx| {
                    this.recording = None;
                    this.refusal = None;
                    cx.notify();
                });
            })
    } else {
        Button::new(SharedString::from(format!("shortcut-{name}-add")))
            .debug_selector(move || format!("shortcut-{name}-add"))
            .icon(IconName::Plus)
            .ghost()
            .xsmall()
            .tooltip("Add a key")
            .on_click(move |_, _, cx| {
                settings.update(cx, |this, cx| {
                    this.recording = Some(index);
                    this.refusal = None;
                    cx.notify();
                });
            })
    };
    let danger = cx.theme().danger;
    v_flex()
        .items_end()
        .gap_1()
        .child(
            h_flex()
                .gap_1()
                .flex_wrap()
                .justify_end()
                .children(chips)
                .child(record),
        )
        .children(
            errors
                .into_iter()
                .map(move |error| div().text_xs().text_color(danger).child(error)),
        )
}

/// Takes the next key pressed in the Settings window for the row that records, before
/// any binding or key listener can act on it: Escape there cancels, not closes.
pub(super) fn key_recorder(window: &Window, cx: &mut Context<SettingsWindow>) -> Subscription {
    let this = cx.weak_entity();
    let handle = window.window_handle();
    cx.intercept_keystrokes(move |event, window, cx| {
        if window.window_handle() != handle {
            return;
        }
        let taken = this
            .update(cx, |this, cx| this.record(&event.keystroke, cx))
            .unwrap_or(false);
        if taken {
            cx.stop_propagation();
        }
    })
}

impl SettingsWindow {
    /// Whether a row was recording, and so took the key.
    pub(super) fn record(&mut self, stroke: &Keystroke, cx: &mut Context<Self>) -> bool {
        let Some(index) = self.recording else {
            return false;
        };
        if matches!(
            stroke.key.as_str(),
            "shift" | "control" | "alt" | "platform" | "function"
        ) {
            // A modifier pressed and let go alone arrives as a key of its own; keep waiting.
            return true;
        }
        if stroke.key == "escape" && !stroke.modifiers.modified() {
            self.recording = None;
            self.refusal = None;
        } else {
            let key = keymap::normalize(stroke);
            if let Some(why) = keymap::refusal(&key) {
                let refusal = format!("{} {why}.", keymap::written(&key));
                self.refusal = Some((index, refusal.into()));
            } else {
                self.recording = None;
                self.refusal = None;
                let _ = self
                    .owner
                    .update(cx, |owner, cx| owner.add_shortcut_key(index, key, cx));
            }
        }
        cx.notify();
        true
    }
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
        |group, (label, keys)| group.item(fixed_row(label, keys)),
    )
}

/// A row whose keys stay as they are, which the page only shows.
fn fixed_row(label: &'static str, keys: &'static [&'static str]) -> SettingItem {
    SettingItem::new(
        label,
        SettingField::render(move |_, _, _| {
            h_flex().gap_1().children(
                keys.iter().map(|key| {
                    Kbd::new(Keystroke::parse(key).expect("a valid keystroke")).outline()
                }),
            )
        }),
    )
}
