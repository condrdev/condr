//! The Program Status Protocol, OSC 7501 (ADR 0044): a program tells its terminal what
//! it is doing, `OSC 7501 ; key=value : key=value … ST`. This module parses one report
//! or feature-detection query to the letter of revision 0.4 of the specification; what
//! Condr makes of it is the detector's business.
//!
//! <https://www.superlogical.com/rex/docs/build/program-status>

use base64::Engine as _;
use base64::engine::{DecodePaddingMode, GeneralPurpose, GeneralPurposeConfig};

/// The OSC payload prefix every report and query starts with.
pub const PROGRAM_STATUS_OSC_PREFIX: &str = "7501;";

/// What a supporting terminal answers a query with: a bare `?` promises exactly the five
/// base states and three base kinds.
pub const PROGRAM_STATUS_REPLY: &[u8] = b"\x1b]7501;?\x1b\\";

/// The limits the specification fixes; a report past any of them is dropped whole.
const MAX_SEQUENCE_BYTES: usize = 4096;
const MAX_KEY_BYTES: usize = 16;
const MAX_MSG_ENCODED_BYTES: usize = 2732;
const MAX_MSG_DECODED_BYTES: usize = 2048;
const MAX_TITLE_ENCODED_BYTES: usize = 256;
const MAX_TITLE_DECODED_BYTES: usize = 192;
const MAX_APP_BYTES: usize = 32;
const MAX_ID_BYTES: usize = 128;
const MAX_ID_SEGMENT_BYTES: usize = 32;
const MAX_ID_LEVELS: usize = 8;

/// Standard base64 with padding optional, as the specification allows.
const BASE64: GeneralPurpose = GeneralPurpose::new(
    &base64::alphabet::STANDARD,
    GeneralPurposeConfig::new().with_decode_padding_mode(DecodePaddingMode::Indifferent),
);

/// One OSC 7501 sequence a program wrote.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum ProgramStatusReport {
    /// `OSC 7501 ; ? ST`: does this terminal understand the protocol?
    Query,
    Status(ProgramStatus),
}

/// A report, validated: the keys Condr reads from the record it would replace.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ProgramStatus {
    pub state: ProgramState,
    /// The record addressed, a `/`-separated path; `None` is the root record.
    pub id: Option<String>,
    /// What a `blocked` program waits for; absent for every other state.
    pub kind: Option<BlockedKind>,
    /// The program's stable identifier, such as `claude-code`.
    pub app: Option<String>,
    /// One line about the activity, wait or result, decoded.
    pub msg: Option<String>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ProgramState {
    Idle,
    Working,
    Done,
    Blocked,
    Error,
    /// Not a state: delete the addressed record and its descendants.
    Clear,
}

impl ProgramState {
    fn parse(value: &str) -> Option<Self> {
        Some(match value {
            "idle" => Self::Idle,
            "working" => Self::Working,
            "done" => Self::Done,
            "blocked" => Self::Blocked,
            "error" => Self::Error,
            "clear" => Self::Clear,
            _ => return None,
        })
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum BlockedKind {
    /// Approval to act.
    Permission,
    /// The user must type a reply.
    Question,
    /// A login, token or credential.
    Auth,
}

impl BlockedKind {
    fn parse(value: &str) -> Option<Self> {
        Some(match value {
            "permission" => Self::Permission,
            "question" => Self::Question,
            "auth" => Self::Auth,
            _ => return None,
        })
    }

    /// What to show when the report carried no `msg`.
    pub const fn label(self) -> &'static str {
        match self {
            Self::Permission => "Permission",
            Self::Question => "Question",
            Self::Auth => "Login",
        }
    }
}

impl ProgramStatusReport {
    /// Parses an OSC payload (everything between `ESC ]` and the terminator) that starts
    /// with [`PROGRAM_STATUS_OSC_PREFIX`]. `None` is a report the specification says to
    /// ignore: over a limit, no or unknown `state`, a malformed `id`, text that does not
    /// decode or carries a control character.
    pub fn parse(payload: &[u8]) -> Option<Self> {
        let body = payload.strip_prefix(PROGRAM_STATUS_OSC_PREFIX.as_bytes())?;
        // `ESC ]` and a BEL around the payload; an ST makes it one byte longer.
        if payload.len() + 3 > MAX_SEQUENCE_BYTES {
            return None;
        }
        // A byte outside UTF-8 is outside the value set too: its pair is skipped below,
        // not the report.
        let body = String::from_utf8_lossy(body);
        if body.trim() == "?" {
            return Some(Self::Query);
        }
        let mut state = None;
        let mut id = None;
        let mut kind = None;
        let mut app = None;
        let mut msg = None;
        for pair in body.split(':') {
            // A bad pair is skipped; the rest of the report still counts.
            let Some((key, value)) = pair.split_once('=') else {
                continue;
            };
            let (key, value) = (key.trim(), value.trim());
            if key.is_empty()
                || !key.bytes().all(|b| b.is_ascii_lowercase())
                || !value.bytes().all(valid_value_byte)
            {
                continue;
            }
            if key.len() > MAX_KEY_BYTES {
                return None;
            }
            // A repeated key's last value wins; unknown keys are how the protocol grows.
            match key {
                "state" => state = Some(ProgramState::parse(value)?),
                "id" => id = Some(valid_id(value)?.to_owned()),
                "kind" => kind = BlockedKind::parse(value),
                "app" => app = valid_app(value).map(str::to_owned),
                "msg" => msg = decode_text(value, MAX_MSG_ENCODED_BYTES, MAX_MSG_DECODED_BYTES)?,
                "title" => {
                    decode_text(value, MAX_TITLE_ENCODED_BYTES, MAX_TITLE_DECODED_BYTES)?;
                }
                _ => {}
            }
        }
        let state = state?;
        Some(Self::Status(ProgramStatus {
            state,
            id,
            kind: kind.filter(|_| state == ProgramState::Blocked),
            app,
            msg,
        }))
    }
}

fn valid_value_byte(byte: u8) -> bool {
    byte.is_ascii_alphanumeric() || matches!(byte, b'_' | b'.' | b',' | b'+' | b'/' | b'=' | b'-')
}

fn valid_segment_byte(byte: u8) -> bool {
    byte.is_ascii_alphanumeric() || matches!(byte, b'_' | b'.' | b'+' | b'-')
}

/// `segment ("/" segment)*`, each segment 1 to 32 bytes, at most 8 levels and 128 bytes.
fn valid_id(id: &str) -> Option<&str> {
    let segments = id.split('/');
    (id.len() <= MAX_ID_BYTES
        && segments.clone().count() <= MAX_ID_LEVELS
        && segments.clone().all(|segment| {
            (1..=MAX_ID_SEGMENT_BYTES).contains(&segment.len())
                && segment.bytes().all(valid_segment_byte)
        }))
    .then_some(id)
}

/// An invalid `app` is treated as absent rather than failing the report.
fn valid_app(app: &str) -> Option<&str> {
    ((1..=MAX_APP_BYTES).contains(&app.len()) && app.bytes().all(valid_segment_byte)).then_some(app)
}

/// Decodes a base64 UTF-8 line. The encoded size is checked before decoding, as the
/// specification asks; a control character anywhere fails the whole report. The outer
/// `None` is that failure; an empty value decodes to no text.
fn decode_text(encoded: &str, max_encoded: usize, max_decoded: usize) -> Option<Option<String>> {
    if encoded.len() > max_encoded {
        return None;
    }
    let bytes = BASE64.decode(encoded).ok()?;
    if bytes.len() > max_decoded {
        return None;
    }
    let text = String::from_utf8(bytes).ok()?;
    if text.chars().any(char::is_control) {
        return None;
    }
    Some((!text.is_empty()).then_some(text))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse(body: &str) -> Option<ProgramStatusReport> {
        ProgramStatusReport::parse(format!("7501;{body}").as_bytes())
    }

    fn status(body: &str) -> ProgramStatus {
        match parse(body) {
            Some(ProgramStatusReport::Status(status)) => status,
            other => panic!("{body:?} parsed to {other:?}"),
        }
    }

    /// The specification's own examples, byte for byte.
    #[test]
    fn the_specifications_examples_parse() {
        assert_eq!(parse("?"), Some(ProgramStatusReport::Query));
        assert_eq!(parse(" ? "), Some(ProgramStatusReport::Query));
        assert_eq!(
            status(
                "state=blocked:kind=permission:app=terraform:msg=QXBwbHkgMyB0byBhZGQsIDEgdG8gY2hhbmdlLCAwIHRvIGRlc3Ryb3k/"
            ),
            ProgramStatus {
                state: ProgramState::Blocked,
                id: None,
                kind: Some(BlockedKind::Permission),
                app: Some("terraform".into()),
                msg: Some("Apply 3 to add, 1 to change, 0 to destroy?".into()),
            }
        );
        // Padding is optional, and a child record names its parent in its id.
        assert_eq!(
            status("state=done:id=us-east:title=VVMgRWFzdA==:msg=SGVhbHRoeQ"),
            ProgramStatus {
                state: ProgramState::Done,
                id: Some("us-east".into()),
                kind: None,
                app: None,
                msg: Some("Healthy".into()),
            }
        );
        assert_eq!(
            status("state=clear:id=build/test").id.as_deref(),
            Some("build/test")
        );
        assert_eq!(status("state=working:app=brew:msg=").msg, None);
    }

    #[test]
    fn pairs_are_lenient_and_limits_are_strict() {
        // Whitespace is stripped, a bad pair is skipped, the last repeat wins, unknown
        // keys and an unknown kind are ignored, and kind only means something blocked.
        let lenient = status(
            " state=working : state = blocked :junk: =x : kind=permission : progress=40 : later=1 ",
        );
        assert_eq!(lenient.state, ProgramState::Blocked);
        assert_eq!(lenient.kind, Some(BlockedKind::Permission));
        assert_eq!(status("state=blocked:kind=dance").kind, None);
        assert_eq!(status("state=working:kind=permission").kind, None);
        assert_eq!(status("state=working:app=has space").app, None);
        assert_eq!(status("state=working:app=").app, None);

        // No state, an unknown state, a bad id, bad base64 or a control character
        // drop the whole report; so does a key over 16 bytes or a sequence over 4096.
        assert_eq!(parse("app=brew"), None);
        assert_eq!(parse("state=napping"), None);
        assert_eq!(parse("state=working:id=/x"), None);
        assert_eq!(parse("state=working:id=a//b"), None);
        assert_eq!(parse("state=working:id=a/b/c/d/e/f/g/h/i"), None);
        // A byte outside the value set makes a bad pair, skipped like any other; base64
        // that is in the set but does not decode fails the report.
        assert_eq!(status("state=working:msg=!!!").msg, None);
        assert_eq!(parse("state=working:msg=A"), None);
        // "a\nb"
        assert_eq!(parse("state=working:msg=YQpi"), None);
        assert_eq!(parse("state=working:abcdefghijklmnopq=1"), None);
        let long = "A".repeat(2733);
        assert_eq!(parse(&format!("state=working:msg={long}")), None);
        // 2049 bytes decoded, within the encoded limit.
        let decoded_long = BASE64.encode("a".repeat(2049));
        assert_eq!(parse(&format!("state=working:msg={decoded_long}")), None);
        let title = BASE64.encode("t".repeat(193));
        assert_eq!(parse(&format!("state=working:title={title}")), None);
        assert_eq!(
            parse(&format!("state=working:title={}", "A".repeat(257))),
            None
        );
        let huge = format!("state=working:{}", "x=1:".repeat(1100));
        assert_eq!(parse(&huge), None);
        assert_eq!(ProgramStatusReport::parse(b"7500;state=working"), None);
        // Ids past 128 bytes or a 32-byte segment fail; an app past 32 bytes is absent.
        let segments = format!("state=working:id={}", ["abcdefghijklmnop"; 8].join("/"));
        assert_eq!(parse(&segments).is_some(), segments.len() - 17 <= 128);
        assert_eq!(parse(&format!("state=working:id={}", "a".repeat(33))), None);
        assert!(
            parse(&format!(
                "state=working:id={}/{}",
                "a".repeat(32),
                "b".repeat(32)
            ))
            .is_some()
        );
        assert_eq!(
            parse(&format!("state=working:id={}", ["a"; 9].join("/"))),
            None
        );
        assert_eq!(
            status(&format!("state=working:app={}", "a".repeat(33))).app,
            None
        );
        // A byte outside UTF-8 spoils only its pair.
        assert_eq!(
            ProgramStatusReport::parse(b"7501;state=working:app=co\xffdex:msg=aGk="),
            Some(ProgramStatusReport::Status(ProgramStatus {
                state: ProgramState::Working,
                id: None,
                kind: None,
                app: None,
                msg: Some("hi".into()),
            }))
        );
    }
}
