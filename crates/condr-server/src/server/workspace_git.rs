//! What the Server knows about a Workspace's repository (ADR 0017): the discovered
//! repository plus its working-tree changes, recomputed as one unit so branch, upstream and
//! file list never disagree, and the filesystem watcher that asks for those recomputations.

use super::*;
use condr_core::{GitChanges, GitError};
use notify::Watcher as _;
use std::collections::HashMap;
use std::path::Path;
use std::sync::Weak;

/// One Workspace's Git state. Equality decides whether a refresh is worth an event.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(super) struct WorkspaceGit {
    pub(super) repository: GitRepository,
    pub(super) changes: GitChanges,
}

impl WorkspaceGit {
    /// A repository just discovered, before its changes were computed; the watcher's first
    /// refresh fills them in off the state lock.
    pub(super) fn discovered(repository: GitRepository) -> Self {
        Self {
            repository,
            changes: GitChanges::default(),
        }
    }

    /// Everything the sidebar shows, computed in one pass. Runs unlocked: status walks the
    /// whole work tree.
    pub(super) fn scan(root: &Path) -> Result<Option<Self>, GitError> {
        discover_repository(root)?
            .map(Self::from_repository)
            .transpose()
    }

    /// The change list of a repository already discovered; the expensive half of `scan`.
    pub(super) fn from_repository(repository: GitRepository) -> Result<Self, GitError> {
        let changes = repository.changes()?;
        Ok(Self {
            repository,
            changes,
        })
    }
}

pub(super) fn workspace_git_snapshot(
    workspace_id: WorkspaceId,
    git: &WorkspaceGit,
) -> WorkspaceGitSnapshot {
    WorkspaceGitSnapshot {
        workspace_id,
        branch: git.repository.branch().map(str::to_owned),
        linked_worktree: git.repository.is_linked_worktree(),
        upstream: git.repository.upstream(),
        changes: git.changes.clone(),
    }
}

/// Records a scan result and tells clients when it differs from what they know. `false`
/// when the Workspace is gone or has moved, so the result was for nobody.
pub(super) fn apply_workspace_git_refresh(
    state: &mut RuntimeState,
    workspace_id: WorkspaceId,
    root: &Path,
    next: Option<WorkspaceGit>,
) -> bool {
    if state
        .session
        .workspace(workspace_id)
        .is_none_or(|workspace| workspace.root_directory() != root)
    {
        return false;
    }
    if state.workspace_git.get(&workspace_id) == next.as_ref() {
        return true;
    }
    let git = match next {
        Some(git) => {
            let snapshot = workspace_git_snapshot(workspace_id, &git);
            state.watch_workspace_git(workspace_id, root, Some(&git.repository));
            state.workspace_git.insert(workspace_id, git);
            Some(snapshot)
        }
        None => {
            state.workspace_git.remove(&workspace_id);
            None
        }
    };
    state.publish_background(SessionEvent::WorkspaceGitChanged { workspace_id, git });
    true
}

/// Installs a freshly discovered repository for a Workspace that was just created or
/// restored, announces it, and asks the watcher for the first full scan.
pub(super) fn set_workspace_git(
    state: &mut RuntimeState,
    workspace_id: WorkspaceId,
    git: Option<GitRepository>,
) {
    state
        .workspace_git_scanned_at
        .insert(workspace_id, Instant::now());
    let git = git.map(WorkspaceGit::discovered);
    let changed = state.workspace_git.get(&workspace_id) != git.as_ref();
    let root = state
        .session
        .workspace(workspace_id)
        .map(|workspace| workspace.root_directory().to_path_buf());
    if let Some(root) = &root {
        state.watch_workspace_git(workspace_id, root, git.as_ref().map(|git| &git.repository));
    }
    match git {
        Some(git) => {
            state.workspace_git.insert(workspace_id, git);
        }
        None => {
            state.workspace_git.remove(&workspace_id);
        }
    }
    // Clients learn Git state from events now that a layout change no longer makes
    // them fetch a Bootstrap; a Workspace created on a repository announces it here.
    if changed {
        let git = state
            .workspace_git
            .get(&workspace_id)
            .map(|git| workspace_git_snapshot(workspace_id, git));
        state.publish_background(SessionEvent::WorkspaceGitChanged { workspace_id, git });
    }
    if let Some(watcher) = &state.git_watcher {
        watcher.refresh(workspace_id);
    }
}

impl RuntimeState {
    /// Starts the filesystem watcher once the state has its shared handle, and puts every
    /// restored Workspace under watch with a first full scan.
    pub(super) fn start_git_watcher(&mut self, state: Weak<Mutex<RuntimeState>>) {
        let watcher = GitWatcher::start(state);
        for workspace in self.session.workspaces() {
            let git_dir = self
                .workspace_git
                .get(&workspace.id())
                .map(|git| git.repository.git_directory().to_path_buf());
            watcher.watch(
                workspace.id(),
                workspace.root_directory().to_path_buf(),
                git_dir,
            );
            watcher.refresh(workspace.id());
        }
        self.git_watcher = Some(watcher);
    }

    fn watch_workspace_git(
        &self,
        workspace_id: WorkspaceId,
        root: &Path,
        repository: Option<&GitRepository>,
    ) {
        if let Some(watcher) = &self.git_watcher {
            watcher.watch(
                workspace_id,
                root.to_path_buf(),
                repository.map(|repository| repository.git_directory().to_path_buf()),
            );
        }
    }

    /// Drops the watches of Workspaces that left the Session.
    pub(super) fn unwatch_closed_workspaces(&self) {
        if let Some(watcher) = &self.git_watcher {
            watcher.retain(
                self.session
                    .workspaces()
                    .iter()
                    .map(|workspace| workspace.id())
                    .collect(),
            );
        }
    }
}

/// Events within this window fold into one recomputation: a save, a checkout or a build
/// touches many files at once.
const GIT_WATCH_DEBOUNCE: Duration = Duration::from_millis(300);

enum WatchMessage {
    Watch {
        workspace_id: WorkspaceId,
        root: PathBuf,
        git_dir: Option<PathBuf>,
    },
    Retain(Vec<WorkspaceId>),
    Refresh(WorkspaceId),
    Filesystem(notify::Result<notify::Event>),
}

/// Watches every Workspace root and recomputes its Git state when files change, off the
/// state lock. `.git/objects` churn and paths the repository ignores do not count.
pub(super) struct GitWatcher {
    messages: mpsc::Sender<WatchMessage>,
}

struct Watched {
    root: PathBuf,
    root_subscribed: bool,
    git_dir: Option<PathBuf>,
    /// A git directory outside the root that has its own watch.
    git_dir_subscribed: Option<PathBuf>,
    /// Paths changed since the last scan, relative to the root; empty with `due` set means
    /// a refresh was asked for outright.
    pending: Vec<PathBuf>,
    due: Option<Instant>,
    forced: bool,
}

impl GitWatcher {
    pub(super) fn start(state: Weak<Mutex<RuntimeState>>) -> Self {
        let (messages, inbox) = mpsc::channel::<WatchMessage>();
        let filesystem = messages.clone();
        let watcher = notify::recommended_watcher(move |event| {
            let _ = filesystem.send(WatchMessage::Filesystem(event));
        });
        thread::spawn(move || run_watcher(watcher, inbox, state));
        Self { messages }
    }

    pub(super) fn watch(&self, workspace_id: WorkspaceId, root: PathBuf, git_dir: Option<PathBuf>) {
        let _ = self.messages.send(WatchMessage::Watch {
            workspace_id,
            root,
            git_dir,
        });
    }

    pub(super) fn refresh(&self, workspace_id: WorkspaceId) {
        let _ = self.messages.send(WatchMessage::Refresh(workspace_id));
    }

    /// Keeps only the listed Workspaces under watch.
    fn retain(&self, keep: Vec<WorkspaceId>) {
        let _ = self.messages.send(WatchMessage::Retain(keep));
    }
}

fn run_watcher(
    watcher: notify::Result<notify::RecommendedWatcher>,
    inbox: mpsc::Receiver<WatchMessage>,
    state: Weak<Mutex<RuntimeState>>,
) {
    let mut watcher = match watcher {
        Ok(watcher) => Some(watcher),
        Err(error) => {
            eprintln!(
                "condr-server: file watching is unavailable, Git changes refresh on terminal activity only: {error}"
            );
            None
        }
    };
    let mut watched: HashMap<WorkspaceId, Watched> = HashMap::new();
    loop {
        let next_due = watched.values().filter_map(|entry| entry.due).min();
        let message = match next_due {
            Some(due) => match inbox.recv_timeout(due.saturating_duration_since(Instant::now())) {
                Ok(message) => Some(message),
                Err(mpsc::RecvTimeoutError::Timeout) => None,
                Err(mpsc::RecvTimeoutError::Disconnected) => return,
            },
            None => match inbox.recv() {
                Ok(message) => Some(message),
                Err(_) => return,
            },
        };
        match message {
            Some(WatchMessage::Watch {
                workspace_id,
                root,
                git_dir,
            }) => {
                let entry = watched.entry(workspace_id).or_insert_with(|| Watched {
                    root: root.clone(),
                    root_subscribed: false,
                    git_dir: None,
                    git_dir_subscribed: None,
                    pending: Vec::new(),
                    due: None,
                    forced: false,
                });
                if entry.root != root {
                    if let Some(watcher) = &mut watcher
                        && entry.root_subscribed
                    {
                        let _ = watcher.unwatch(&entry.root);
                    }
                    entry.root = root;
                    entry.root_subscribed = false;
                }
                if !entry.root_subscribed
                    && let Some(watcher) = &mut watcher
                {
                    entry.root_subscribed =
                        subscribe(watcher, &entry.root, notify::RecursiveMode::Recursive);
                }
                if let Some(git_dir) = git_dir {
                    watch_git_dir(entry, watcher.as_mut(), git_dir);
                }
            }
            Some(WatchMessage::Retain(keep)) => {
                let gone: Vec<WorkspaceId> = watched
                    .keys()
                    .copied()
                    .filter(|id| !keep.contains(id))
                    .collect();
                for workspace_id in gone {
                    if let Some(entry) = watched.remove(&workspace_id)
                        && let Some(watcher) = &mut watcher
                    {
                        if entry.root_subscribed {
                            let _ = watcher.unwatch(&entry.root);
                        }
                        if let Some(git_dir) = entry.git_dir_subscribed {
                            let _ = watcher.unwatch(&git_dir);
                        }
                    }
                }
            }
            Some(WatchMessage::Refresh(workspace_id)) => {
                if let Some(entry) = watched.get_mut(&workspace_id) {
                    entry.forced = true;
                    entry.due = Some(Instant::now());
                }
            }
            Some(WatchMessage::Filesystem(Err(error))) => {
                eprintln!("condr-server: file watcher: {error}");
            }
            Some(WatchMessage::Filesystem(Ok(event))) => {
                let now = Instant::now();
                for path in &event.paths {
                    for entry in watched.values_mut() {
                        if let Some(relative) = relevant_change(entry, path) {
                            if entry.pending.len() < MAX_PENDING_PATHS {
                                entry.pending.push(relative);
                            } else {
                                entry.forced = true;
                            }
                            entry.due = Some(now + GIT_WATCH_DEBOUNCE);
                        }
                    }
                }
            }
            None => {}
        }

        let now = Instant::now();
        let due: Vec<WorkspaceId> = watched
            .iter()
            .filter(|(_, entry)| entry.due.is_some_and(|due| due <= now))
            .map(|(id, _)| *id)
            .collect();
        for workspace_id in due {
            let Some(entry) = watched.get_mut(&workspace_id) else {
                continue;
            };
            entry.due = None;
            let pending = std::mem::take(&mut entry.pending);
            let forced = std::mem::take(&mut entry.forced);
            let root = entry.root.clone();
            // Discovery is cheap; the status walk is not, so the ignore check sits between.
            let next = match discover_repository(&root) {
                Ok(None) => Ok(None),
                Ok(Some(repository)) => {
                    if !forced && repository.ignores_all(&pending) {
                        continue;
                    }
                    watch_git_dir(
                        entry,
                        watcher.as_mut(),
                        repository.git_directory().to_path_buf(),
                    );
                    WorkspaceGit::from_repository(repository).map(Some)
                }
                Err(error) => Err(error),
            };
            let next = match next {
                Ok(next) => next,
                Err(error) => {
                    eprintln!(
                        "condr-server: Git scan of {} failed: {error}",
                        root.display()
                    );
                    continue;
                }
            };
            let Some(state) = state.upgrade() else {
                return;
            };
            let mut state = state.lock().expect("server state lock poisoned");
            if apply_workspace_git_refresh(&mut state, workspace_id, &root, next) {
                // The same batch is what the Files sidebar and Preview Tabs follow
                // (ADR 0018); ignored-only batches were skipped above.
                state.publish_background(SessionEvent::WorkspaceFilesChanged { workspace_id });
            }
        }
    }
}

/// Past this many distinct paths in one window the batch is treated as "everything changed"
/// rather than checked path by path against the ignore rules.
const MAX_PENDING_PATHS: usize = 256;

fn subscribe(
    watcher: &mut notify::RecommendedWatcher,
    path: &Path,
    mode: notify::RecursiveMode,
) -> bool {
    match watcher.watch(path, mode) {
        Ok(()) => true,
        Err(error) => {
            eprintln!(
                "condr-server: cannot watch {} for Git changes: {error}",
                path.display()
            );
            false
        }
    }
}

/// A linked worktree keeps HEAD and index outside its root, so that directory gets its own
/// watch; the main worktree's `.git` is under the recursive root watch already.
fn watch_git_dir(
    entry: &mut Watched,
    watcher: Option<&mut notify::RecommendedWatcher>,
    git_dir: PathBuf,
) {
    if git_dir.starts_with(&entry.root) {
        entry.git_dir = Some(git_dir);
        return;
    }
    if entry.git_dir_subscribed.as_ref() != Some(&git_dir)
        && let Some(watcher) = watcher
    {
        if let Some(previous) = entry.git_dir_subscribed.take() {
            let _ = watcher.unwatch(&previous);
        }
        if subscribe(watcher, &git_dir, notify::RecursiveMode::NonRecursive) {
            entry.git_dir_subscribed = Some(git_dir.clone());
        }
    }
    entry.git_dir = Some(git_dir);
}

/// The path relative to the Workspace root when `path` can change its Git status; the
/// object store never does, and a change under the git directory counts as `.git` itself.
fn relevant_change(entry: &Watched, path: &Path) -> Option<PathBuf> {
    if let Some(git_dir) = &entry.git_dir
        && let Ok(inside) = path.strip_prefix(git_dir)
    {
        return (!inside.starts_with("objects") && !inside.starts_with("lfs"))
            .then(|| PathBuf::from(".git"));
    }
    let relative = path.strip_prefix(&entry.root).ok()?;
    if relative.starts_with(".git") {
        let inside = relative.strip_prefix(".git").unwrap_or(relative);
        return (!inside.starts_with("objects") && !inside.starts_with("lfs"))
            .then(|| PathBuf::from(".git"));
    }
    Some(relative.to_path_buf())
}
