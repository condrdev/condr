//! The Program Status Protocol, OSC 7501 (ADR 0044): a program tells its terminal what
//! it is doing. libghostty-vt parses the reports and answers the feature query (ADR 0051);
//! these are the reports as Condr keeps them, and what Condr makes of them is the
//! detector's business.
//!
//! <https://www.superlogical.com/rex/docs/build/program-status>

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
    /// What to show when the report carried no `msg`.
    pub const fn label(self) -> &'static str {
        match self {
            Self::Permission => "Permission",
            Self::Question => "Question",
            Self::Auth => "Login",
        }
    }
}
