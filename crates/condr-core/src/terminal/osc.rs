//! The working directory a program reports: OSC 7 as a `file://` URI or kitty's
//! `kitty-shell-cwd://`, ConEmu's `9;9` and iTerm2's `1337;CurrentDir` as a path.
//! libghostty-vt parses the sequences and hands over what the program sent (ADR 0051);
//! this turns it into a directory on this machine.

use super::*;
use crate::uri::percent_decode;

/// The directory a program reported, if it names one here.
pub(super) fn reported_cwd_of(pwd: &str) -> Option<PathBuf> {
    let uri = pwd.trim_start();
    if let Some(rest) = uri.strip_prefix("file://") {
        return local_uri_cwd(rest.trim_end(), true);
    }
    // Condr's shell integration sends kitty's form, as Ghostty's does: the path unencoded.
    if let Some(rest) = uri.strip_prefix("kitty-shell-cwd://") {
        return local_uri_cwd(rest, false);
    }
    let path = pwd
        .strip_prefix('"')
        .and_then(|path| path.strip_suffix('"'))
        .unwrap_or(pwd);
    (!path.is_empty() && !path.contains('\0')).then(|| PathBuf::from(path))
}

/// `[host]/path` after the scheme, percent-encoded or as it is; only this machine's paths
/// are usable.
fn local_uri_cwd(rest: &str, encoded: bool) -> Option<PathBuf> {
    let decode = |text: &str| {
        if encoded {
            percent_decode(text)
        } else {
            Some(text.to_owned())
        }
    };
    let path = match rest.find('/') {
        Some(0) => rest,
        Some(slash) => {
            let host = decode(&rest[..slash])?;
            let host = host.trim_end_matches('.');
            if !host.eq_ignore_ascii_case("localhost")
                && !sysinfo::System::host_name().is_some_and(|local| {
                    let local = local.trim_end_matches('.');
                    host.eq_ignore_ascii_case(local)
                        || local
                            .split_once('.')
                            .is_some_and(|(short, _)| host.eq_ignore_ascii_case(short))
                })
            {
                return None;
            }
            &rest[slash..]
        }
        None => return None,
    };
    let path = decode(path)?;
    #[cfg(windows)]
    let path = {
        let bytes = path.as_bytes();
        let drive = bytes.len() >= 3 && bytes[1].is_ascii_alphabetic() && bytes[2] == b':';
        if drive { &path[1..] } else { path.as_str() }.replace('/', "\\")
    };
    (!path.is_empty() && !path.contains('\0')).then(|| PathBuf::from(path))
}
