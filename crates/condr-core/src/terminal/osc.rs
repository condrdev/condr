//! The working directory a program reports: OSC 7 as a `file://` URI, ConEmu's `9;9` and
//! iTerm2's `1337;CurrentDir` as a path. libghostty-vt parses the sequences and hands over
//! what the program sent (ADR 0051); this turns it into a directory on this machine.

use super::*;
use crate::uri::percent_decode;

/// The directory a program reported, if it names one here.
pub(super) fn reported_cwd_of(pwd: &str) -> Option<PathBuf> {
    if pwd.trim_start().starts_with("file://") {
        return file_uri_cwd(pwd);
    }
    let path = pwd
        .strip_prefix('"')
        .and_then(|path| path.strip_suffix('"'))
        .unwrap_or(pwd);
    (!path.is_empty() && !path.contains('\0')).then(|| PathBuf::from(path))
}

/// OSC 7 carries `file://[host]/path`; only this machine's paths are usable.
pub(super) fn file_uri_cwd(uri: &str) -> Option<PathBuf> {
    let rest = uri.trim().strip_prefix("file://")?;
    let path = match rest.find('/') {
        Some(0) => rest,
        Some(slash) => {
            let host = percent_decode(&rest[..slash])?;
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
    let path = percent_decode(path)?;
    #[cfg(windows)]
    let path = {
        let bytes = path.as_bytes();
        let drive = bytes.len() >= 3 && bytes[1].is_ascii_alphabetic() && bytes[2] == b':';
        if drive { &path[1..] } else { path.as_str() }.replace('/', "\\")
    };
    (!path.is_empty() && !path.contains('\0')).then(|| PathBuf::from(path))
}
