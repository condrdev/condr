use super::*;
use crate::uri::percent_decode;
use std::borrow::Cow;

#[derive(Default)]
pub(super) struct OscScanner {
    state: OscState,
    payload: Vec<u8>,
    /// Bytes of a possible agent event held back from the VT across reads.
    held: Vec<u8>,
}

#[derive(Clone, Copy, Default)]
pub(super) enum OscState {
    #[default]
    Ground,
    /// An `ESC` was seen and held; the next byte says what it starts.
    Escape,
    Payload,
    PayloadEscape,
    Discard,
    DiscardEscape,
    /// Inside DCS/SOS/PM/APC, where BEL is payload and ST ends the string; a bare `ESC`
    /// starting something else ends it too, as the VT reads it.
    ControlString,
    ControlStringEscape,
}

/// The OSC payloads Condr reads itself because vte drops them: cwd reports as OSC 7
/// `file://` URIs, ConEmu `9;9;<cwd>` and iTerm2 `1337;CurrentDir=<cwd>`, and the OSC 777
/// agent events `condr agent-hook` writes (ADR 0014). This scanner runs over the raw bytes.
#[derive(Clone, Debug, PartialEq)]
pub(super) enum OscReport {
    Cwd(PathBuf),
    Agent(AgentEvent),
}

/// What the VT gets from one read: the bytes as they came, or a copy with the agent
/// event sequences cut out. Everything from an `ESC` is held back until the scanner
/// knows the sequence is not an agent event, which for anything else is settled within
/// a few bytes, so ordinary output is borrowed, not copied.
struct VtFilter<'a, 'h> {
    bytes: &'a [u8],
    carry: &'h mut Vec<u8>,
    out: Option<Vec<u8>>,
    /// How much of `bytes` is accounted for in `out` (passed or dropped).
    emitted: usize,
    hold_from: Option<usize>,
}

impl<'a, 'h> VtFilter<'a, 'h> {
    fn new(bytes: &'a [u8], carry: &'h mut Vec<u8>) -> Self {
        let hold_from = (!carry.is_empty()).then_some(0);
        Self {
            bytes,
            carry,
            out: None,
            emitted: 0,
            hold_from,
        }
    }

    fn hold(&mut self, at: usize) {
        self.hold_from = Some(at);
    }

    /// Holds from `at` unless an earlier hold is still open.
    fn hold_if_free(&mut self, at: usize) {
        self.hold_from.get_or_insert(at);
    }

    /// Makes `out` real and current up to `upto`.
    fn materialize(&mut self, upto: usize) {
        match &mut self.out {
            Some(out) => out.extend_from_slice(&self.bytes[self.emitted..upto]),
            None => self.out = Some(self.bytes[..upto].to_vec()),
        }
        self.emitted = upto;
    }

    /// The held bytes were not an agent event after all: the VT gets them, in order.
    fn release(&mut self, end: usize) {
        let Some(from) = self.hold_from.take() else {
            return;
        };
        if self.out.is_none() && self.carry.is_empty() {
            return;
        }
        self.materialize(from);
        let out = self.out.as_mut().expect("materialized");
        out.append(self.carry);
        out.extend_from_slice(&self.bytes[from..end]);
        self.emitted = end;
    }

    /// The held bytes were an agent event: the VT never sees them.
    fn discard(&mut self, end: usize) {
        let Some(from) = self.hold_from.take() else {
            return;
        };
        self.materialize(from);
        self.carry.clear();
        self.emitted = end;
    }

    /// Like `release`/`discard` up to `end`, except that the last held byte, the `ESC`
    /// that ended the string, stays held: it starts whatever comes next.
    fn settle_keeping_escape(&mut self, end: usize, keep: bool) {
        let Some(from) = self.hold_from else {
            return;
        };
        if end > from {
            let escape = end - 1;
            if keep {
                self.release(escape);
            } else {
                self.discard(escape);
            }
            self.hold_from = Some(escape);
        } else {
            // The whole string, ESC included, arrived in earlier reads.
            let escape = self.carry.pop();
            if keep {
                self.release(end);
            } else {
                self.discard(end);
            }
            if let Some(escape) = escape {
                self.carry.push(escape);
                self.hold_from = Some(end);
            }
        }
    }

    fn finish(mut self) -> Cow<'a, [u8]> {
        if let Some(from) = self.hold_from.take() {
            self.materialize(from);
            self.carry.extend_from_slice(&self.bytes[from..]);
            self.emitted = self.bytes.len();
        }
        match self.out {
            Some(mut out) => {
                out.extend_from_slice(&self.bytes[self.emitted..]);
                Cow::Owned(out)
            }
            None => Cow::Borrowed(self.bytes),
        }
    }
}

/// Where a string ended.
#[derive(Clone, Copy)]
enum Terminator {
    /// BEL or ST: the terminator belongs to the string.
    Proper,
    /// A bare `ESC` followed by something else, as the VT reads it: the string ends, and
    /// the `ESC` begins the next sequence.
    Escape,
}

impl OscScanner {
    /// Scans one read, reporting what Condr consumes itself, and returns what the VT
    /// should see.
    pub(super) fn advance<'a>(
        &mut self,
        bytes: &'a [u8],
        mut report: impl FnMut(OscReport),
    ) -> Cow<'a, [u8]> {
        let mut held = std::mem::take(&mut self.held);
        let mut vt = VtFilter::new(bytes, &mut held);
        for (index, &byte) in bytes.iter().enumerate() {
            match self.state {
                OscState::Ground => {
                    if byte == 0x1b {
                        vt.hold(index);
                        self.state = OscState::Escape;
                    }
                }
                OscState::Escape => self.after_escape(byte, index, &mut vt),
                OscState::Payload => match byte {
                    0x07 => self.finish(index, Terminator::Proper, &mut vt, &mut report),
                    0x1b => {
                        // Held even when the payload was released: this ESC may start
                        // an agent event, as the VT would read it.
                        vt.hold_if_free(index);
                        self.state = OscState::PayloadEscape;
                    }
                    _ => self.push_payload(byte, index, &mut vt),
                },
                OscState::PayloadEscape => match byte {
                    b'\\' => self.finish(index, Terminator::Proper, &mut vt, &mut report),
                    _ => {
                        self.finish(index, Terminator::Escape, &mut vt, &mut report);
                        self.after_escape(byte, index, &mut vt);
                    }
                },
                OscState::Discard | OscState::ControlString => match byte {
                    0x07 if matches!(self.state, OscState::Discard) => self.reset(),
                    0x1b => {
                        vt.hold(index);
                        self.state = if matches!(self.state, OscState::Discard) {
                            OscState::DiscardEscape
                        } else {
                            OscState::ControlStringEscape
                        };
                    }
                    _ => {}
                },
                OscState::DiscardEscape | OscState::ControlStringEscape => match byte {
                    b'\\' => {
                        vt.release(index + 1);
                        self.reset();
                    }
                    _ => {
                        self.reset();
                        self.after_escape(byte, index, &mut vt);
                    }
                },
            }
        }
        let filtered = vt.finish();
        self.held = held;
        filtered
    }

    /// The byte after a held `ESC` decides what it started.
    fn after_escape(&mut self, byte: u8, index: usize, vt: &mut VtFilter<'_, '_>) {
        match byte {
            b']' => {
                self.payload.clear();
                self.state = OscState::Payload;
            }
            0x1b => {
                // ESC ESC: the first one was nothing; this one may start something.
                vt.release(index);
                vt.hold(index);
                self.state = OscState::Escape;
            }
            b'P' | b'X' | b'^' | b'_' => {
                // DCS/SOS/PM/APC: whatever looks like an OSC inside is payload.
                vt.release(index + 1);
                self.state = OscState::ControlString;
            }
            _ => {
                vt.release(index + 1);
                self.state = OscState::Ground;
            }
        }
    }

    /// Whether the payload so far can still turn out to be an agent event.
    fn could_be_agent_event(&self) -> bool {
        let prefix = AGENT_EVENT_OSC_PREFIX.as_bytes();
        prefix.starts_with(&self.payload) || self.payload.starts_with(prefix)
    }

    fn push_payload(&mut self, byte: u8, index: usize, vt: &mut VtFilter<'_, '_>) {
        if self.payload.len() == MAX_OSC_CWD_BYTES {
            self.payload.clear();
            self.state = OscState::Discard;
            vt.release(index + 1);
        } else {
            self.payload.push(byte);
            if !self.could_be_agent_event() {
                vt.release(index + 1);
            }
        }
    }

    /// The OSC is complete. `index` is the byte that ended it: the BEL, the `\` of an ST,
    /// or the byte after a bare `ESC`.
    fn finish(
        &mut self,
        index: usize,
        terminator: Terminator,
        vt: &mut VtFilter<'_, '_>,
        report: &mut impl FnMut(OscReport),
    ) {
        let ours = self.payload.starts_with(AGENT_EVENT_OSC_PREFIX.as_bytes());
        if ours {
            // Ours, well-formed or not: the terminal has no use for it either way.
            if let Some(event) = AgentEvent::decode(&self.payload) {
                report(OscReport::Agent(event));
            }
        } else if let Ok(payload) = std::str::from_utf8(&self.payload) {
            if let Some(cwd) = payload.strip_prefix("9;9;") {
                let cwd = cwd
                    .strip_prefix('"')
                    .and_then(|cwd| cwd.strip_suffix('"'))
                    .unwrap_or(cwd);
                report(OscReport::Cwd(PathBuf::from(cwd)));
            } else if let Some(cwd) = payload.strip_prefix("7;").and_then(file_uri_cwd) {
                report(OscReport::Cwd(cwd));
            } else if let Some(cwd) = payload
                .strip_prefix("1337;CurrentDir=")
                .filter(|cwd| !cwd.is_empty())
            {
                report(OscReport::Cwd(PathBuf::from(cwd)));
            }
        }
        match (terminator, ours) {
            (Terminator::Proper, true) => vt.discard(index + 1),
            (Terminator::Proper, false) => vt.release(index + 1),
            (Terminator::Escape, ours) => vt.settle_keeping_escape(index, !ours),
        }
        self.reset();
    }

    fn reset(&mut self) {
        self.payload.clear();
        self.state = OscState::Ground;
    }
}

/// OSC 7 carries `file://[host]/path`; only this machine's paths are usable.
fn file_uri_cwd(uri: &str) -> Option<PathBuf> {
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
