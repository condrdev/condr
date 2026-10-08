//! What the Server knows about a Workspace's repository (ADR 0017): the discovered
//! repository plus its working-tree changes, recomputed as one unit so branch, upstream and
//! file list never disagree, and the filesystem watcher that asks for those recomputations.

use super::*;
use condr_core::protocol::GitBaseChanges;
use condr_core::{GitBase, GitChanges, GitError, GitFingerprint};
use notify::Watcher as _;
use std::collections::HashMap;
use std::path::Path;
use std::sync::Weak;

#[cfg(test)]
mod tests;

/// One Workspace's Git state. Equality decides whether a refresh is worth an event.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(super) struct WorkspaceGit {
    pub(super) repository: GitRepository,
    pub(super) changes: GitChanges,
    /// The base and the working tree against it (ADR 0034).
    pub(super) base: Option<(GitBase, GitChanges)>,
}

impl WorkspaceGit {
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
        // A base that cannot be resolved costs the base view, never the HEAD one.
        let base = repository.base(recorded).unwrap_or_else(|error| {
            tracing::warn!(
                "cannot resolve the base of {}: {error}",
                repository.root().display()
            );
            None
        });
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
        base: git.base.as_ref().map(|(base, changes)| GitBaseChanges {
            branch: base.name().to_owned(),
            changes: changes.clone(),
        }),
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
        git_dir: Option<PathBuf>,
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
    git_dir: Option<PathBuf>,
    /// A git directory outside the root that has its own watch.
    git_dir_subscribed: Option<PathBuf>,
    /// Paths changed since the last filesystem scan, relative to the root.
    pending: Vec<PathBuf>,
    filesystem_due: Option<Instant>,
    terminal_due: Option<Instant>,
    scanned_at: Option<Instant>,
    fingerprint: Option<GitFingerprint>,
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

    fn terminal_activity(&self, workspace_id: WorkspaceId, activity: Instant) {
        let mut pending = self
            .terminal_activity
            .lock()
            .expect("Git activity lock poisoned");
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
            git_dir: None,
            git_dir_subscribed: None,
            pending: Vec::new(),
            filesystem_due: None,
            terminal_due: None,
            scanned_at: None,
            fingerprint: None,
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

    fn record_scan(&mut self, now: Instant, fingerprint: Option<GitFingerprint>) {
        self.scanned_at = Some(now);
        self.fingerprint = fingerprint;
        self.terminal_due = None;
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
                git_dir,
            }) => {
                let entry = watched
                    .entry(workspace_id)
                    .or_insert_with(|| Watched::new(root.clone()));
                if entry.root != root {
                    if let Some(watcher) = &mut watcher
                        && entry.root_subscribed
                    {
                        let _ = watcher.unwatch(&entry.root);
                    }
                    if let Some(watcher) = &mut watcher
                        && let Some(git_dir) = &entry.git_dir_subscribed
                    {
                        let _ = watcher.unwatch(git_dir);
                    }
                    *entry = Watched::new(root);
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
                    entry.filesystem_due = Some(Instant::now());
                }
            }
            Some(WatchMessage::TerminalActivity(workspace_id)) => {
                let activity = terminal_activity
                    .lock()
                    .expect("Git activity lock poisoned")
                    .remove(&workspace_id);
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
            if !refresh_workspace(workspace_id, entry, watcher.as_mut(), &state) {
                return;
            }
        }
    }
}

/// The only background scan/commit path. Both sources run serially here, so an older
/// terminal scan can never publish after a newer filesystem scan.
fn refresh_workspace(
    workspace_id: WorkspaceId,
    entry: &mut Watched,
    watcher: Option<&mut notify::RecommendedWatcher>,
    state: &Weak<Mutex<RuntimeState>>,
) -> bool {
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
        let state = shared.lock().expect("server state lock poisoned");
        if state
            .session
            .workspace(workspace_id)
            .is_none_or(|workspace| workspace.root_directory() != entry.root)
        {
            // Retain/Watch is already queued; do not keep an obsolete deadline hot.
            entry.terminal_due = None;
            return true;
        }
        let known = state.workspace_git.get(&workspace_id).map(|git| {
            (
                git.repository.clone(),
                git.base.as_ref().map(|(base, _)| base.clone()),
            )
        });
        (known, state.recorded_base(workspace_id))
    };
    // Terminal output alone does not justify a status walk when ref files did not move.
    // Record the fingerprint before scanning: a switch racing the scan differs next time.
    let fingerprint = known
        .as_ref()
        .and_then(|(repository, base)| repository.fingerprint(base.as_ref()));
    let terminal_changed =
        terminal_due && (fingerprint.is_none() || fingerprint != entry.fingerprint);
    if terminal_due {
        entry.record_scan(now, fingerprint.clone());
    }
    if !filesystem_due && !terminal_changed {
        return true;
    }
    // Index/ref-only updates do not invalidate Files/Preview; terminal activity never did.
    let mut working_tree_changed = forced || pending.iter().any(|path| path != Path::new(".git"));
    let next = match discover_repository(&entry.root) {
        Ok(None) => {
            entry.record_scan(now, fingerprint);
            Ok(None)
        }
        Ok(Some(repository)) => {
            if filesystem_due
                && !forced
                && (overflow || !entry.storm)
                && repository.ignores_all(&pending)
            {
                entry.storm = overflow;
                if !terminal_changed {
                    return true;
                }
                working_tree_changed = false;
            }
            entry.storm = false;
            watch_git_dir(entry, watcher, repository.git_directory().to_path_buf());
            entry.record_scan(now, fingerprint);
            WorkspaceGit::from_repository(repository, recorded.as_deref()).map(Some)
        }
        Err(error) => {
            entry.record_scan(now, fingerprint);
            Err(error)
        }
    };
    let next = match next {
        Ok(next) => next,
        Err(error) => {
            tracing::warn!("Git scan of {} failed: {error}", entry.root.display());
            return true;
        }
    };
    let Some(shared) = state.upgrade() else {
        return false;
    };
    let mut state = shared.lock().expect("server state lock poisoned");
    if apply_workspace_git_refresh(&mut state, workspace_id, &entry.root, next)
        && working_tree_changed
    {
        // The same batch is what the Files sidebar and Preview Tabs follow (ADR 0018).
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

fn resolved(path: &Path) -> PathBuf {
    std::fs::canonicalize(path).unwrap_or_else(|_| path.to_path_buf())
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
    let relative = path
        .strip_prefix(&entry.root)
        .or_else(|_| path.strip_prefix(&entry.real_root))
        .ok()?;
    if relative.starts_with(".git") {
        let inside = relative.strip_prefix(".git").unwrap_or(relative);
        return (!inside.starts_with("objects") && !inside.starts_with("lfs"))
            .then(|| PathBuf::from(".git"));
    }
    Some(relative.to_path_buf())
}
