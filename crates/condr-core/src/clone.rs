//! The rules a New Directory and a Clone Repository follow before the Server makes the
//! directory (ADR 0059), shared by the GUI's forms, the CLI and the Server.

/// The longest clone URL the Server types into a Pane.
pub const MAX_CLONE_URL_BYTES: usize = 2048;

/// The URL with surrounding whitespace trimmed, or why it is refused. It is typed into a
/// shell, so quote characters are refused outright, U+2018 to U+201F among them, which
/// PowerShell reads as quotes. A local path must be absolute, since git would resolve a
/// relative one against the shell's directory.
pub fn validate_clone_url(url: &str) -> Result<&str, String> {
    let url = url.trim();
    if url.is_empty() {
        return Err("Enter a repository URL".into());
    }
    if url.chars().any(char::is_control) {
        return Err("The URL cannot contain control characters".into());
    }
    if url
        .chars()
        .any(|ch| matches!(ch, '\'' | '"' | '`' | '\u{2018}'..='\u{201F}'))
    {
        return Err("The URL cannot contain quotes".into());
    }
    if url.len() > MAX_CLONE_URL_BYTES {
        return Err(format!(
            "The URL is longer than {MAX_CLONE_URL_BYTES} bytes"
        ));
    }
    if is_local_path(url) && !is_absolute_path(url) {
        return Err("A local repository needs an absolute path".into());
    }
    Ok(url)
}

/// git's own test: a URL names a scheme, an scp-like `host:path` has no `/` before its
/// first `:`, and anything else, a drive letter included, is a local path.
fn is_local_path(url: &str) -> bool {
    if url.contains("://") {
        return false;
    }
    match url.split_once(':') {
        Some((host, _)) => host.contains('/') || has_drive_prefix(url),
        None => true,
    }
}

fn has_drive_prefix(path: &str) -> bool {
    let bytes = path.as_bytes();
    bytes.len() >= 2 && bytes[0].is_ascii_alphabetic() && bytes[1] == b':'
}

fn is_absolute_path(path: &str) -> bool {
    path.starts_with('/')
        || path.starts_with('\\')
        || (has_drive_prefix(path) && matches!(path.as_bytes().get(2), Some(b'/' | b'\\')))
}

/// Why `name` cannot be the one directory the Server makes under the parent. The rules
/// are Windows' as well everywhere, so a form can check them without knowing the
/// Server's platform.
pub fn validate_directory_name(name: &str) -> Result<(), String> {
    if name.is_empty() {
        return Err("Enter a name".into());
    }
    if name == "." || name == ".." {
        return Err("The name cannot be . or ..".into());
    }
    if name.chars().any(|ch| {
        ch.is_control() || matches!(ch, '/' | '\\' | '<' | '>' | ':' | '"' | '|' | '?' | '*')
    }) {
        return Err("The name cannot contain / \\ < > : \" | ? * or control characters".into());
    }
    if name.ends_with(['.', ' ']) {
        return Err("The name cannot end in a dot or a space".into());
    }
    let stem = name.split('.').next().unwrap_or(name).trim_end();
    let reserved = ["CON", "PRN", "AUX", "NUL"]
        .iter()
        .any(|device| stem.eq_ignore_ascii_case(device))
        || ["COM", "LPT"].iter().any(|device| {
            stem.len() == 4
                && stem.is_ascii()
                && stem[..3].eq_ignore_ascii_case(device)
                && matches!(stem.as_bytes()[3], b'1'..=b'9')
        });
    if reserved {
        return Err(format!("{name} is a reserved name on Windows"));
    }
    Ok(())
}

/// The directory `git clone <url>` would make, derived as git's `git_url_basename` does:
/// the scheme and any `user@` go, then trailing separators, `/.git`, a lone host's port,
/// and `.git` or `.bundle`; what follows the last `/`, `\` or `:` is the name.
pub fn clone_directory_name(url: &str) -> Option<String> {
    let mut rest = url.trim();
    if let Some((_, after)) = rest.split_once("://") {
        rest = after;
    }
    let host_end = rest.find(['/', '\\']).unwrap_or(rest.len());
    if let Some(at) = rest[..host_end].rfind('@') {
        rest = &rest[at + 1..];
    }
    let separator = |ch: char| ch == '/' || ch == '\\';
    rest = rest.trim_end_matches(|ch: char| separator(ch) || ch.is_whitespace());
    if let Some(stripped) = rest.strip_suffix(".git")
        && stripped.len() > 1
        && stripped.ends_with(separator)
    {
        rest = stripped.trim_end_matches(separator);
    }
    if !rest.contains('/')
        && let Some((host, port)) = rest.rsplit_once(':')
        && port.bytes().all(|byte| byte.is_ascii_digit())
    {
        rest = host;
    }
    let start = rest
        .rfind(|ch: char| separator(ch) || ch == ':')
        .map_or(0, |at| at + 1);
    let name = &rest[start..];
    let name = name
        .strip_suffix(".git")
        .or_else(|| name.strip_suffix(".bundle"))
        .unwrap_or(name);
    (!name.is_empty() && validate_directory_name(name).is_ok()).then(|| name.to_owned())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_directory_name_is_the_one_git_would_make() {
        for (url, name) in [
            ("https://github.com/condrdev/condr.git", Some("condr")),
            ("https://github.com/condrdev/condr", Some("condr")),
            ("https://github.com/condrdev/condr/", Some("condr")),
            ("https://github.com/condrdev/condr/.git", Some("condr")),
            ("https://user@host:8080/team/repo.git", Some("repo")),
            ("https://host:8080", Some("host")),
            ("ssh://git@host:22/team/repo.git", Some("repo")),
            ("git@github.com:condrdev/condr.git", Some("condr")),
            ("host:repo.git", Some("repo")),
            ("/srv/git/project.git", Some("project")),
            ("/srv/git/backup.bundle", Some("backup")),
            (r"C:\repos\tool.git", Some("tool")),
            ("/home/me/John Smith/repo", Some("repo")),
            ("https://host/", Some("host")),
            ("/", None),
            ("", None),
        ] {
            assert_eq!(clone_directory_name(url).as_deref(), name, "{url}");
        }
    }

    #[test]
    fn a_url_is_trimmed_and_refused_when_a_shell_could_misread_it() {
        assert_eq!(
            validate_clone_url("  https://host/repo.git\n"),
            Ok("https://host/repo.git")
        );
        assert_eq!(
            validate_clone_url("/home/me/John Smith/repo"),
            Ok("/home/me/John Smith/repo")
        );
        assert_eq!(validate_clone_url(r"C:\repos\tool"), Ok(r"C:\repos\tool"));
        assert_eq!(
            validate_clone_url("git@github.com:o/r.git"),
            Ok("git@github.com:o/r.git")
        );
        for refused in [
            "",
            "  ",
            "https://host/a\u{7}b",
            "https://host/x';id;'",
            "https://host/x\";id",
            "https://host/`id`",
            "https://host/x\u{2019};calc;\u{2018}",
            "https://host/x\u{201C}",
            "../repo",
            "repo",
            "~/repo",
            "C:repo",
        ] {
            assert!(validate_clone_url(refused).is_err(), "{refused:?}");
        }
        let long = format!("https://host/{}", "a".repeat(MAX_CLONE_URL_BYTES));
        assert!(validate_clone_url(&long).is_err());
    }

    #[test]
    fn a_directory_name_is_one_plain_component_on_every_platform() {
        for name in ["repo", "my repo", ".config", "a.b"] {
            assert_eq!(validate_directory_name(name), Ok(()), "{name}");
        }
        for name in [
            "", ".", "..", "a/b", r"a\b", "C:foo", "a:b", "a*b", "a?", "end.", "end ", "CON",
            "nul.txt", "com1", "LPT9", "a\u{1}",
        ] {
            assert!(validate_directory_name(name).is_err(), "{name:?}");
        }
        assert_eq!(validate_directory_name("COM0"), Ok(()));
        assert_eq!(validate_directory_name("a\u{20AC}"), Ok(()));
        assert_eq!(validate_directory_name("console"), Ok(()));
    }
}
