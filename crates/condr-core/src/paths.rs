use std::path::{Path, PathBuf};

pub fn executable_directory() -> PathBuf {
    std::env::current_exe()
        .ok()
        .and_then(|executable| executable.parent().map(Path::to_path_buf))
        .or_else(|| std::env::current_dir().ok())
        .unwrap_or_else(|| PathBuf::from("."))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn executable_directory_is_the_parent_of_the_running_binary() {
        assert_eq!(
            executable_directory(),
            std::env::current_exe().unwrap().parent().unwrap()
        );
        assert_eq!(
            crate::default_worktree_root(),
            executable_directory().join("worktrees")
        );
    }
}
