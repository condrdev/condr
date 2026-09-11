//! Agent CLIs available on this machine's PATH, independent of live Pane detection.
//! No CLI is executed here.

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::AgentKind;

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct AgentInstallation {
    pub kind: AgentKind,
    /// Absolute PATH entry, preserving symlinks and package-manager shims.
    pub executable: PathBuf,
}

impl AgentKind {
    /// The native interactive CLI command.
    pub const fn executable(self) -> &'static str {
        match self {
            Self::Antigravity => "agy",
            Self::Cursor => "cursor-agent",
            _ => self.id(),
        }
    }
}

/// Reads the caller's current PATH on every call. An absent PATH yields no agents.
/// Shell aliases, shell profile changes and installations outside PATH are not searched.
pub fn discover() -> Vec<AgentInstallation> {
    let Some(path) = std::env::var_os("PATH") else {
        return Vec::new();
    };
    let directories = std::env::split_paths(&path)
        .filter_map(|directory| std::path::absolute(directory).ok())
        .collect::<Vec<_>>();
    discover_in(&directories)
}

fn discover_in(directories: &[PathBuf]) -> Vec<AgentInstallation> {
    AgentKind::ALL
        .into_iter()
        .filter_map(|kind| {
            directories.iter().find_map(|directory| {
                candidates(directory, kind.executable())
                    .into_iter()
                    .find(|path| executable_file(path))
                    .map(|executable| AgentInstallation { kind, executable })
            })
        })
        .collect()
}

fn candidates(directory: &Path, command: &str) -> Vec<PathBuf> {
    #[cfg(windows)]
    if Path::new(command).extension().is_none() {
        return [".exe", ".cmd", ".bat", ".ps1"]
            .map(|extension| directory.join(format!("{command}{extension}")))
            .into();
    }
    vec![directory.join(command)]
}

fn executable_file(path: &Path) -> bool {
    let Ok(metadata) = path.metadata() else {
        return false;
    };
    if !metadata.is_file() {
        return false;
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt as _;
        metadata.permissions().mode() & 0o111 != 0
    }
    #[cfg(not(unix))]
    {
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn discovery_checks_files_and_path_order_without_executing_agents() {
        let root = std::env::temp_dir().join(format!(
            "condr-agent-discovery-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let first = root.join("first directory");
        let second = root.join("second");
        std::fs::create_dir_all(&first).unwrap();
        std::fs::create_dir_all(&second).unwrap();
        let codex = candidates(&first, "codex")[0].clone();
        let fallback = candidates(&second, "codex")[0].clone();
        let opencode = candidates(&first, AgentKind::OpenCode.executable())[0].clone();
        for path in [&codex, &fallback, &opencode] {
            std::fs::write(path, "this must never be executed").unwrap();
            #[cfg(unix)]
            {
                use std::os::unix::fs::PermissionsExt as _;
                std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o755)).unwrap();
            }
        }
        std::fs::create_dir(first.join("claude")).unwrap();
        let directories = [first, second];
        let found = discover_in(&directories);
        assert_eq!(found.len(), 2);
        assert_eq!(
            found[0],
            AgentInstallation {
                kind: AgentKind::Codex,
                executable: codex.clone()
            }
        );
        assert_eq!(
            found[1],
            AgentInstallation {
                kind: AgentKind::OpenCode,
                executable: opencode
            }
        );
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt as _;
            std::fs::set_permissions(&codex, std::fs::Permissions::from_mode(0o644)).unwrap();
            assert_eq!(discover_in(&directories)[0].executable, fallback);
        }
        assert!(discover_in(&[]).is_empty());
        std::fs::remove_dir_all(root).unwrap();
    }
}
