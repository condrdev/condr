//! Syntax highlighting for the read-only Editors, the Preview and Diff Tabs (ADR 0032):
//! the Sublime syntaxes `bat` curates, parsed by syntect on Oniguruma behind GPUI Kit's
//! `InputHighlighter` seam, and coloured by the theme Settings picks.

use std::{
    cell::{Cell, RefCell},
    collections::HashMap,
    ops::Range,
    rc::Rc,
    sync::{
        Arc, LazyLock, OnceLock,
        atomic::{AtomicBool, Ordering},
    },
};

use gpui_kit::component::input::{
    EditorState, FoldRange, HighlightStyleResolver, InputEdit, InputHighlighter,
    InputHighlighterFactory, Rope,
};
use gpui_kit::component::theme::Theme as GuiTheme;
use gpui_kit::{
    App, Context, FontWeight, Global, HighlightStyle, Hsla, Rgba, SharedString, Task,
    UnderlineStyle, Window, px,
};
use syntect::{
    easy::ScopeRangeIterator,
    highlighting::{Color, FontStyle, Highlighter, Style, Theme, ThemeSet},
    parsing::{ParseState, Scope, ScopeStack, SyntaxReference, SyntaxSet},
};
use two_face::theme::{EmbeddedLazyThemeSet, EmbeddedThemeName};

macro_rules! vendored {
    ($file:literal) => {
        Source::Vendored(include_str!(concat!(
            "../../assets/code_themes/",
            $file,
            ".tmTheme"
        )))
    };
}

static SYNTAXES: LazyLock<SyntaxSet> = LazyLock::new(two_face::syntax::extra_newlines);
static BUNDLED_THEMES: LazyLock<EmbeddedLazyThemeSet> = LazyLock::new(two_face::theme::extra);

/// Longer lines are left plain, as Kit's own painting does past the same length: a
/// minified bundle would otherwise keep a background thread busy for seconds.
const MAX_LINE_LENGTH: usize = 10_000;

/// Where a theme Settings offers comes from.
enum Source {
    Bundled(EmbeddedThemeName),
    /// A tmTheme `script/generate-code-themes.mjs` vendored.
    Vendored(&'static str),
}

/// The themes Settings offers, by the name it shows and stores, in alphabetical order: a
/// light and a dark variant of each family that has both.
const CODE_THEMES: &[(&str, Source)] = &[
    ("Dracula", Source::Bundled(EmbeddedThemeName::Dracula)),
    ("GitHub Dark", vendored!("github-dark")),
    ("GitHub Light", vendored!("github-light")),
    (
        "Gruvbox Dark",
        Source::Bundled(EmbeddedThemeName::GruvboxDark),
    ),
    (
        "Gruvbox Light",
        Source::Bundled(EmbeddedThemeName::GruvboxLight),
    ),
    ("JetBrains Darcula", vendored!("jetbrains-darcula")),
    ("JetBrains Light", vendored!("jetbrains-light")),
    ("Nord", Source::Bundled(EmbeddedThemeName::Nord)),
    (
        "One Half Dark",
        Source::Bundled(EmbeddedThemeName::OneHalfDark),
    ),
    (
        "One Half Light",
        Source::Bundled(EmbeddedThemeName::OneHalfLight),
    ),
    (
        "Solarized Dark",
        Source::Bundled(EmbeddedThemeName::SolarizedDark),
    ),
    (
        "Solarized Light",
        Source::Bundled(EmbeddedThemeName::SolarizedLight),
    ),
    ("Tokyo Night", vendored!("tokyo-night")),
    ("Tokyo Night Day", vendored!("tokyo-night-day")),
];

/// Each theme, loaded the first time it paints.
static LOADED: [OnceLock<Theme>; CODE_THEMES.len()] =
    [const { OnceLock::new() }; CODE_THEMES.len()];

/// What an empty theme name, Settings' "Default", paints with in a light and a dark GUI.
const DEFAULT_LIGHT_THEME: &str = "GitHub Light";
const DEFAULT_DARK_THEME: &str = "GitHub Dark";

pub(crate) fn theme_names() -> impl Iterator<Item = &'static str> {
    CODE_THEMES.iter().map(|(name, _)| *name)
}

/// The named theme when Settings offers it, else the default for the GUI's mode.
fn theme(name: &str, dark: bool) -> &'static Theme {
    let default = if dark {
        DEFAULT_DARK_THEME
    } else {
        DEFAULT_LIGHT_THEME
    };
    let position = |wanted: &str| CODE_THEMES.iter().position(|(name, _)| *name == wanted);
    let index = position(name)
        .or_else(|| position(default))
        .expect("the default themes are listed");
    LOADED[index].get_or_init(|| match CODE_THEMES[index].1 {
        Source::Bundled(name) => BUNDLED_THEMES.get(name).clone(),
        Source::Vendored(plist) => ThemeSet::load_from_reader(&mut std::io::Cursor::new(plist))
            .expect("a vendored theme parses"),
    })
}

/// The theme Settings chose, empty for Default, and the one the highlighters paint with
/// now. Every highlighter holds `painting`, so a new choice recolours each open Editor
/// on its next paint without parsing it again.
pub(crate) struct CodeTheme {
    pub(crate) name: SharedString,
    painting: Rc<Cell<&'static Theme>>,
}

impl Global for CodeTheme {}

/// Records the chosen theme and applies it.
pub(crate) fn set_code_theme(name: SharedString, cx: &mut App) {
    let painting = painting(cx);
    cx.set_global(CodeTheme { name, painting });
    apply_code_theme(cx);
}

fn painting(cx: &App) -> Rc<Cell<&'static Theme>> {
    cx.try_global::<CodeTheme>().map_or_else(
        || Rc::new(Cell::new(theme("", false))),
        |chosen| chosen.painting.clone(),
    )
}

/// Paints with the chosen theme and gives Kit's code editors its background and
/// current-line colours, so the gutter matches the text. Changing the GUI mode resets
/// Kit's highlight theme, and Default follows the mode, so this runs again after every
/// mode change.
pub(crate) fn apply_code_theme(cx: &mut App) {
    let Some(code) = cx.try_global::<CodeTheme>() else {
        return;
    };
    let chosen = theme(&code.name, GuiTheme::global(cx).is_dark());
    code.painting.set(chosen);

    let gui = GuiTheme::global_mut(cx);
    let mut highlight = (*gui.highlight_theme).clone();
    let background = chosen.settings.background.map(hsla);
    highlight.style.editor_background = background;
    highlight.style.editor_gutter_background = background;
    highlight.style.editor_foreground = chosen.settings.foreground.map(text_color);
    highlight.style.editor_active_line = chosen.settings.line_highlight.map(hsla);
    gui.highlight_theme = Arc::new(highlight);
    cx.refresh_windows();
}

/// Builds a highlighter for the language names the Editors use: a file extension
/// (`rs`), a file name the syntax lists (`Makefile`) or a syntax name (`diff`). A name
/// without a syntax still gets one, to paint the text in the theme's foreground.
pub(crate) fn highlighter_factory(cx: &App) -> InputHighlighterFactory {
    let painting = painting(cx);
    Rc::new(move |language| {
        Some(Box::new(SyntaxHighlighter::new(
            language,
            syntax_for(language),
            painting.clone(),
        )) as Box<dyn InputHighlighter>)
    })
}

fn syntax_for(language: &str) -> Option<&'static SyntaxReference> {
    let syntaxes = &*SYNTAXES;
    syntaxes
        .find_syntax_by_token(language)
        .filter(|syntax| syntax.name != syntaxes.find_syntax_plain_text().name)
}

/// What a parse found, and the styles one theme gives it.
#[derive(Default)]
struct Parsed {
    /// Ordered, non-overlapping byte ranges, each with the index of its scope stack.
    spans: Vec<(Range<usize>, u32)>,
    stacks: Vec<Vec<Scope>>,
    styles: Option<StackStyles>,
}

/// The style of each stack under one theme, resolved the first time painting needs it.
struct StackStyles {
    theme: &'static Theme,
    highlighter: Highlighter<'static>,
    by_stack: Vec<Option<HighlightStyle>>,
}

impl StackStyles {
    fn new(theme: &'static Theme, stacks: usize) -> Self {
        Self {
            theme,
            highlighter: Highlighter::new(theme),
            by_stack: vec![None; stacks],
        }
    }

    fn get(&mut self, stack: u32, stacks: &[Vec<Scope>]) -> HighlightStyle {
        let Self {
            theme,
            highlighter,
            by_stack,
        } = self;
        *by_stack[stack as usize].get_or_insert_with(|| {
            highlight_style(
                highlighter.style_for_stack(&stacks[stack as usize]),
                theme.settings.background,
            )
        })
    }
}

struct SyntaxHighlighter {
    language: SharedString,
    syntax: Option<&'static SyntaxReference>,
    painting: Rc<Cell<&'static Theme>>,
    /// The text `parsed` describes, as Kit last handed it over.
    text: Rope,
    parsed: Rc<RefCell<Parsed>>,
    parse: Option<Task<()>>,
}

impl SyntaxHighlighter {
    fn new(
        language: &str,
        syntax: Option<&'static SyntaxReference>,
        painting: Rc<Cell<&'static Theme>>,
    ) -> Self {
        Self {
            language: SharedString::from(language.to_owned()),
            syntax,
            painting,
            text: Rope::new(),
            parsed: Rc::default(),
            parse: None,
        }
    }
}

impl InputHighlighter for SyntaxHighlighter {
    fn language(&self) -> SharedString {
        self.language.clone()
    }

    fn update(
        &mut self,
        _: Option<InputEdit>,
        text: &Rope,
        _: bool,
        window: &mut Window,
        cx: &mut Context<EditorState>,
    ) {
        let Some(syntax) = self.syntax else {
            return;
        };
        // Every refresh of a read-only Editor reaches here as a whole-document edit, so
        // the part that changed comes from comparing the texts. The old colours stay on
        // the unchanged start and end while the new parse runs, so an agent editing the
        // shown file neither blanks nor shifts them.
        keep_unchanged(&mut self.parsed.borrow_mut().spans, &self.text, text);
        self.text = text.clone();

        // ponytail: every update parses the whole text again (a second or so for a 1 MiB
        // file); cache a ParseState every few hundred lines and resume from the one
        // before the change if agents editing large files make that visible.
        let parsed = self.parsed.clone();
        let text = text.clone();
        let cancel = Arc::new(AtomicBool::new(false));
        self.parse = Some(cx.spawn_in(window, async move |editor, cx| {
            let _cancel = CancelOnDrop(cancel.clone());
            let result = cx
                .background_executor()
                .spawn(async move { parse(&text.to_string(), syntax, &cancel) })
                .await;
            if let Some(result) = result {
                *parsed.borrow_mut() = result;
                editor.update(cx, |_, cx| cx.notify()).ok();
            }
        }));
    }

    fn styles(
        &self,
        range: &Range<usize>,
        _: &dyn HighlightStyleResolver,
    ) -> Vec<(Range<usize>, HighlightStyle)> {
        let theme = self.painting.get();
        // Kit draws unstyled text in the GUI's foreground; the theme's own keeps it
        // readable on the theme's background.
        let plain = HighlightStyle {
            color: theme.settings.foreground.map(text_color),
            ..HighlightStyle::default()
        };
        let mut parsed = self.parsed.borrow_mut();
        let Parsed {
            spans,
            stacks,
            styles,
        } = &mut *parsed;
        let styles = match styles {
            Some(styles) if std::ptr::eq(styles.theme, theme) => styles,
            _ => styles.insert(StackStyles::new(theme, stacks.len())),
        };

        let first = spans.partition_point(|(span, _)| span.end <= range.start);
        let mut runs = Vec::new();
        let mut at = range.start;
        for (span, stack) in spans[first..]
            .iter()
            .take_while(|(span, _)| span.start < range.end)
        {
            let start = span.start.max(at);
            let end = span.end.min(range.end);
            if at < start {
                runs.push((at..start, plain));
            }
            runs.push((start..end, styles.get(*stack, stacks)));
            at = end;
        }
        if at < range.end {
            runs.push((at..range.end, plain));
        }
        runs
    }

    fn fold_ranges(&self, _: &Rope) -> Vec<FoldRange> {
        Vec::new()
    }
}

/// Stops the background parse once the task awaiting it is dropped for a newer one.
struct CancelOnDrop(Arc<AtomicBool>);

impl Drop for CancelOnDrop {
    fn drop(&mut self) {
        self.0.store(true, Ordering::Relaxed);
    }
}

/// Parses `text` from the start, or gives up when `cancel` is set. A line the syntax
/// fails on stays plain.
fn parse(text: &str, syntax: &SyntaxReference, cancel: &AtomicBool) -> Option<Parsed> {
    let syntaxes = &*SYNTAXES;
    let mut state = ParseState::new(syntax);
    let mut stack = ScopeStack::new();
    let mut parsed = Parsed::default();
    let mut ids: HashMap<Vec<Scope>, u32> = HashMap::new();
    let mut offset = 0;
    for line in text.split_inclusive('\n') {
        if cancel.load(Ordering::Relaxed) {
            return None;
        }
        let start = offset;
        offset += line.len();
        if line.len() > MAX_LINE_LENGTH {
            continue;
        }
        let Ok(ops) = state.parse_line(line, syntaxes) else {
            continue;
        };
        for (range, op) in ScopeRangeIterator::new(&ops, line) {
            if stack.apply(op).is_err() {
                break;
            }
            if range.is_empty() {
                continue;
            }
            let scopes = stack.as_slice();
            let id = match ids.get(scopes) {
                Some(&id) => id,
                None => {
                    let id = parsed.stacks.len() as u32;
                    parsed.stacks.push(scopes.to_vec());
                    ids.insert(scopes.to_vec(), id);
                    id
                }
            };
            let range = start + range.start..start + range.end;
            match parsed.spans.last_mut() {
                Some((last, last_id)) if last.end == range.start && *last_id == id => {
                    last.end = range.end;
                }
                _ => parsed.spans.push((range, id)),
            }
        }
    }
    Some(parsed)
}

fn highlight_style(style: Style, background: Option<Color>) -> HighlightStyle {
    let font = style.font_style;
    HighlightStyle {
        color: Some(text_color(style.foreground)),
        // A theme resolves every scope to some background; only one that differs from
        // the editor's is a highlight of its own, such as a diff's added line.
        background_color: (Some(style.background) != background).then(|| hsla(style.background)),
        font_weight: font.contains(FontStyle::BOLD).then_some(FontWeight::BOLD),
        font_style: font
            .contains(FontStyle::ITALIC)
            .then_some(gpui_kit::FontStyle::Italic),
        underline: font.contains(FontStyle::UNDERLINE).then(|| UnderlineStyle {
            thickness: px(1.),
            ..UnderlineStyle::default()
        }),
        ..HighlightStyle::default()
    }
}

fn hsla(color: Color) -> Hsla {
    Rgba {
        r: f32::from(color.r) / 255.,
        g: f32::from(color.g) / 255.,
        b: f32::from(color.b) / 255.,
        a: f32::from(color.a) / 255.,
    }
    .into()
}

/// Text ignores a theme's alpha, as bat does: gruvbox gives its foreground half of it.
fn text_color(color: Color) -> Hsla {
    hsla(Color { a: 255, ..color })
}

/// Keeps the spans on the start and end `old` and `new` share, the end ones moved to
/// where that text now sits, and drops the rest.
fn keep_unchanged<T>(spans: &mut Vec<(Range<usize>, T)>, old: &Rope, new: &Rope) {
    let prefix = old
        .bytes()
        .zip(new.bytes())
        .take_while(|(a, b)| a == b)
        .count();
    let longest_suffix = old.len().min(new.len()) - prefix;
    let suffix = old
        .bytes_at(old.len())
        .reversed()
        .zip(new.bytes_at(new.len()).reversed())
        .take(longest_suffix)
        .take_while(|(a, b)| a == b)
        .count();
    let old_end = old.len() - suffix;
    let new_end = new.len() - suffix;
    spans.retain_mut(|(range, _)| {
        if range.end <= prefix {
            true
        } else if range.start >= old_end {
            *range = range.start - old_end + new_end..range.end - old_end + new_end;
            true
        } else {
            false
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    struct NoTheme;

    impl HighlightStyleResolver for NoTheme {
        fn style(&self, _: &str) -> Option<HighlightStyle> {
            None
        }
    }

    /// A highlighter over `text` as parsed, painting with the named theme.
    fn highlighted(language: &str, text: &str, theme_name: &str) -> SyntaxHighlighter {
        let syntax = syntax_for(language);
        let highlighter = SyntaxHighlighter::new(
            language,
            syntax,
            Rc::new(Cell::new(theme(theme_name, false))),
        );
        if let Some(syntax) = syntax {
            *highlighter.parsed.borrow_mut() =
                parse(text, syntax, &AtomicBool::new(false)).expect("not cancelled");
        }
        highlighter
    }

    /// The style painted at the start of `needle`.
    fn style_at(highlighter: &SyntaxHighlighter, text: &str, needle: &str) -> HighlightStyle {
        let at = text.find(needle).expect("the needle is in the text");
        highlighter.styles(&(at..at + 1), &NoTheme)[0].1
    }

    #[test]
    fn languages_resolve_by_extension_file_name_or_syntax_name() {
        // Kotlin is here because Kit's tree-sitter Kotlin once shipped without ever
        // colouring a file.
        for language in [
            "rs", "ts", "tsx", "kt", "toml", "md", "yml", "Makefile", "diff",
        ] {
            assert!(syntax_for(language).is_some(), "{language}");
        }
        for language in ["text", "txt", "no-such-language"] {
            assert!(syntax_for(language).is_none(), "{language}");
        }
    }

    #[test]
    fn every_theme_loads_and_default_follows_the_mode() {
        let names: Vec<_> = theme_names().collect();
        assert!(
            names.is_sorted_by_key(|name| name.to_lowercase()),
            "Settings lists the themes by name"
        );
        assert!(names.contains(&DEFAULT_LIGHT_THEME) && names.contains(&DEFAULT_DARK_THEME));
        for name in names {
            // Loading parses a vendored tmTheme; each must come with its own colours.
            let settings = &theme(name, false).settings;
            assert!(
                settings.background.is_some() && settings.foreground.is_some(),
                "{name}"
            );
        }

        assert!(std::ptr::eq(
            theme("", false),
            theme(DEFAULT_LIGHT_THEME, false)
        ));
        assert!(std::ptr::eq(
            theme("", true),
            theme(DEFAULT_DARK_THEME, true)
        ));
        assert!(std::ptr::eq(
            theme("no such theme", true),
            theme(DEFAULT_DARK_THEME, true)
        ));
        assert!(
            std::ptr::eq(theme("Dracula", false), theme("Dracula", true)),
            "a named theme holds in either mode"
        );
    }

    #[test]
    fn the_theme_colours_each_scope_and_the_rest_in_its_foreground() {
        let text = "// note\nfn main() { let s = \"x\"; }\n";
        let highlighter = highlighted("rs", text, "GitHub Light");
        let comment = style_at(&highlighter, text, "// note");
        let keyword = style_at(&highlighter, text, "fn");
        let string = style_at(&highlighter, text, "\"x\"");
        assert!(comment.color != keyword.color && keyword.color != string.color);

        let foreground = theme("GitHub Light", false)
            .settings
            .foreground
            .map(text_color);
        let plain = highlighted("text", "just words\n", "GitHub Light");
        assert_eq!(style_at(&plain, "just words\n", "words").color, foreground);
    }

    #[test]
    fn a_new_theme_recolours_without_parsing_again() {
        let text = "fn main() {}\n";
        let highlighter = highlighted("rs", text, "GitHub Light");
        let before = style_at(&highlighter, text, "fn");
        highlighter.painting.set(theme("Solarized Light", false));
        let after = style_at(&highlighter, text, "fn");
        assert_ne!(before.color, after.color);
    }

    #[test]
    fn styles_cover_the_asked_range() {
        let text = "fn main() {}\n";
        let highlighter = highlighted("rs", text, "GitHub Light");
        let runs = highlighter.styles(&(1..text.len()), &NoTheme);
        assert_eq!(runs.first().unwrap().0.start, 1);
        assert_eq!(runs.last().unwrap().0.end, text.len());
        assert!(runs.windows(2).all(|pair| pair[0].0.end == pair[1].0.start));
    }

    #[test]
    fn a_cancelled_parse_gives_up() {
        let syntax = syntax_for("rs").unwrap();
        assert!(parse("fn main() {}\n", syntax, &AtomicBool::new(true)).is_none());
    }

    #[test]
    fn an_edit_keeps_the_spans_on_either_side() {
        let old = Rope::from("aaa\nbbb\nccc\n");
        let new = Rope::from("aaa\nXXXXX\nccc\n");
        let mut spans = vec![(0..3, "a"), (4..7, "b"), (8..11, "c")];
        keep_unchanged(&mut spans, &old, &new);
        assert_eq!(spans, vec![(0..3, "a"), (10..13, "c")]);

        let mut spans = vec![(0..3, "a"), (4..7, "b")];
        keep_unchanged(&mut spans, &old, &old);
        assert_eq!(
            spans,
            vec![(0..3, "a"), (4..7, "b")],
            "an unchanged text keeps all"
        );
    }
}
