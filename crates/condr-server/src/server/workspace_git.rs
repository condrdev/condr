//! What the Server knows about a Workspace's repositories (ADR 0017, ADR 0073): each one
//! discovered plus its working-tree changes, recomputed as one unit so branch, upstream and
//! file list never disagree, and the filesystem watcher that asks for those recomputations.

use super::*;
use condr_core::protocol::{GitBaseChanges, GitHead, RepositoryGitSnapshot};
use condr_core::{GitBase, GitChanges, GitError, GitFingerprint, workspace_repositories};
use notify::Watcher as _;
use std::collections::HashMap;
use std::panic::{self, AssertUnwindSafe};
use std::path::Path;
use std::sync::Weak;

#[cfg(test)]
mod tests;

/// One repository a Workspace shows: the one its Root is in, or one right under its Root
/// (ADR 0073). Equality decides whether a refresh is worth an event.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(super) struct RepositoryGit {
    pub(super) repository: GitRepository,
    pub(super) changes: GitChanges,
    /// The base and the working tree against it (ADR 0034).
    pub(super) base: Option<(GitBase, GitChanges)>,
}

impl RepositoryGit {
    /// A repository just discovered, before its changes were computed; the watcher's first
    /// refresh fills them in off the state lock.
    pub(super) fn discovered(repository: GitRepository) -> Self {
        Self {
            repository,
            changes: GitChanges::default(),
            base: None,
        }
    }

    /// Everything the sidebar shows, computed in one pass off the state lock.
    pub(super) fn from_repository(
        repository: GitRepository,
        recorded: Option<&str>,
    ) -> Result<Self, GitError> {
        // A base that cannot be resolved costs the base view, never the HEAD one. A
        // repository under the Root has none: one comparison row cannot name a base for
        // each (ADR 0073).
        let base = if repository.directory().as_str().is_empty() {
            repository.base(recorded).unwrap_or_else(|error| {
                tracing::warn!(
                    "cannot resolve the base of {}: {error}",
                    repository.root().display()
                );
                None
            })
        } else {
            None
        };
        let (changes, base_changes) = repository.changes(base.as_ref())?;
        Ok(Self {
            repository,
            changes,
            base: base.zip(base_changes),
        })
    }
}

impl RuntimeState {
    /// The branch Condr recorded as a Managed Worktree's base when it made the branch.
    pub(super) fn recorded_base(&self, workspace_id: WorkspaceId) -> Option<String> {
        let workspace = self.session.workspace(workspace_id)?;
        workspace.worktree()?.base_branch().map(str::to_owned)
    }

    /// The repository a Workspace's Root is in, which alone has worktrees, a base and
    /// Tasks (ADR 0073).
    pub(super) fn root_repository(&self, workspace_id: WorkspaceId) -> Option<&RepositoryGit> {
        self.workspace_git
            .get(&workspace_id)?
            .first()
            .filter(|git| git.repository.directory().as_str().is_empty())
    }
}

pub(super) fn workspace_git_snapshot(
    workspace_id: WorkspaceId,
    repositories: &[RepositoryGit],
) -> WorkspaceGitSnapshot {
    WorkspaceGitSnapshot {
        workspace_id,
        repositories: repositories
            .iter()
            .map(|git| RepositoryGitSnapshot {
                directory: git.repository.directory().to_relative_path_buf(),
                branch: git.repository.branch().map(str::to_owned),
                linked_worktree: git.repository.is_linked_worktree(),
                upstream: git.repository.upstream(),
                changes: git.changes.clone(),
                base: git.base.as_ref().map(|(base, changes)| GitBaseChanges {
                    branch: base.name().to_owned(),
                    changes: changes.clone(),
                }),
                head: Some(GitHead {
                    commit: git.repository.head().map(str::to_owned),
                }),
            })
            .collect(),
    }
}

/// Records a scan result and tells clients when it differs from what they know; no
/// repository at all means the Workspace has no Git. `false` when the Workspace is gone or
/// has moved, so the result was for nobody.
pub(super) fn apply_workspace_git_refresh(
    state: &mut RuntimeState,
    workspace_id: WorkspaceId,
    root: &Path,
    next: Vec<RepositoryGit>,
) -> bool {
    if state
        .session
        .workspace(workspace_id)
        .is_none_or(|workspace| workspace.root_directory() != root)
    {
        return false;
    }
    if state
        .workspace_git
        .get(&workspace_id)
        .map_or(&[][..], Vec::as_slice)
        == next.as_slice()
    {
        return true;
    }
    state.watch_workspace_git(workspace_id, root, &next);
    let git = install_workspace_git(state, workspace_id, next);
    state.publish_background(SessionEvent::WorkspaceGitChanged { workspace_id, git });
    true
}

/// Puts `repositories` in place for the Workspace and gives its snapshot, `None` for none.
fn install_workspace_git(
    state: &mut RuntimeState,
    workspace_id: WorkspaceId,
    repositories: Vec<RepositoryGit>,
) -> Option<WorkspaceGitSnapshot> {
    if repositories.is_empty() {
        state.workspace_git.remove(&workspace_id);
        return None;
    }
    let snapshot = workspace_git_snapshot(workspace_id, &repositories);
    state.workspace_git.insert(workspace_id, repositories);
    Some(snapshot)
}

/// Installs freshly discovered repositories for a Workspace that was just created or
/// restored, announces them, and asks the watcher for the first full scan.
pub(super) fn set_workspace_git(
    state: &mut RuntimeState,
    workspace_id: WorkspaceId,
    repositories: Vec<GitRepository>,
) {
    let repositories: Vec<RepositoryGit> = repositories
        .into_iter()
        .map(RepositoryGit::discovered)
        .collect();
    let changed = state
        .workspace_git
        .get(&workspace_id)
        .map_or(&[][..], Vec::as_slice)
        != repositories.as_slice();
    let root = state
        .session
        .workspace(workspace_id)
        .map(|workspace| workspace.root_directory().to_path_buf());
    if let Some(root) = &root {
        state.watch_workspace_git(workspace_id, root, &repositories);
    }
    let git = install_workspace_git(state, workspace_id, repositories);
    // Clients learn Git state from events now that a layout change no longer makes
    // them fetch a Bootstrap; a Workspace created on a repository announces it here.
    if changed {
        state.publish_background(SessionEvent::WorkspaceGitChanged { workspace_id, git });
    }
    if let Some(watcher) = &state.git_watcher {
        watcher.refresh(workspace_id);
    }
}

/// Each repository's git directory, by its directory under the Root.
fn git_dirs<'a>(
    repositories: impl IntoIterator<Item = &'a GitRepository>,
) -> Vec<(RelativePathBuf, PathBuf)> {
    repositories
        .into_iter()
        .map(|repository| {
            (
                repository.directory().to_relative_path_buf(),
                repository.git_directory().to_path_buf(),
            )
        })
        .collect()
}

impl RuntimeState {
    /// Starts the filesystem watcher once the state has its shared handle, and puts every
    /// restored Workspace under watch with a first full scan.
    pub(super) fn start_git_watcher(&mut self, state: Weak<Mutex<RuntimeState>>) {
        let watcher = GitWatcher::start(state);
        for workspace in self.session.workspaces() {
            let git_dirs = git_dirs(
                self.workspace_git
                    .get(&workspace.id())
                    .into_iter()
                    .flatten()
                    .map(|git| &git.repository),
            );
            watcher.watch(
                workspace.id(),
                workspace.root_directory().to_path_buf(),
                git_dirs,
            );
            watcher.refresh(workspace.id());
        }
        self.git_watcher = Some(watcher);
    }

    fn watch_workspace_git(
        &self,
        workspace_id: WorkspaceId,
        root: &Path,
        repositories: &[RepositoryGit],
    ) {
        if let Some(watcher) = &self.git_watcher {
            watcher.watch(
                workspace_id,
                root.to_path_buf(),
                git_dirs(repositories.iter().map(|git| &git.repository)),
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

    /// Every Pane in a Workspace nudges the same scheduler. The probe never does Git I/O.
    pub(super) fn request_terminal_git_refresh(&self, pane_id: PaneId, activity: Instant) {
        if let Some(workspace) = self.session.workspace_for_pane(pane_id)
            && let Some(watcher) = &self.git_watcher
        {
            watcher.terminal_activity(workspace.id(), activity);
        }
    }
}

/// Events within this window fold into one recomputation: a save, a checkout or a build
/// touches many files at once.
const GIT_WATCH_DEBOUNCE: Duration = Duration::from_millis(300);
const GIT_SCAN_INTERVAL: Duration = Duration::from_secs(2);

enum WatchMessage {
    Watch {
        workspace_id: WorkspaceId,
        root: PathBuf,
        git_dirs: Vec<(RelativePathBuf, PathBuf)>,
    },
    Retain(Vec<WorkspaceId>),
    Refresh(WorkspaceId),
    TerminalActivity(WorkspaceId),
    Filesystem(notify::Result<notify::Event>),
    Stop,
}

/// Watches every Workspace root and recomputes its Git state when files change, off the
/// state lock. `.git/objects` churn and paths the repository ignores do not count.
pub(super) struct GitWatcher {
    messages: mpsc::Sender<WatchMessage>,
    /// At most one queued terminal wakeup per Workspace, even while a scan is slow.
    terminal_activity: Arc<Mutex<HashMap<WorkspaceId, Instant>>>,
    stopping: Arc<AtomicBool>,
    worker: Option<thread::JoinHandle<()>>,
}

struct Watched {
    root: PathBuf,
    /// `root` with links resolved: FSEvents reports resolved paths (macOS's `/var` is
    /// `/private/var`), inotify and ReadDirectoryChangesW the watched spelling.
    real_root: PathBuf,
    root_subscribed: bool,
    /// Each repository's git directory, by its directory under the root.
    git_dirs: Vec<(RelativePathBuf, PathBuf)>,
    /// The git directories outside the root, which have watches of their own.
    git_dirs_subscribed: Vec<PathBuf>,
    /// Paths changed since the last filesystem scan, relative to the root.
    pending: Vec<PathBuf>,
    filesystem_due: Option<Instant>,
    terminal_due: Option<Instant>,
    scanned_at: Option<Instant>,
    /// Each repository's fingerprint at the last scan, by its directory.
    fingerprints: HashMap<RelativePathBuf, Option<GitFingerprint>>,
    forced: bool,
    /// The last window dropped paths past `MAX_PENDING_PATHS`.
    overflow: bool,
    /// Overflowing, all-ignored windows were skipped: a build is writing. The first quiet
    /// window after them is scanned in full, in case a real change hid among the drops.
    storm: bool,
}

impl GitWatcher {
    pub(super) fn start(state: Weak<Mutex<RuntimeState>>) -> Self {
        let (messages, inbox) = mpsc::channel::<WatchMessage>();
        let filesystem = messages.clone();
        let watcher = notify::recommended_watcher(move |event| {
            let _ = filesystem.send(WatchMessage::Filesystem(event));
        });
        Self::spawn(state, messages, inbox, watcher)
    }

    #[cfg(test)]
    pub(super) fn without_filesystem(state: Weak<Mutex<RuntimeState>>) -> Self {
        let (messages, inbox) = mpsc::channel();
        Self::spawn(
            state,
            messages,
            inbox,
            Err(notify::Error::generic("test: no notify backend")),
        )
    }

    fn spawn(
        state: Weak<Mutex<RuntimeState>>,
        messages: mpsc::Sender<WatchMessage>,
        inbox: mpsc::Receiver<WatchMessage>,
        watcher: notify::Result<notify::RecommendedWatcher>,
    ) -> Self {
        let terminal_activity = Arc::new(Mutex::new(HashMap::new()));
        let stopping = Arc::new(AtomicBool::new(false));
        let worker = {
            let terminal_activity = Arc::clone(&terminal_activity);
            let stopping = Arc::clone(&stopping);
            thread::spawn(move || run_watcher(watcher, inbox, state, terminal_activity, stopping))
        };
        Self {
            messages,
            terminal_activity,
            stopping,
            worker: Some(worker),
        }
    }

    pub(super) fn watch(
        &self,
        workspace_id: WorkspaceId,
        root: PathBuf,
        git_dirs: Vec<(RelativePathBuf, PathBuf)>,
    ) {
        let _ = self.messages.send(WatchMessage::Watch {
            workspace_id,
            root,
            git_dirs,
        });
    }

    pub(super) fn refresh(&self, workspace_id: WorkspaceId) {
        let _ = self.messages.send(WatchMessage::Refresh(workspace_id));
    }

    fn terminal_activity(&self, workspace_id: WorkspaceId, activity: Instant) {
        let mut pending = self.terminal_activity.lock().or_abort();
        // Keep the latest timestamp without adding another wakeup to the inbox.
        match pending.entry(workspace_id) {
            std::collections::hash_map::Entry::Occupied(mut entry) => {
                *entry.get_mut() = (*entry.get()).max(activity);
            }
            std::collections::hash_map::Entry::Vacant(entry) => {
                entry.insert(activity);
                let _ = self
                    .messages
                    .send(WatchMessage::TerminalActivity(workspace_id));
            }
        }
    }

    /// Keeps only the listed Workspaces under watch.
    fn retain(&self, keep: Vec<WorkspaceId>) {
        let _ = self.messages.send(WatchMessage::Retain(keep));
    }

    /// The Server takes this out of RuntimeState and joins without holding its lock.
    pub(super) fn shutdown(mut self) {
        self.stop();
        if let Some(worker) = self.worker.take()
            && worker.join().is_err()
        {
            tracing::warn!("Git watcher thread panicked during shutdown");
        }
    }

    fn stop(&self) {
        // Stop wins over an existing filesystem backlog; the message also wakes idle recv.
        self.stopping.store(true, Ordering::Release);
        let _ = self.messages.send(WatchMessage::Stop);
    }
}

impl Drop for GitWatcher {
    fn drop(&mut self) {
        // A worker's temporary upgraded Weak may own the last RuntimeState. Never join
        // here: dropping that state on the worker itself must only request its exit.
        self.stop();
    }
}

impl Watched {
    fn new(root: PathBuf) -> Self {
        Self {
            real_root: resolved(&root),
            root,
            root_subscribed: false,
            git_dirs: Vec::new(),
            git_dirs_subscribed: Vec::new(),
            pending: Vec::new(),
            filesystem_due: None,
            terminal_due: None,
            scanned_at: None,
            fingerprints: HashMap::new(),
            forced: false,
            overflow: false,
            storm: false,
        }
    }

    fn terminal_activity(&mut self, activity: Instant) {
        if self.scanned_at.is_some_and(|scanned| scanned >= activity) {
            return;
        }
        let due = self.scanned_at.map_or(activity, |scanned| {
            activity.max(scanned + GIT_SCAN_INTERVAL)
        });
        // Further output must not push a waiting Workspace's scan back indefinitely.
        self.terminal_due = Some(self.terminal_due.map_or(due, |pending| pending.min(due)));
    }

    fn next_due(&self) -> Option<Instant> {
        self.filesystem_due
            .into_iter()
            .chain(self.terminal_due)
            .min()
    }

    fn record_scan(
        &mut self,
        now: Instant,
        fingerprints: HashMap<RelativePathBuf, Option<GitFingerprint>>,
    ) {
        self.scanned_at = Some(now);
        self.fingerprints = fingerprints;
        self.terminal_due = None;
    }

    fn unwatch(&mut self, watcher: &mut notify::RecommendedWatcher) {
        if self.root_subscribed {
            let _ = watcher.unwatch(&self.root);
        }
        for git_dir in self.git_dirs_subscribed.drain(..) {
            let _ = watcher.unwatch(&git_dir);
        }
    }
}

fn run_watcher(
    watcher: notify::Result<notify::RecommendedWatcher>,
    inbox: mpsc::Receiver<WatchMessage>,
    state: Weak<Mutex<RuntimeState>>,
    terminal_activity: Arc<Mutex<HashMap<WorkspaceId, Instant>>>,
    stopping: Arc<AtomicBool>,
) {
    let mut watcher = match watcher {
        Ok(watcher) => Some(watcher),
        Err(error) => {
            tracing::warn!(
                "file watching is unavailable, Git changes refresh on terminal activity only: {error}"
            );
            None
        }
    };
    let mut watched: HashMap<WorkspaceId, Watched> = HashMap::new();
    loop {
        if stopping.load(Ordering::Acquire) {
            return;
        }
        let next_due = watched.values().filter_map(Watched::next_due).min();
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
                git_dirs,
            }) => {
                let entry = watched
                    .entry(workspace_id)
                    .or_insert_with(|| Watched::new(root.clone()));
                if entry.root != root {
                    if let Some(watcher) = &mut watcher {
                        entry.unwatch(watcher);
                    }
                    *entry = Watched::new(root);
                }
                if !entry.root_subscribed
                    && let Some(watcher) = &mut watcher
                {
                    entry.root_subscribed =
                        subscribe(watcher, &entry.root, notify::RecursiveMode::Recursive);
                }
                watch_git_dirs(entry, watcher.as_mut(), git_dirs);
            }
            Some(WatchMessage::Retain(keep)) => {
                let gone: Vec<WorkspaceId> = watched
                    .keys()
                    .copied()
                    .filter(|id| !keep.contains(id))
                    .collect();
                for workspace_id in gone {
                    if let Some(mut entry) = watched.remove(&workspace_id)
                        && let Some(watcher) = &mut watcher
                    {
                        entry.unwatch(watcher);
                    }
                }
            }
            Some(WatchMessage::Refresh(workspace_id)) => {
                if let Some(entry) = watched.get_mut(&workspace_id) {
                    entry.forced = true;
                    entry.filesystem_due = Some(Instant::now());
                }
            }
            Some(WatchMessage::TerminalActivity(workspace_id)) => {
                let activity = terminal_activity.lock().or_abort().remove(&workspace_id);
                if let Some(activity) = activity
                    && let Some(entry) = watched.get_mut(&workspace_id)
                {
                    entry.terminal_activity(activity);
                }
            }
            Some(WatchMessage::Filesystem(Err(error))) => {
                tracing::warn!("file watcher: {error}");
            }
            Some(WatchMessage::Filesystem(Ok(event))) => {
                let now = Instant::now();
                for path in &event.paths {
                    for entry in watched.values_mut() {
                        if let Some(relative) = relevant_change(entry, path) {
                            if entry.pending.len() < MAX_PENDING_PATHS {
                                entry.pending.push(relative);
                            } else {
                                entry.overflow = true;
                            }
                            if !entry.forced {
                                entry.filesystem_due = Some(now + GIT_WATCH_DEBOUNCE);
                            }
                        }
                    }
                }
            }
            Some(WatchMessage::Stop) => return,
            None => {}
        }

        let now = Instant::now();
        let mut due: Vec<_> = watched
            .iter()
            .filter_map(|(id, entry)| entry.next_due().map(|due| (*id, due)))
            .filter(|(_, due)| *due <= now)
            .collect();
        // A busy Workspace cannot repeatedly jump ahead of another one's overdue scan.
        due.sort_by_key(|(_, due)| *due);
        for (workspace_id, _) in due {
            if stopping.load(Ordering::Acquire) {
                return;
            }
            let entry = watched.get_mut(&workspace_id).expect("watched Workspace");
            if !refresh_isolated(workspace_id, entry, watcher.as_mut(), &state) {
                return;
            }
        }
    }
}

/// One Workspace's scan, whose panic loses that scan and not every later one (ADR 0053):
/// its pending work is dropped, and the next change schedules another.
fn refresh_isolated(
    workspace_id: WorkspaceId,
    entry: &mut Watched,
    watcher: Option<&mut notify::RecommendedWatcher>,
    state: &Weak<Mutex<RuntimeState>>,
) -> bool {
    panic::catch_unwind(AssertUnwindSafe(|| {
        refresh_workspace(workspace_id, entry, watcher, state)
    }))
    .unwrap_or_else(|_| {
        tracing::error!("a Git scan panicked; waiting for the next change");
        entry.pending.clear();
        entry.filesystem_due = None;
        entry.forced = false;
        entry.overflow = false;
        entry.record_scan(Instant::now(), HashMap::new());
        true
    })
}

/// The only background scan/commit path. Both sources run serially here, so an older
/// terminal scan can never publish after a newer filesystem scan. Only the repositories a
/// changed path, a moved ref or a forced refresh touched walk their status (ADR 0073).
fn refresh_workspace(
    workspace_id: WorkspaceId,
    entry: &mut Watched,
    watcher: Option<&mut notify::RecommendedWatcher>,
    state: &Weak<Mutex<RuntimeState>>,
) -> bool {
    #[cfg(test)]
    if let Some(shared) = state.upgrade() {
        RuntimeState::panic_if_injected(&shared, "git-scan", workspace_id.as_u64());
    }
    let now = Instant::now();
    let filesystem_due = entry.filesystem_due.is_some_and(|due| due <= now);
    let terminal_due = entry.terminal_due.is_some_and(|due| due <= now);
    let (pending, forced, overflow) = if filesystem_due {
        entry.filesystem_due = None;
        (
            std::mem::take(&mut entry.pending),
            std::mem::take(&mut entry.forced),
            std::mem::take(&mut entry.overflow),
        )
    } else {
        (Vec::new(), false, false)
    };
    let (known, recorded) = {
        let Some(shared) = state.upgrade() else {
            return false;
        };
        let state = shared.lock().or_abort();
        if state
            .session
            .workspace(workspace_id)
            .is_none_or(|workspace| workspace.root_directory() != entry.root)
        {
            // Retain/Watch is already queued; do not keep an obsolete deadline hot.
            entry.terminal_due = None;
            return true;
        }
        let known = state
            .workspace_git
            .get(&workspace_id)
            .cloned()
            .unwrap_or_default();
        (known, state.recorded_base(workspace_id))
    };
    // Terminal output alone does not justify a status walk when no ref file moved; with no
    // repository known it rediscovers, as one may have appeared. Record the fingerprints
    // before scanning: a switch racing the scan differs next time.
    let fingerprints: HashMap<RelativePathBuf, Option<GitFingerprint>> = known
        .iter()
        .map(|git| {
            let base = git.base.as_ref().map(|(base, _)| base);
            (
                git.repository.directory().to_relative_path_buf(),
                git.repository.fingerprint(base),
            )
        })
        .collect();
    let moved: Vec<RelativePathBuf> = fingerprints
        .iter()
        .filter(|(directory, fingerprint)| {
            fingerprint.is_none() || entry.fingerprints.get(*directory) != Some(*fingerprint)
        })
        .map(|(directory, _)| directory.clone())
        .collect();
    let terminal_changed = terminal_due && (known.is_empty() || !moved.is_empty());
    if terminal_due {
        entry.record_scan(now, fingerprints.clone());
    }
    if !filesystem_due && !terminal_changed {
        return true;
    }
    // Index/ref-only updates do not invalidate Files/Preview; terminal activity never did.
    let mut working_tree_changed = forced || pending.iter().any(|path| !is_git_marker(path));
    // The known repositories a changed path that they do not ignore lies in. A path in none
    // changes Files only.
    let holds = |git: &RepositoryGit, path: &PathBuf| {
        RelativePathBuf::from_path(path)
            .is_ok_and(|path| git.repository.to_repository(&path).is_some())
    };
    let mut touched = Vec::new();
    let mut unheld = false;
    for git in &known {
        let held: Vec<&PathBuf> = pending.iter().filter(|path| holds(git, path)).collect();
        if !held.is_empty() && !git.repository.ignores_all(held) {
            touched.push(git.repository.directory());
        }
    }
    for path in &pending {
        unheld |= !known.iter().any(|git| holds(git, path));
    }
    let storm = entry.storm;
    let ignored = filesystem_due
        && !forced
        && (overflow || !storm)
        && !pending.is_empty()
        && !unheld
        && touched.is_empty();
    if ignored {
        entry.storm = overflow;
        if !terminal_changed {
            return true;
        }
        working_tree_changed = false;
    }
    entry.storm = false;
    // Paths dropped past the cap, or hidden in a storm, may lie in any repository.
    let everything = forced || (filesystem_due && !ignored && (overflow || storm));
    let discovered = match workspace_repositories(&entry.root) {
        Ok(discovered) => discovered,
        Err(error) => {
            entry.record_scan(now, fingerprints);
            tracing::warn!("Git scan of {} failed: {error}", entry.root.display());
            return true;
        }
    };
    watch_git_dirs(entry, watcher, git_dirs(&discovered));
    entry.record_scan(now, fingerprints);
    let next: Vec<RepositoryGit> = discovered
        .into_iter()
        .map(|repository| {
            let directory = repository.directory();
            let previous = known
                .iter()
                .find(|git| git.repository.directory() == directory);
            let walk = everything
                || previous.is_none_or(|previous| previous.repository != repository)
                || (!ignored && touched.contains(&directory))
                || (terminal_changed && moved.iter().any(|moved| moved == directory));
            match previous {
                Some(previous) if !walk => previous.clone(),
                _ => RepositoryGit::from_repository(repository.clone(), recorded.as_deref())
                    .unwrap_or_else(|error| {
                        tracing::warn!(
                            "Git scan of {} failed: {error}",
                            repository.root().display()
                        );
                        previous
                            .cloned()
                            .unwrap_or_else(|| RepositoryGit::discovered(repository))
                    }),
            }
        })
        .collect();
    let Some(shared) = state.upgrade() else {
        return false;
    };
    let mut state = shared.lock().or_abort();
    if apply_workspace_git_refresh(&mut state, workspace_id, &entry.root, next)
        && working_tree_changed
    {
        // The same batch is what the Files sidebar and Preview Panes follow (ADR 0018).
        state.publish_background(SessionEvent::WorkspaceFilesChanged { workspace_id });
    }
    true
}

/// Past this many paths in one window the rest are dropped and the kept ones stand in for
/// the batch: a build writing thousands of ignored files must not turn into a status walk
/// per window. A real change hidden behind them is picked up by the next event or by the
/// terminal-activity scan.
const MAX_PENDING_PATHS: usize = 256;

fn subscribe(
    watcher: &mut notify::RecommendedWatcher,
    path: &Path,
    mode: notify::RecursiveMode,
) -> bool {
    match watcher.watch(path, mode) {
        Ok(()) => true,
        Err(error) => {
            tracing::warn!("cannot watch {} for Git changes: {error}", path.display());
            false
        }
    }
}

/// A linked worktree, and the repository a Root below its top is in, keep HEAD and index
/// outside the root, so those directories get their own watches; a git directory under the
/// root is under the recursive root watch already.
fn watch_git_dirs(
    entry: &mut Watched,
    watcher: Option<&mut notify::RecommendedWatcher>,
    git_dirs: Vec<(RelativePathBuf, PathBuf)>,
) {
    if let Some(watcher) = watcher {
        let outside: Vec<&PathBuf> = git_dirs
            .iter()
            .map(|(_, git_dir)| git_dir)
            .filter(|git_dir| {
                !git_dir.starts_with(&entry.root) && !git_dir.starts_with(&entry.real_root)
            })
            .collect();
        entry.git_dirs_subscribed.retain(|subscribed| {
            let keep = outside.contains(&subscribed);
            if !keep {
                let _ = watcher.unwatch(subscribed);
            }
            keep
        });
        for git_dir in outside {
            if !entry.git_dirs_subscribed.contains(git_dir)
                && subscribe(watcher, git_dir, notify::RecursiveMode::NonRecursive)
            {
                entry.git_dirs_subscribed.push(git_dir.clone());
            }
        }
    }
    entry.git_dirs = git_dirs;
}

fn resolved(path: &Path) -> PathBuf {
    std::fs::canonicalize(path).unwrap_or_else(|_| path.to_path_buf())
}

/// The path relative to the Workspace root when `path` can change its Git status; the
/// object store never does, and a change under a git directory counts as its repository's
/// `.git` itself.
fn relevant_change(entry: &Watched, path: &Path) -> Option<PathBuf> {
    // The innermost one: a linked worktree's git directory lies inside its main one's.
    if let Some((directory, inside)) = entry
        .git_dirs
        .iter()
        .filter_map(|(directory, git_dir)| Some((directory, path.strip_prefix(git_dir).ok()?)))
        .min_by_key(|(_, inside)| inside.components().count())
    {
        return (!inside.starts_with("objects") && !inside.starts_with("lfs"))
            .then(|| directory.to_path("").join(".git"));
    }
    let relative = path
        .strip_prefix(&entry.root)
        .or_else(|_| path.strip_prefix(&entry.real_root))
        .ok()?;
    // An upload's temporary file changes nothing until it is placed (ADR 0061).
    if relative
        .file_name()
        .and_then(|name| name.to_str())
        .is_some_and(condr_core::is_transfer_temp)
    {
        return None;
    }
    // A `.git` at any depth: the Root's own, a repository's under it, or a nested one.
    let mut components = relative.components();
    let mut marker = PathBuf::new();
    while let Some(component) = components.next() {
        marker.push(component);
        if component.as_os_str() == ".git" {
            let inside = components.as_path();
            return (!inside.starts_with("objects") && !inside.starts_with("lfs"))
                .then_some(marker);
        }
    }
    Some(relative.to_path_buf())
}

/// A path [`relevant_change`] gave for a change inside a git directory.
fn is_git_marker(path: &Path) -> bool {
    path.file_name().is_some_and(|name| name == ".git")
}
