//! The GUI's own messages live in `locales/*.yml`, one file per area, each key with its
//! `en` and `zh-CN` text side by side; `rust_i18n::t!` looks one up under the current
//! locale (ADR 0066). Kit's components translate their own strings under the same locale.

/// The languages the GUI speaks, as `[client] language` names them.
pub(crate) const LANGUAGES: [&str; 2] = ["en", "zh-CN"];

/// The language to show: `configured` when the GUI speaks it, otherwise the first of the
/// system's preferred languages it speaks, otherwise English.
pub(crate) fn resolve(
    configured: Option<&str>,
    preferred: impl IntoIterator<Item = String>,
) -> &'static str {
    configured
        .and_then(|configured| {
            LANGUAGES
                .into_iter()
                .find(|language| *language == configured)
        })
        .or_else(|| preferred.into_iter().find_map(|tag| spoken(&tag)))
        .unwrap_or("en")
}

/// Which of `LANGUAGES` a system locale (`zh-Hans-CN`, `zh_CN.UTF-8`) asks for. Traditional
/// Chinese asks for none: it reads Simplified no better than it reads English.
fn spoken(tag: &str) -> Option<&'static str> {
    let tag = tag
        .split(['.', '@'])
        .next()?
        .to_ascii_lowercase()
        .replace('_', "-");
    let mut parts = tag.split('-');
    match parts.next()? {
        "en" => Some("en"),
        "zh" => {
            let parts = parts.collect::<Vec<_>>();
            let traditional = !parts.contains(&"hans")
                && parts
                    .iter()
                    .any(|part| matches!(*part, "hant" | "tw" | "hk" | "mo"));
            (!traditional).then_some("zh-CN")
        }
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeSet;
    use std::path::Path;

    fn messages(locale: &str) -> Vec<(String, String)> {
        crate::_rust_i18n_backend()
            .messages_for_locale(locale)
            .unwrap_or_default()
            .into_iter()
            .map(|(key, text)| (key.into_owned(), text.into_owned()))
            .collect()
    }

    fn keys(locale: &str) -> BTreeSet<String> {
        messages(locale).into_iter().map(|(key, _)| key).collect()
    }

    fn placeholders(text: &str) -> BTreeSet<&str> {
        text.split("%{")
            .skip(1)
            .filter_map(|rest| rest.split_once('}').map(|(name, _)| name))
            .collect()
    }

    /// Every `.rs` file under `src`, full-line comments left out so a doc comment that
    /// shows `t!` is not read as a call.
    fn source() -> String {
        fn walk(directory: &Path, out: &mut String) {
            let mut entries = std::fs::read_dir(directory)
                .expect("src is readable")
                .map(|entry| entry.expect("readable entry").path())
                .collect::<Vec<_>>();
            entries.sort();
            for path in entries {
                if path.is_dir() {
                    walk(&path, out);
                } else if path.extension().is_some_and(|extension| extension == "rs") {
                    let text = std::fs::read_to_string(&path).expect("source is UTF-8");
                    for line in text.lines() {
                        if !line.trim_start().starts_with("//") {
                            out.push_str(line);
                            out.push('\n');
                        }
                    }
                }
            }
        }
        let mut out = String::new();
        walk(&Path::new(env!("CARGO_MANIFEST_DIR")).join("src"), &mut out);
        out
    }

    /// The literal key of every `t!("…")` in `source`, wherever rustfmt broke the call.
    fn called_keys(source: &str) -> BTreeSet<&str> {
        source
            .match_indices("t!(")
            .filter(|(at, _)| {
                !source[..*at]
                    .chars()
                    .next_back()
                    .is_some_and(|before| before.is_alphanumeric() || before == '_')
            })
            .filter_map(|(at, call)| {
                let rest = source[at + call.len()..].trim_start().strip_prefix('"')?;
                rest.split_once('"').map(|(key, _)| key)
            })
            // Keys are `area.snake_case.parts`; anything else is not a call, as this
            // file's own `"t!("` is not.
            .filter(|key| {
                !key.is_empty()
                    && key.chars().all(|c| {
                        c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_' || c == '.'
                    })
            })
            .collect()
    }

    #[test]
    fn resolve_prefers_the_setting_then_the_system() {
        let system = |tags: &[&str]| tags.iter().map(|tag| tag.to_string()).collect::<Vec<_>>();
        assert_eq!(super::resolve(Some("en"), system(&["zh-Hans-CN"])), "en");
        assert_eq!(super::resolve(Some("zh-CN"), system(&["en-US"])), "zh-CN");
        // An unknown setting is no setting.
        assert_eq!(super::resolve(Some("fr"), system(&["zh-Hans-CN"])), "zh-CN");
        assert_eq!(super::resolve(None, system(&["zh_CN.UTF-8"])), "zh-CN");
        assert_eq!(super::resolve(None, system(&["zh-SG"])), "zh-CN");
        assert_eq!(super::resolve(None, system(&["zh-Hans-HK"])), "zh-CN");
        assert_eq!(super::resolve(None, system(&["zh-Hant-TW", "en-US"])), "en");
        assert_eq!(super::resolve(None, system(&["zh-HK"])), "en");
        assert_eq!(
            super::resolve(None, system(&["ja-JP", "zh-Hans", "en"])),
            "zh-CN"
        );
        assert_eq!(super::resolve(None, system(&["de-DE"])), "en");
        assert_eq!(super::resolve(None, system(&[])), "en");
    }

    #[test]
    fn every_message_has_english_and_chinese() {
        let locales = crate::_rust_i18n_backend().available_locales();
        assert_eq!(
            locales,
            super::LANGUAGES,
            "locales/*.yml holds exactly these languages"
        );
        let english = keys("en");
        let chinese = keys("zh-CN");
        let no_chinese = english.difference(&chinese).collect::<Vec<_>>();
        let no_english = chinese.difference(&english).collect::<Vec<_>>();
        assert!(no_chinese.is_empty(), "no zh-CN text: {no_chinese:#?}");
        assert!(no_english.is_empty(), "no en text: {no_english:#?}");
    }

    #[test]
    fn translations_keep_every_placeholder() {
        let chinese = messages("zh-CN")
            .into_iter()
            .collect::<std::collections::HashMap<_, _>>();
        let mismatched = messages("en")
            .into_iter()
            .filter(|(key, english)| {
                chinese
                    .get(key)
                    .is_some_and(|text| placeholders(text) != placeholders(english))
            })
            .map(|(key, _)| key)
            .collect::<Vec<_>>();
        assert!(
            mismatched.is_empty(),
            "placeholders differ: {mismatched:#?}"
        );
    }

    #[test]
    fn source_and_messages_name_the_same_keys() {
        let source = source();
        let english = keys("en");
        let unknown = called_keys(&source)
            .into_iter()
            .filter(|key| !english.contains(*key))
            .collect::<Vec<_>>();
        assert!(
            unknown.is_empty(),
            "t! names a key no locale file has: {unknown:#?}"
        );
        // A key taken from a table rather than a `t!` literal still appears quoted.
        let unused = english
            .iter()
            .filter(|key| !source.contains(&format!("\"{key}\"")))
            .collect::<Vec<_>>();
        assert!(unused.is_empty(), "no code names these keys: {unused:#?}");
    }

    #[test]
    fn called_keys_reads_split_calls_and_skips_other_macros() {
        let source = "t!(\"a.b\")\nassert!(\"x\")\nlet x = t!(\n    \"c.d\",\n    n = 1\n);";
        assert_eq!(called_keys(source), BTreeSet::from(["a.b", "c.d"]));
    }
}
