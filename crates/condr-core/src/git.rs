use std::ffi::OsString;
use std::fmt;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};
use std::time::SystemTime;

/// Modification times of the HEAD files a branch switch rewrites. Equal fingerprints mean a
/// rediscovery would return the same branch, so the caller can skip spawning `git`.
pub type GitHeadFingerprint = (SystemTime, SystemTime);

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct GitRepository {
    root: PathBuf,
    git_directory: PathBuf,
    common_directory: PathBuf,
    branch: Option<String>,
}

impl GitRepository {
    pub fn root(&self) -> &Path {
        &self.root
    }

    pub fn branch(&self) -> Option<&str> {
        self.branch.as_deref()
    }

    pub fn is_linked_worktree(&self) -> bool {
        self.git_directory != self.common_directory
    }

    pub fn head_fingerprint(&self) -> Option<GitHeadFingerprint> {
        let modified = |directory: &Path| fs::metadata(directory.join("HEAD"))?.modified();
        Some((
            modified(&self.git_directory).ok()?,
            modified(&self.common_directory).ok()?,
        ))
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct GitError(String);

impl fmt::Display for GitError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.0)
    }
}

impl std::error::Error for GitError {}

pub fn discover_repository(path: impl AsRef<Path>) -> Result<Option<GitRepository>, GitError> {
    let path = path.as_ref();
    // One `rev-parse` answers everything but the branch; it fails outside a work tree, which is
    // the "not a repository" case rather than an error.
    let facts = run_git(
        path,
        [
            "rev-parse",
            "--is-inside-work-tree",
            "--path-format=absolute",
            "--show-toplevel",
            "--absolute-git-dir",
            "--git-common-dir",
        ],
    )?;
    if !facts.status.success() {
        return Ok(None);
    }
    let mut lines = facts.stdout.split(|byte| *byte == b'\n');
    if lines.next().map(<[u8]>::trim_ascii) != Some(b"true") {
        return Ok(None);
    }
    let mut path_line = || {
        let bytes = lines.next().map(<[u8]>::trim_ascii).unwrap_or_default();
        if bytes.is_empty() {
            return Err(GitError("Git returned an empty path".into()));
        }
        Ok(PathBuf::from(bytes_to_os_string(bytes)?))
    };
    let root = path_line()?;
    let git_directory = path_line()?;
    let common_directory = path_line()?;
    let branch = head_branch(path, &git_directory, &common_directory)?;

    Ok(Some(GitRepository {
        root,
        git_directory,
        common_directory,
        branch,
    }))
}

/// The checked-out branch, read from the `HEAD` file like herdr does so the common path spawns
/// no process: `ref: refs/heads/<name>` names it (an unborn branch too), anything else is a
/// detached HEAD. Reftable repositories keep HEAD elsewhere, so they ask `git`.
fn head_branch(
    cwd: &Path,
    git_directory: &Path,
    common_directory: &Path,
) -> Result<Option<String>, GitError> {
    if uses_reftable(common_directory) {
        let head = run_git(cwd, ["symbolic-ref", "--short", "-q", "HEAD"])?;
        return Ok(head
            .status
            .success()
            .then(|| text(&head).trim().to_owned())
            .filter(|branch| !branch.is_empty()));
    }
    let head = fs::read_to_string(git_directory.join("HEAD"))
        .map_err(|error| GitError(format!("cannot read HEAD: {error}")))?;
    Ok(head
        .trim()
        .strip_prefix("ref: refs/heads/")
        .filter(|branch| !branch.is_empty())
        .map(str::to_owned))
}

fn uses_reftable(common_directory: &Path) -> bool {
    fs::read_to_string(common_directory.join("config")).is_ok_and(|config| {
        config.lines().any(|line| {
            let mut parts = line.splitn(2, '=');
            parts.next().is_some_and(|key| key.trim() == "refstorage")
                && parts
                    .next()
                    .is_some_and(|value| value.trim().eq_ignore_ascii_case("reftable"))
        })
    })
}

pub fn default_worktree_root() -> Option<PathBuf> {
    crate::data_directory().map(|root| root.join("worktrees"))
}

pub fn create_worktree(
    parent: &GitRepository,
    branch: &str,
    worktree_root: impl AsRef<Path>,
) -> Result<GitRepository, GitError> {
    if parent.is_linked_worktree() {
        return Err(GitError(
            "create a worktree from the main repository Workspace".into(),
        ));
    }
    let branch = branch.trim();
    if branch.is_empty() {
        return Err(GitError("branch name cannot be empty".into()));
    }
    checked(&parent.root, ["check-ref-format", "--branch", branch])?;

    let repo_name = parent
        .root
        .file_name()
        .and_then(|name| name.to_str())
        .filter(|name| !name.is_empty())
        .unwrap_or("repository");
    let destination = worktree_root
        .as_ref()
        .join(repo_name)
        .join(branch_slug(branch));
    if destination.exists() {
        return Err(GitError(format!(
            "worktree destination already exists: {}",
            destination.display()
        )));
    }
    if let Some(parent_directory) = destination.parent() {
        fs::create_dir_all(parent_directory)
            .map_err(|error| GitError(format!("failed to create worktree directory: {error}")))?;
    }

    let reference = format!("refs/heads/{branch}");
    let branch_exists = run_git(
        &parent.root,
        ["show-ref", "--verify", "--quiet", reference.as_str()],
    )?
    .status
    .success();
    let mut command = git_command(&parent.root);
    command.arg("worktree").arg("add");
    if branch_exists {
        command.arg(&destination).arg(branch);
    } else {
        command.arg("-b").arg(branch).arg(&destination).arg("HEAD");
    }
    checked_output(command.output().map_err(command_error)?)?;

    let child = discover_repository(&destination)?
        .ok_or_else(|| GitError("created worktree is not a Git checkout".into()))?;
    ensure_same_repository(parent, &child)?;
    Ok(child)
}

pub fn open_worktree(
    parent: &GitRepository,
    path: impl AsRef<Path>,
) -> Result<GitRepository, GitError> {
    if parent.is_linked_worktree() {
        return Err(GitError(
            "open a worktree from the main repository Workspace".into(),
        ));
    }
    let child = discover_repository(path)?
        .ok_or_else(|| GitError("selected directory is not a Git worktree".into()))?;
    ensure_same_repository(parent, &child)?;
    if !child.is_linked_worktree() {
        return Err(GitError(
            "selected directory is not a linked Git worktree".into(),
        ));
    }
    Ok(child)
}

pub fn remove_worktree(parent: &GitRepository, child: &GitRepository) -> Result<(), GitError> {
    validate_worktree_removal(parent, child)?;
    let mut command = git_command(&parent.root);
    command.arg("worktree").arg("remove").arg(&child.root);
    checked_output(command.output().map_err(command_error)?)?;
    Ok(())
}

pub fn validate_worktree_removal(
    parent: &GitRepository,
    child: &GitRepository,
) -> Result<(), GitError> {
    ensure_same_repository(parent, child)?;
    if !child.is_linked_worktree() {
        return Err(GitError("only a linked Git worktree can be removed".into()));
    }
    let status = checked(
        &child.root,
        ["status", "--porcelain", "--untracked-files=all"],
    )?;
    if !status.stdout.is_empty() {
        return Err(GitError(
            "worktree has modified or untracked files; clean it before removal".into(),
        ));
    }
    Ok(())
}

fn ensure_same_repository(parent: &GitRepository, child: &GitRepository) -> Result<(), GitError> {
    if parent.common_directory == child.common_directory {
        Ok(())
    } else {
        Err(GitError(
            "selected worktree belongs to a different Git repository".into(),
        ))
    }
}

fn branch_slug(branch: &str) -> String {
    let mut slug = String::new();
    for character in branch.chars() {
        if character.is_ascii_alphanumeric() || matches!(character, '.' | '_' | '-') {
            slug.push(character);
        } else if !slug.ends_with('-') {
            slug.push('-');
        }
    }
    slug.trim_matches('-').to_owned()
}

fn git_command(cwd: &Path) -> Command {
    let mut command = Command::new("git");
    command.arg("-C").arg(cwd);
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt as _;
        // 不带此标志时,每次 git 调用都会在 GUI 进程下闪出一个控制台窗口。
        const CREATE_NO_WINDOW: u32 = 0x0800_0000;
        command.creation_flags(CREATE_NO_WINDOW);
    }
    command
}

fn run_git<I, S>(cwd: &Path, args: I) -> Result<Output, GitError>
where
    I: IntoIterator<Item = S>,
    S: AsRef<std::ffi::OsStr>,
{
    git_command(cwd).args(args).output().map_err(command_error)
}

fn checked<I, S>(cwd: &Path, args: I) -> Result<Output, GitError>
where
    I: IntoIterator<Item = S>,
    S: AsRef<std::ffi::OsStr>,
{
    checked_output(run_git(cwd, args)?)
}

fn checked_output(output: Output) -> Result<Output, GitError> {
    if output.status.success() {
        Ok(output)
    } else {
        let message = String::from_utf8_lossy(&output.stderr).trim().to_owned();
        Err(GitError(if message.is_empty() {
            format!("Git command failed with {}", output.status)
        } else {
            message
        }))
    }
}

fn command_error(error: std::io::Error) -> GitError {
    GitError(format!("failed to run Git: {error}"))
}

fn text(output: &Output) -> String {
    String::from_utf8_lossy(&output.stdout).into_owned()
}

#[cfg(unix)]
fn bytes_to_os_string(bytes: &[u8]) -> Result<OsString, GitError> {
    use std::os::unix::ffi::OsStringExt as _;
    Ok(OsString::from_vec(bytes.to_vec()))
}

#[cfg(not(unix))]
fn bytes_to_os_string(bytes: &[u8]) -> Result<OsString, GitError> {
    String::from_utf8(bytes.to_vec())
        .map(OsString::from)
        .map_err(|_| GitError("Git returned a path that is not valid UTF-8".into()))
}
