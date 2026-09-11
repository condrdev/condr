//! Git through gitoxide: repository facts for the sidebar and the linked-worktree lifecycle.
//! Nothing here spawns `git` and nothing contacts a remote; the upstream numbers come from
//! the remote-tracking ref as the user's last fetch left it.

use std::fmt;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::AtomicBool;
use std::time::SystemTime;

use gix::bstr::ByteSlice as _;
use gix::refs::transaction::PreviousValue;
use serde::{Deserialize, Serialize};

/// Modification times of the files that move the branch, its head or its upstream. Equal
/// fingerprints mean a rediscovery would report the same branch and ahead/behind counts, so
/// the caller can skip opening the repository.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct GitFingerprint(Vec<Option<SystemTime>>);

/// How far the branch has diverged from its upstream, as of the last local fetch.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct GitUpstream {
    pub ahead: u32,
    pub behind: u32,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct GitRepository {
    root: PathBuf,
    git_directory: PathBuf,
    common_directory: PathBuf,
    linked_worktree: bool,
    branch: Option<String>,
    /// The upstream's full ref name (`refs/remotes/origin/main`), also the loose ref file a
    /// fetch or push rewrites under the common directory.
    upstream_ref: Option<PathBuf>,
    upstream: Option<GitUpstream>,
}

impl GitRepository {
    pub fn root(&self) -> &Path {
        &self.root
    }

    pub fn branch(&self) -> Option<&str> {
        self.branch.as_deref()
    }

    pub fn is_linked_worktree(&self) -> bool {
        self.linked_worktree
    }

    pub fn upstream(&self) -> Option<GitUpstream> {
        self.upstream
    }

    /// `None` only when a HEAD file is unreadable; the other files are optional and their
    /// appearance or disappearance is itself a change.
    pub fn fingerprint(&self) -> Option<GitFingerprint> {
        let modified = |path: PathBuf| fs::metadata(path).and_then(|meta| meta.modified()).ok();
        let mut times = vec![
            Some(modified(self.git_directory.join("HEAD"))?),
            Some(modified(self.common_directory.join("HEAD"))?),
            // Commits, resets and pulls move the head without rewriting HEAD: the reflog and
            // the branch's loose ref record them. Fetch and push rewrite the upstream's loose
            // ref; `pack-refs` folds either into packed-refs. Reftable keeps all of it in its
            // table lists.
            modified(self.git_directory.join("logs/HEAD")),
            modified(self.common_directory.join("packed-refs")),
            modified(self.common_directory.join("config")),
            modified(self.git_directory.join("reftable/tables.list")),
            modified(self.common_directory.join("reftable/tables.list")),
        ];
        if let Some(branch) = &self.branch {
            times.push(modified(
                self.common_directory.join("refs/heads").join(branch),
            ));
        }
        if let Some(upstream) = &self.upstream_ref {
            times.push(modified(self.common_directory.join(upstream)));
        }
        Some(GitFingerprint(times))
    }

    fn open(&self) -> Result<gix::Repository, GitError> {
        gix::open(&self.git_directory).map_err(git_error)
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

fn git_error(error: impl fmt::Display) -> GitError {
    GitError(error.to_string())
}

/// Absolute, symlink-free and without `..`, so two spellings of one directory compare equal.
fn real_path(path: &Path) -> Result<PathBuf, GitError> {
    gix::path::realpath(path).map_err(|error| GitError(format!("cannot resolve path: {error}")))
}

/// The repository whose work tree contains `path`; `None` outside any work tree, which
/// includes a path inside a `.git` directory.
pub fn discover_repository(path: impl AsRef<Path>) -> Result<Option<GitRepository>, GitError> {
    let path = real_path(path.as_ref())?;
    let repo = match gix::discover(&path) {
        Ok(repo) => repo,
        Err(gix::discover::Error::Discover(_)) => return Ok(None),
        Err(error) => return Err(git_error(error)),
    };
    let Some(root) = repo.workdir() else {
        return Ok(None);
    };
    // gix keeps paths as the `.git` and `commondir` files spell them; one spelling per
    // directory keeps `GitRepository` comparable and `..` out of the way.
    let root = real_path(root)?;
    let git_directory = real_path(repo.git_dir())?;
    let common_directory = real_path(repo.common_dir())?;
    if path.starts_with(&git_directory) {
        return Ok(None);
    }
    let linked_worktree = repo.kind() == gix::repository::Kind::LinkedWorkTree;

    // `ref: refs/heads/<name>` names the branch, an unborn one too; a detached HEAD has none.
    let head_name = repo.head_name().map_err(git_error)?;
    let branch = head_name
        .as_ref()
        .map(|name| name.shorten().to_str_lossy().into_owned());
    let (upstream_ref, upstream) = match &head_name {
        Some(name) => upstream_of(&repo, name.as_ref())?,
        None => (None, None),
    };

    Ok(Some(GitRepository {
        root,
        git_directory,
        common_directory,
        linked_worktree,
        branch,
        upstream_ref,
        upstream,
    }))
}

/// The branch's configured upstream and how far the two have diverged. A branch without an
/// upstream has neither; a `gone` upstream (configured, ref missing) names it but has no counts,
/// as does an unborn branch.
fn upstream_of(
    repo: &gix::Repository,
    branch: &gix::refs::FullNameRef,
) -> Result<(Option<PathBuf>, Option<GitUpstream>), GitError> {
    let Some(Ok(upstream_name)) =
        repo.branch_remote_tracking_ref_name(branch, gix::remote::Direction::Fetch)
    else {
        return Ok((None, None));
    };
    let upstream_ref = PathBuf::from(upstream_name.as_bstr().to_str_lossy().as_ref());
    let Some(upstream) = repo
        .try_find_reference(upstream_name.as_ref())
        .map_err(git_error)?
    else {
        return Ok((Some(upstream_ref), None));
    };
    let Ok(local) = repo.head_id() else {
        return Ok((Some(upstream_ref), None));
    };
    let local = local.detach();
    let upstream = upstream.into_fully_peeled_id().map_err(git_error)?.detach();
    let counts = GitUpstream {
        ahead: count_commits(repo, local, upstream)?,
        behind: count_commits(repo, upstream, local)?,
    };
    Ok((Some(upstream_ref), Some(counts)))
}

/// Commits reachable from `tip` but not from `hidden`: `git rev-list --count tip ^hidden`.
fn count_commits(
    repo: &gix::Repository,
    tip: gix::ObjectId,
    hidden: gix::ObjectId,
) -> Result<u32, GitError> {
    if tip == hidden {
        return Ok(0);
    }
    let mut count = 0;
    for commit in repo
        .rev_walk([tip])
        .with_hidden([hidden])
        .all()
        .map_err(git_error)?
    {
        commit.map_err(git_error)?;
        count += 1;
    }
    Ok(count)
}

pub fn default_worktree_root() -> Option<PathBuf> {
    crate::data_directory().map(|root| root.join("worktrees"))
}

/// `git worktree add`: registers the checkout under `<common>/worktrees/<id>`, creates the
/// branch from HEAD when it does not exist yet, and checks its tree out into the destination.
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
    let reference = branch_ref_name(branch)?;

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

    let repo = parent.open()?;
    ensure_branch_not_checked_out(&repo, reference.as_ref())?;
    if repo
        .try_find_reference(reference.as_ref())
        .map_err(git_error)?
        .is_none()
    {
        let head = repo
            .head_id()
            .map_err(|_| GitError("cannot create a worktree from an unborn branch".into()))?;
        repo.reference(
            reference.clone(),
            head.detach(),
            PreviousValue::MustNotExist,
            format!("worktree add: created branch {branch}"),
        )
        .map_err(git_error)?;
    }

    let admin_directory = register_worktree(&repo, &destination, reference.as_ref())?;
    if let Err(error) = checkout_worktree(&destination) {
        // Leave nothing half-made: git would report the failure and no worktree.
        let _ = fs::remove_dir_all(&destination);
        let _ = fs::remove_dir_all(&admin_directory);
        return Err(error);
    }

    let child = discover_repository(&destination)?
        .ok_or_else(|| GitError("created worktree is not a Git checkout".into()))?;
    ensure_same_repository(parent, &child)?;
    Ok(child)
}

/// The `refs/heads/` name for a user-typed branch, rejected like `check-ref-format --branch`.
fn branch_ref_name(branch: &str) -> Result<gix::refs::FullName, GitError> {
    if branch.starts_with('-') {
        return Err(GitError(format!("invalid branch name: {branch}")));
    }
    gix::refs::FullName::try_from(format!("refs/heads/{branch}"))
        .map_err(|_| GitError(format!("invalid branch name: {branch}")))
}

/// git refuses a second checkout of one branch; every worktree's HEAD is a symbolic ref file
/// that names it.
fn ensure_branch_not_checked_out(
    repo: &gix::Repository,
    reference: &gix::refs::FullNameRef,
) -> Result<(), GitError> {
    let checked_out =
        |head: Option<gix::refs::FullName>| head.is_some_and(|head| head.as_ref() == reference);
    if checked_out(repo.head_name().map_err(git_error)?) {
        return Err(GitError(format!(
            "branch {} is already checked out in the main worktree",
            reference.shorten()
        )));
    }
    for proxy in repo.worktrees().map_err(git_error)? {
        let worktree = proxy
            .into_repo_with_possibly_inaccessible_worktree()
            .map_err(git_error)?;
        if checked_out(worktree.head_name().map_err(git_error)?) {
            return Err(GitError(format!(
                "branch {} is already checked out in another worktree",
                reference.shorten()
            )));
        }
    }
    Ok(())
}

/// Writes the files that make `destination` a linked worktree of `repo` and returns its
/// private git directory. The layout is git's own: `HEAD`, `commondir` and `gitdir` beside
/// each other, and a `.git` file in the checkout pointing back.
fn register_worktree(
    repo: &gix::Repository,
    destination: &Path,
    reference: &gix::refs::FullNameRef,
) -> Result<PathBuf, GitError> {
    let io = |error: std::io::Error| GitError(format!("failed to register worktree: {error}"));
    let worktrees = repo.common_dir().join("worktrees");
    let base_id = destination
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or("worktree")
        .to_owned();
    let mut id = base_id.clone();
    let mut suffix = 1;
    while worktrees.join(&id).exists() {
        id = format!("{base_id}{suffix}");
        suffix += 1;
    }
    let admin_directory = worktrees.join(&id);
    fs::create_dir_all(&admin_directory).map_err(io)?;
    fs::create_dir_all(destination).map_err(io)?;
    let dot_git = destination.join(".git");
    fs::write(
        admin_directory.join("HEAD"),
        format!("ref: {}\n", reference.as_bstr()),
    )
    .map_err(io)?;
    fs::write(admin_directory.join("commondir"), "../..\n").map_err(io)?;
    fs::write(
        admin_directory.join("gitdir"),
        format!("{}\n", git_path(&dot_git)),
    )
    .map_err(io)?;
    fs::write(dot_git, format!("gitdir: {}\n", git_path(&admin_directory))).map_err(io)?;
    Ok(admin_directory)
}

/// git writes forward slashes on every platform and reads either.
fn git_path(path: &Path) -> String {
    path.to_string_lossy().replace('\\', "/")
}

/// Fills the registered, still empty worktree with HEAD's tree and writes its index.
fn checkout_worktree(destination: &Path) -> Result<(), GitError> {
    let repo = gix::open(destination).map_err(git_error)?;
    let workdir = repo
        .workdir()
        .ok_or_else(|| GitError("registered worktree has no work tree".into()))?;
    let tree = repo.head_tree_id().map_err(git_error)?;
    let mut index = repo.index_from_tree(&tree).map_err(git_error)?;
    let mut options = repo
        .checkout_options(gix::worktree::stack::state::attributes::Source::IdMapping)
        .map_err(git_error)?;
    options.destination_is_initially_empty = true;
    gix::worktree::state::checkout(
        &mut index,
        workdir,
        repo.objects.clone().into_arc().map_err(git_error)?,
        &gix::progress::Discard,
        &gix::progress::Discard,
        &AtomicBool::new(false),
        options,
    )
    .map_err(git_error)?;
    index.write(Default::default()).map_err(git_error)?;
    Ok(())
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

/// `git worktree remove`: the checkout and its registration go, the branch stays.
pub fn remove_worktree(parent: &GitRepository, child: &GitRepository) -> Result<(), GitError> {
    validate_worktree_removal(parent, child)?;
    let io = |error: std::io::Error| GitError(format!("failed to remove worktree: {error}"));
    fs::remove_dir_all(&child.root).map_err(io)?;
    fs::remove_dir_all(&child.git_directory).map_err(io)?;
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
    if child.git_directory.join("locked").exists() {
        return Err(GitError(
            "worktree is locked; unlock it before removal".into(),
        ));
    }
    if is_dirty(&child.open()?)? {
        return Err(GitError(
            "worktree has modified or untracked files; clean it before removal".into(),
        ));
    }
    Ok(())
}

/// `git status --porcelain --untracked-files=all` is non-empty: staged changes against HEAD,
/// or any modified or untracked path in the work tree.
fn is_dirty(repo: &gix::Repository) -> Result<bool, GitError> {
    if repo.is_dirty().map_err(git_error)? {
        return Ok(true);
    }
    let mut changes = repo
        .status(gix::progress::Discard)
        .map_err(git_error)?
        .untracked_files(gix::status::UntrackedFiles::Files)
        .into_index_worktree_iter(Vec::<gix::bstr::BString>::new())
        .map_err(git_error)?;
    match changes.next() {
        None => Ok(false),
        Some(Ok(_)) => Ok(true),
        Some(Err(error)) => Err(git_error(error)),
    }
}

fn ensure_same_repository(parent: &GitRepository, child: &GitRepository) -> Result<(), GitError> {
    if parent.common_directory == child.common_directory {
        Ok(())
    } else {
        Err(GitError(format!(
            "worktree does not belong to this repository ({} vs {})",
            child.common_directory.display(),
            parent.common_directory.display()
        )))
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
    let slug = slug.trim_matches('-');
    // A branch made only of non-ASCII characters would otherwise land on the container itself.
    if slug.is_empty() {
        "worktree".to_owned()
    } else {
        slug.to_owned()
    }
}
