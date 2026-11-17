//! Tasks (ADR 0068): work a person writes down before any Agent starts, which a Server
//! keeps and starts. A started Task holds one Workspace; nothing stores a state, since To
//! do, started and archived follow from the fields.

use crate::{AgentKind, PaneId, TabId, WorkspaceId};
use std::path::PathBuf;

/// The largest description a Task holds, in bytes.
pub const MAX_TASK_DESCRIPTION_BYTES: usize = 64 * 1024;
/// The longest title, in bytes.
pub const MAX_TASK_TITLE_BYTES: usize = 1024;

/// What a failed Task request answers with, as `.error.code` (ADR 0068). Messages are for
/// people; Clients match on these.
pub mod task_error {
    pub const TASK_NOT_FOUND: &str = "task_not_found";
    /// The CLI's: no Task holds the Workspace of the Pane it runs in.
    pub const NO_TASK: &str = "no_task";
    pub const TASK_STARTING: &str = "task_starting";
    pub const TASK_STARTED: &str = "task_started";
    pub const TASK_ARCHIVED: &str = "task_archived";
    pub const WORKSPACE_HAS_TASK: &str = "workspace_has_task";
    pub const WHERE_REQUIRED: &str = "where_required";
    pub const INVALID_TASK: &str = "invalid_task";
    pub const WORKSPACE_NOT_FOUND: &str = "workspace_not_found";
    pub const NO_REPOSITORY: &str = "no_repository";
    pub const REPOSITORY_NOT_OPEN: &str = "repository_not_open";
    pub const NO_LAST_PLACE: &str = "no_last_place";
    pub const FOLDER_MISSING: &str = "folder_missing";
    pub const BRANCH_CHECKED_OUT: &str = "branch_checked_out";
    pub const AGENT_UNAVAILABLE: &str = "agent_unavailable";
    pub const AGENT_UNSUPPORTED: &str = "agent_unsupported";
    /// The task file's path holds a character an npm `.cmd` shim refuses.
    pub const UNSUPPORTED_PATH: &str = "unsupported_path";
    pub const WORKSPACE_FAILED: &str = "workspace_failed";
    pub const TASK_FILE_FAILED: &str = "task_file_failed";
    pub const SHELL_NOT_READY: &str = "shell_not_ready";
    pub const AGENT_START_FAILED: &str = "agent_start_failed";
    pub const WORKTREE_NOT_REMOVABLE: &str = "worktree_not_removable";
    pub const SERVER_STOPPING: &str = "server_stopping";
    /// The Server hit an error of its own; its log has the details.
    pub const INTERNAL_ERROR: &str = "internal_error";
}

/// A Task's id: a UUIDv7 its Server gives it, unique across Devices and ordered by when
/// it was made (ADR 0068). It reads as the hyphenated lowercase form.
#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct TaskId(uuid::Uuid);

impl TaskId {
    pub const fn from_bytes(bytes: [u8; 16]) -> Self {
        Self(uuid::Uuid::from_bytes(bytes))
    }

    pub const fn as_bytes(&self) -> &[u8; 16] {
        self.0.as_bytes()
    }

    pub const fn from_uuid(id: uuid::Uuid) -> Self {
        Self(id)
    }

    /// The id whose 128 bits are `value`, for tests and fixtures.
    pub const fn from_u128(value: u128) -> Self {
        Self(uuid::Uuid::from_u128(value))
    }
}

impl std::fmt::Display for TaskId {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        self.0.hyphenated().fmt(formatter)
    }
}

impl std::str::FromStr for TaskId {
    type Err = uuid::Error;

    fn from_str(text: &str) -> Result<Self, Self::Err> {
        text.parse().map(Self)
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Task {
    pub id: TaskId,
    pub title: String,
    /// Plain text; `None` where only the summary travels: the Bootstrap, events and a list.
    pub description: Option<String>,
    /// The root directory of a repository Workspace on the Device.
    pub repository: Option<PathBuf>,
    /// Its place in To do, lowest first.
    pub order: i64,
    pub archived: bool,
    /// The Workspace a started Task holds.
    pub workspace: Option<WorkspaceId>,
    /// A Start of this Task made that Workspace, a new worktree or its folder opened
    /// again, rather than borrowing one already open; archiving it closes the Workspace
    /// by default (ADR 0068). False while it holds none.
    pub made_workspace: bool,
    /// A Start of it is under way. The Server's alone; not stored.
    pub starting: bool,
    /// Where it was last worked.
    pub last: Option<LastPlace>,
}

/// Where a Task was last worked: what Continue opens again.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct LastPlace {
    pub path: PathBuf,
    pub branch: Option<String>,
    /// Its Base Branch (ADR 0034).
    pub base_branch: Option<String>,
    /// Condr made it as a Managed Worktree.
    pub managed: bool,
    /// The repository a worktree belongs to.
    pub repository: Option<PathBuf>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum TaskStatus {
    ToDo,
    Starting,
    Started,
    Archived,
}

impl TaskStatus {
    /// As the CLI writes it.
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::ToDo => "todo",
            Self::Starting => "starting",
            Self::Started => "started",
            Self::Archived => "archived",
        }
    }
}

impl Task {
    pub fn status(&self) -> TaskStatus {
        if self.archived {
            TaskStatus::Archived
        } else if self.workspace.is_some() {
            TaskStatus::Started
        } else if self.starting {
            TaskStatus::Starting
        } else {
            TaskStatus::ToDo
        }
    }

    /// Without its description, as the Bootstrap, events and lists carry it.
    pub fn summary(&self) -> Self {
        Self {
            description: None,
            ..self.clone()
        }
    }
}

/// Where a Start runs.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum StartPlace {
    /// The Task's last place: a Workspace open on it, or the folder opened again.
    Continue,
    /// A new Tab, named after the Task, in this Workspace.
    Workspace(WorkspaceId),
    /// A new Managed Worktree of the Task's repository; `None` takes the default branch.
    NewWorktree { branch: Option<String> },
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum TaskCommand {
    Create {
        title: String,
        description: String,
        repository: Option<PathBuf>,
    },
    /// `None` leaves a field as it is; `Some(None)` clears the repository.
    Edit {
        task: TaskId,
        title: Option<String>,
        description: Option<String>,
        repository: Option<Option<PathBuf>>,
    },
    /// Only a To do or archived Task.
    Delete {
        task: TaskId,
    },
    Archive {
        task: TaskId,
        close_workspace: bool,
        remove_worktree: bool,
    },
    Restore {
        task: TaskId,
    },
    /// Moves a To do Task to `index` among the To do Tasks.
    Reorder {
        task: TaskId,
        index: u32,
    },
    /// With its description.
    Get {
        task: TaskId,
    },
    /// The archived Tasks, or the others; without descriptions.
    List {
        archived: bool,
    },
    /// `place: None` takes the Server's default (ADR 0068).
    Start {
        task: TaskId,
        place: Option<StartPlace>,
        agent: AgentKind,
    },
}

// A one-off reply, never stored in bulk; Windows' larger `PathBuf` is what trips the lint.
#[allow(clippy::large_enum_variant)]
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum TaskResponse {
    Task(Task),
    Tasks(Vec<Task>),
    Started {
        workspace_id: WorkspaceId,
        tab_id: TabId,
        pane_id: PaneId,
    },
    Done,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct TaskError {
    /// One of [`task_error`].
    pub code: String,
    pub message: String,
    /// Which step of a Start failed: `workspace`, `task_file` or `agent`.
    pub step: Option<String>,
}

impl TaskError {
    pub fn new(code: &str, message: impl Into<String>) -> Self {
        Self {
            code: code.into(),
            message: message.into(),
            step: None,
        }
    }

    pub fn at(mut self, step: &str) -> Self {
        self.step = Some(step.into());
        self
    }
}

/// The branch a New worktree takes by default (ADR 0068): `worktree/<slug>` of the title,
/// as Create Worktree names one, or, for a title with any character outside ASCII,
/// `worktree/<repository slug>-<first 8 hex digits of SHA-256(title)>`.
#[cfg(feature = "runtime")]
pub fn default_task_branch(title: &str, repository: &std::path::Path) -> String {
    if title.is_ascii() {
        return format!(
            "worktree/{}",
            slug(title).unwrap_or_else(|| "change".into())
        );
    }
    use sha2::Digest as _;
    let hash = format!("{:x}", sha2::Sha256::digest(title.as_bytes()));
    let repository = repository
        .file_name()
        .and_then(|name| name.to_str())
        .and_then(slug)
        .unwrap_or_else(|| "repository".into());
    format!("worktree/{repository}-{}", &hash[..8])
}

/// The longest slug a default branch takes, so a long title still names a worktree
/// folder well under a file system's 255-byte limit.
#[cfg(feature = "runtime")]
const MAX_SLUG_CHARACTERS: usize = 60;

#[cfg(feature = "runtime")]
fn slug(text: &str) -> Option<String> {
    let slug = text
        .chars()
        .take(MAX_SLUG_CHARACTERS)
        .map(|character| {
            if character.is_ascii_alphanumeric() || matches!(character, '-' | '_') {
                character.to_ascii_lowercase()
            } else {
                '-'
            }
        })
        .collect::<String>();
    let slug = slug.trim_matches('-');
    (!slug.is_empty()).then(|| slug.to_owned())
}

#[cfg(all(test, feature = "runtime"))]
mod tests {
    use super::*;
    use std::path::Path;

    #[test]
    fn a_default_branch_is_the_title_slug_or_a_hash_of_a_non_ascii_title() {
        let repository = Path::new("/src/API Server");
        assert_eq!(
            default_task_branch("Fix the login bug", repository),
            "worktree/fix-the-login-bug"
        );
        assert_eq!(default_task_branch("!!!", repository), "worktree/change");
        let long = default_task_branch(&"Make it faster ".repeat(40), repository);
        assert_eq!(long.len(), "worktree/".len() + 59, "{long}");
        assert!(!long.ends_with('-'));
        // SHA-256("abc") is ba7816bf…, so a non-ASCII title is checked against the rule
        // with the digest computed the same way.
        use sha2::Digest as _;
        assert_eq!(
            &format!("{:x}", sha2::Sha256::digest(b"abc"))[..8],
            "ba7816bf"
        );
        let branch = default_task_branch("修复输入法", repository);
        let hash = format!("{:x}", sha2::Sha256::digest("修复输入法".as_bytes()));
        assert_eq!(branch, format!("worktree/api-server-{}", &hash[..8]));
        assert_ne!(branch, default_task_branch("Fix 中文 IME bug", repository));
        assert_eq!(
            default_task_branch("中", Path::new("/")),
            default_task_branch("中", Path::new("/x/!!"))
        );
    }

    #[test]
    fn a_status_follows_from_the_fields() {
        let mut task = Task {
            id: TaskId::from_u128(1),
            title: "t".into(),
            description: Some("d".into()),
            repository: None,
            order: 0,
            archived: false,
            workspace: None,
            made_workspace: false,
            starting: false,
            last: None,
        };
        assert_eq!(task.status(), TaskStatus::ToDo);
        task.starting = true;
        assert_eq!(task.status(), TaskStatus::Starting);
        task.workspace = Some(WorkspaceId::from_u64(3));
        assert_eq!(task.status(), TaskStatus::Started);
        task.archived = true;
        assert_eq!(task.status(), TaskStatus::Archived);
        assert_eq!(task.summary().description, None);
    }
}
