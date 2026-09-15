use super::*;

#[test]
fn terminal_shortcut_fallback_maps_only_fixed_chords() {
    let action = |keys: &str| fixed_shortcut(&Keystroke::parse(keys).unwrap());

    assert!(action("ctrl-tab").unwrap().as_any().is::<NextTab>());
    assert!(
        action("ctrl-shift-tab")
            .unwrap()
            .as_any()
            .is::<PreviousTab>()
    );
    if cfg!(target_os = "macos") {
        // Cmd is the application modifier; Ctrl and Option belong to the shell.
        assert!(action("cmd-t").unwrap().as_any().is::<NewTab>());
        assert!(action("cmd-w").unwrap().as_any().is::<ClosePane>());
        assert!(action("cmd-d").unwrap().as_any().is::<SplitRight>());
        assert!(action("cmd-shift-d").unwrap().as_any().is::<SplitDown>());
        assert!(action("cmd-}").unwrap().as_any().is::<NextTab>());
        assert!(action("cmd-{").unwrap().as_any().is::<PreviousTab>());
        assert!(action("cmd-alt-left").unwrap().as_any().is::<FocusLeft>());
        assert!(action("cmd-ctrl-left").is_some());
        assert!(
            action("cmd-shift-enter")
                .unwrap()
                .as_any()
                .is::<ToggleZoom>()
        );
        assert!(action("alt-left").is_none(), "Option+Left moves by word");
        assert!(action("alt-shift-=").is_none(), "Option+Shift+= types ±");
        assert!(
            action("ctrl-shift-t").is_none(),
            "Ctrl chords reach the PTY"
        );
        assert!(action("cmd-shift-t").is_none());
        assert!(action("cmd-ctrl-alt-left").is_none());
    } else {
        assert!(action("alt-shift-=").unwrap().as_any().is::<SplitRight>());
        assert!(action("alt-shift--").unwrap().as_any().is::<SplitDown>());
        assert!(action("alt-+").unwrap().as_any().is::<SplitRight>());
        assert!(action("alt-_").unwrap().as_any().is::<SplitDown>());
        assert!(action("alt-left").unwrap().as_any().is::<FocusLeft>());
        assert!(
            action("alt-shift-enter")
                .unwrap()
                .as_any()
                .is::<ToggleZoom>()
        );
        assert!(action("cmd-t").is_none());
    }
    assert!(action("alt-enter").is_none());
    // Settings must open even while a terminal owns the keystroke, on the platform's
    // own chord: Cmd+, on macOS, Ctrl+, elsewhere.
    assert!(action("secondary-,").unwrap().as_any().is::<OpenSettings>());
    if cfg!(target_os = "macos") {
        assert!(action("ctrl-,").is_none());
    } else {
        assert!(action("cmd-,").is_none());
    }
    assert!(action("ctrl-p").is_none());
}

#[test]
fn numbered_tab_shortcuts_reserve_only_the_platforms_digit_chords() {
    let modifier = if cfg!(target_os = "macos") {
        "cmd"
    } else {
        "alt"
    };
    for number in 1..=9 {
        let action = fixed_shortcut(&Keystroke::parse(&format!("{modifier}-{number}")).unwrap())
            .expect("each numbered Tab has a shortcut");
        assert_eq!(
            action.as_any().downcast_ref::<ActivateTab>(),
            Some(&ActivateTab { index: number - 1 })
        );
    }
    for chord in [
        format!("{modifier}-0"),
        format!("{modifier}-shift-1"),
        "ctrl-1".into(),
        "ctrl-alt-1".into(),
        "1".into(),
        if cfg!(target_os = "macos") {
            "alt-1"
        } else {
            "cmd-1"
        }
        .into(),
    ] {
        assert!(
            fixed_shortcut(&Keystroke::parse(&chord).unwrap()).is_none(),
            "{chord}"
        );
    }
}

#[test]
fn terminal_clipboard_shortcuts_preserve_terminal_control_keys() {
    let shortcut = |keys: &str, has_selection| {
        terminal_clipboard_shortcut(&Keystroke::parse(keys).unwrap(), has_selection)
    };

    // Ctrl+Shift+C/V: clipboard on Linux/Windows; on macOS every Ctrl chord is PTY input.
    let control_shift_copy =
        (!cfg!(target_os = "macos")).then_some(TerminalClipboardShortcut::Copy);
    let control_shift_paste =
        (!cfg!(target_os = "macos")).then_some(TerminalClipboardShortcut::Paste);
    assert_eq!(shortcut("ctrl-shift-c", false), control_shift_copy);
    assert_eq!(shortcut("ctrl-shift-v", false), control_shift_paste);
    assert_eq!(
        shortcut("ctrl-insert", false),
        Some(TerminalClipboardShortcut::Copy)
    );
    assert_eq!(
        shortcut("shift-insert", false),
        Some(TerminalClipboardShortcut::Paste)
    );
    assert_eq!(shortcut("ctrl-c", false), None);
    #[cfg(windows)]
    assert_eq!(
        shortcut("ctrl-c", true),
        Some(TerminalClipboardShortcut::Copy)
    );
    #[cfg(not(windows))]
    assert_eq!(shortcut("ctrl-c", true), None);
    #[cfg(target_os = "macos")]
    assert_eq!(
        shortcut("cmd-c", true),
        Some(TerminalClipboardShortcut::Copy)
    );
    #[cfg(not(target_os = "macos"))]
    assert_eq!(shortcut("cmd-c", true), None);
    assert_eq!(shortcut("ctrl-alt-v", false), None);

    #[cfg(windows)]
    assert_eq!(
        shortcut("ctrl-v", false),
        Some(TerminalClipboardShortcut::Paste)
    );
    #[cfg(not(windows))]
    assert_eq!(shortcut("ctrl-v", false), None);
}

#[test]
fn terminal_keys_follow_platform_keystroke_semantics() {
    let key = |keys: &str| terminal_key_for(&Keystroke::parse(keys).unwrap());
    let character = |text: &str| Some(TerminalKey::Character(text.into()));

    // Plain text and shifted text arrive through the input handler, not as keys.
    assert_eq!(key("s"), None);
    assert_eq!(key("shift-s->S"), None);
    assert_eq!(key("ctrl-c"), character("c"));
    assert_eq!(key("ctrl-alt-c"), character("c"));
    // Ctrl/Alt+Space are NUL and ESC SP, not the letters of "space".
    assert_eq!(key("ctrl-space"), character(" "));
    assert_eq!(key("alt-space"), character(" "));
    // Windows leaves key_char empty under Alt: the shifted letter is still uppercase.
    assert_eq!(key("alt-shift-s"), character("S"));
    // Keys a terminal cannot encode are not turned into their names.
    assert_eq!(key("ctrl-pause"), None);
    assert_eq!(key("shift-tab"), Some(TerminalKey::BackTab));
    assert_eq!(key("ctrl-f13"), Some(TerminalKey::Function(13)));

    // macOS Option produces a character (ß, or S with Shift), which is typed rather than
    // sent as ESC ß; elsewhere Alt is Meta.
    let option_s = key("alt-s->ß");
    if cfg!(target_os = "macos") {
        assert_eq!(option_s, None);
        assert_eq!(key("alt-shift-s->S"), None);
        assert_eq!(key("ctrl-alt-s"), character("s"));
    } else {
        assert_eq!(option_s, character("ß"));
        assert_eq!(key("alt-shift-s->S"), character("S"));
        assert_eq!(key("alt-s->s"), character("s"));
    }
}

#[test]
fn printable_character_input_preference_bypasses_terminal_key_encoding() {
    let event = KeyDownEvent {
        keystroke: Keystroke::parse("ctrl-alt-q->@").unwrap(),
        is_held: false,
        prefer_character_input: true,
    };
    assert!(should_defer_to_character_input(&event));

    let mut ordinary_modified_key = event.clone();
    ordinary_modified_key.prefer_character_input = false;
    assert!(!should_defer_to_character_input(&ordinary_modified_key));

    let mut non_printable = event;
    non_printable.keystroke.key_char = Some("\t".into());
    assert!(!should_defer_to_character_input(&non_printable));
}

#[test]
fn hover_clears_only_from_the_pane_that_owns_it() {
    let owner = pane_id();
    let other = pane_id();
    let link = HoveredTerminalLink {
        range: 0..4,
        uri: "https://example.com".into(),
        position: condr_core::TerminalMousePosition { row: 0, column: 1 },
    };
    let current = (1, owner, link.clone());

    assert_eq!(
        next_hovered_link(None, 1, owner, Some(link.clone())),
        Some(Some(current.clone()))
    );
    assert_eq!(
        next_hovered_link(Some(&current), 1, owner, Some(link.clone())),
        None
    );
    // Another pane (or another connection) reporting no link leaves the owner's hover alone.
    assert_eq!(next_hovered_link(Some(&current), 1, other, None), None);
    assert_eq!(next_hovered_link(Some(&current), 2, owner, None), None);
    // The owner reporting no link clears it; a new link elsewhere replaces it.
    assert_eq!(
        next_hovered_link(Some(&current), 1, owner, None),
        Some(None)
    );
    assert_eq!(
        next_hovered_link(Some(&current), 1, other, Some(link.clone())),
        Some(Some((1, other, link)))
    );
}

#[test]
fn alt_v_alone_is_the_image_paste_gesture_and_maps_only_supported_formats() {
    assert!(is_image_paste_gesture(&Keystroke::parse("alt-v").unwrap()));
    let mut function_chord = Keystroke::parse("alt-v").unwrap();
    function_chord.modifiers.function = true;
    assert!(!is_image_paste_gesture(&function_chord));
    for keys in ["v", "ctrl-v", "alt-shift-v", "ctrl-alt-v", "cmd-v", "alt-c"] {
        assert!(
            !is_image_paste_gesture(&Keystroke::parse(keys).unwrap()),
            "{keys}"
        );
    }
    // Alt+V is not a text-paste shortcut either: on a local Server it reaches the PTY.
    assert!(terminal_clipboard_shortcut(&Keystroke::parse("alt-v").unwrap(), false).is_none());
    use condr_core::protocol::ClipboardImageFormat as Wire;
    assert_eq!(
        clipboard_image_format(gpui_kit::ImageFormat::Png),
        Some(Wire::Png)
    );
    assert_eq!(
        clipboard_image_format(gpui_kit::ImageFormat::Jpeg),
        Some(Wire::Jpeg)
    );
    assert_eq!(clipboard_image_format(gpui_kit::ImageFormat::Svg), None);
    assert_eq!(clipboard_image_format(gpui_kit::ImageFormat::Tiff), None);
}

#[test]
fn clipboard_bmp_conversion_preserves_pixels_and_rejects_bad_input() {
    use condr_core::protocol::{ClipboardImageFormat as Wire, MAX_CLIPBOARD_IMAGE_BYTES};
    use std::io::Cursor;

    let pixels = image::RgbaImage::from_raw(2, 1, vec![255, 0, 0, 255, 0, 0, 255, 96]).unwrap();
    let mut bytes = Vec::new();
    pixels
        .write_to(&mut Cursor::new(&mut bytes), image::ImageFormat::Bmp)
        .unwrap();
    let mut format = Wire::Bmp;
    prepare_clipboard_image(&mut format, &mut bytes).unwrap();
    assert_eq!(format, Wire::Png);
    assert_eq!(
        image::guess_format(&bytes).unwrap(),
        image::ImageFormat::Png
    );
    assert_eq!(image::load_from_memory(&bytes).unwrap().to_rgba8(), pixels);

    for mut format in [Wire::Png, Wire::Jpeg, Wire::Gif, Wire::Webp] {
        let original_format = format;
        let mut bytes = vec![1, 2, 3];
        prepare_clipboard_image(&mut format, &mut bytes).unwrap();
        assert_eq!(format, original_format);
        assert_eq!(bytes, [1, 2, 3]);
    }

    let mut format = Wire::Bmp;
    let mut bytes = vec![1, 2, 3];
    assert!(prepare_clipboard_image(&mut format, &mut bytes).is_err());
    assert_eq!(format, Wire::Bmp);
    assert_eq!(bytes, [1, 2, 3]);
    bytes.resize(MAX_CLIPBOARD_IMAGE_BYTES + 1, 0);
    assert!(
        prepare_clipboard_image(&mut format, &mut bytes)
            .unwrap_err()
            .contains("16 MiB")
    );
}
