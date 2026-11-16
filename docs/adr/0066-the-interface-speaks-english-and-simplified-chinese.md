# The Interface Speaks English and Simplified Chinese

Status: accepted 2026-11-16.

Every word the GUI showed was an English literal in its code, while half of condr.dev's documentation is already in Simplified Chinese. GPUI Kit translates its own components through rust-i18n and lets the application share that locale, so the GUI does the same: its own text goes through rust-i18n, and one locale switches both.

## What is translated

The GUI, in English (`en`) and Simplified Chinese (`zh-CN`). Not the `condr` CLI, not the Companion, which has its platforms' own string catalogs, and not the website, which Starlight translates.

What the GUI shows but does not write stays as it came: a Server's, a Client connection's or the operating system's error text, an Agent's `detail` (including the `Permission`, `Question` or `Login` core fills in), a Refusal's reason, log records, and product and editor names. The GUI's own sentence around such text is translated, the text inside it is not: `无法保存 {path}：{error}`. Keeping it English also keeps the GUI's matching on it working, such as the advice `connection_status.rs` picks by a connect error's wording. Turning every Server message into a code the Client translates is a larger change than this one.

Condr's own concepts keep the English names the Chinese documentation already uses, so what a page says is what the window shows: Workspace, Tab, Pane, Session, Agent, Worktree, Diff Tab, Preview Tab, Needs you and the Agent states. Everything else is translated. `CONTEXT.md` lists both.

## How

`crates/condr-gui/locales/<area>.yml` holds the messages, one file per area of the GUI, in rust-i18n's `_version: 2` form: each key holds its `en` and `zh-CN` text side by side, so a review sees a missing or stale translation next to its English. Keys are semantic, `connection.title.connecting`, never the English text: an edited English sentence must not silently orphan its translation. `rust_i18n::i18n!("locales", fallback = "en")` in `main.rs` compiles them in, and `build.rs` rebuilds the GUI when one changes.

`rust_i18n::t!` looks a message up under the current locale and fills in `%{name}` arguments. A sentence is one message with its values inside it, never fragments joined by code, so a translation can reorder it. A message about a count has `.one` and `.other` forms, chosen by `count == 1` at the call; Chinese gives both the same text.

The GUI does not override Kit's own translations or call `extend!`; `set_locale` switches Kit's components and the GUI's messages together.

Formatting is the GUI's: the Server's `relative_age` and `uptime_text` stay English for the CLI, and the GUI words the same buckets itself. Byte sizes and shortened numbers are not localized.

The Command Palette and Settings search match a label in the current language and in English, so a known English name still finds it.

No interface font is configured for Chinese: macOS and Windows fall back to their own CJK fonts, and Linux needs one installed.

## Choosing the language

`[client] language` in `config.toml`, `en` or `zh-CN`, chosen under Settings > Appearance; absent means the system's, the first preferred UI language that is Simplified Chinese (`zh-Hans`, `zh-CN`, `zh-SG`) or else English. Traditional Chinese falls back to English. A change applies at once: `set_locale`, a refresh of every window, and the macOS menus made again as a key binding change does (ADR 0063).

## Writing Chinese

Address the reader as 你, as the documentation does. Put a space between Chinese and Latin letters or digits (`打开 Pane`, `3 个 Tab`). Use full-width punctuation and keep `…`. A button or menu item is a verb phrase without a full stop; a sentence ends with 。. A failure reads `无法…：{error}`.

## Checks

`i18n.rs` tests that every key has both languages and no other, that a translation keeps its English text's placeholders, that every `t!("…")` names a key that exists and that every key is named somewhere in the source. rust-i18n shows a missing key's name instead of failing, so these tests are what catch one. The locale is process-wide, so tests never call `set_locale`: they run in English, and one that needs a Chinese message asks for it with `t!(key, locale = "zh-CN")`. A debug selector or test never finds an element by its text.
