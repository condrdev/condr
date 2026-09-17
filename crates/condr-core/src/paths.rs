use std::path::PathBuf;

const DIRECTORY_NAME: &str = "condr";

/// Condr's configuration directory: `CONDR_CONFIG_DIR` when set, else the platform's.
/// It also holds the Server identity and the device key of this host.
pub fn config_directory() -> Option<PathBuf> {
    if let Some(path) = std::env::var_os("CONDR_CONFIG_DIR") {
        return Some(PathBuf::from(path));
    }
    dirs::config_dir().map(|root| root.join(DIRECTORY_NAME))
}

pub fn data_directory() -> Option<PathBuf> {
    dirs::data_local_dir().map(|root| root.join(DIRECTORY_NAME))
}

pub fn state_directory() -> Option<PathBuf> {
    dirs::state_dir()
        .or_else(dirs::data_local_dir)
        .map(|root| root.join(DIRECTORY_NAME))
}

/// Condr's log directory: `CONDR_LOG_DIR` when set (tests point helper Servers at a
/// temporary directory), else the platform's.
pub fn log_directory() -> Option<PathBuf> {
    if let Some(path) = std::env::var_os("CONDR_LOG_DIR") {
        return Some(PathBuf::from(path));
    }
    #[cfg(target_os = "macos")]
    return dirs::home_dir().map(|root| root.join("Library/Logs").join(DIRECTORY_NAME));

    #[cfg(not(target_os = "macos"))]
    state_directory()
}

pub fn runtime_directory() -> Option<PathBuf> {
    if let Some(root) = dirs::runtime_dir() {
        return Some(root.join(DIRECTORY_NAME));
    }

    #[cfg(target_os = "macos")]
    return Some(std::env::temp_dir().join(DIRECTORY_NAME));

    #[cfg(not(target_os = "macos"))]
    data_directory().map(|root| root.join("runtime"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn directories_follow_the_platform_roots() {
        assert_eq!(
            config_directory(),
            dirs::config_dir().map(|root| root.join(DIRECTORY_NAME))
        );
        assert_eq!(
            data_directory(),
            dirs::data_local_dir().map(|root| root.join(DIRECTORY_NAME))
        );
        assert_eq!(
            state_directory(),
            dirs::state_dir()
                .or_else(dirs::data_local_dir)
                .map(|root| root.join(DIRECTORY_NAME))
        );

        #[cfg(target_os = "macos")]
        assert_eq!(
            log_directory(),
            dirs::home_dir().map(|root| root.join("Library/Logs").join(DIRECTORY_NAME))
        );
        #[cfg(not(target_os = "macos"))]
        assert_eq!(log_directory(), state_directory());

        #[cfg(target_os = "macos")]
        let expected_runtime = Some(std::env::temp_dir().join(DIRECTORY_NAME));
        #[cfg(not(target_os = "macos"))]
        let expected_runtime = data_directory().map(|root| root.join("runtime"));
        assert_eq!(
            runtime_directory(),
            dirs::runtime_dir()
                .map(|root| root.join(DIRECTORY_NAME))
                .or(expected_runtime)
        );
    }
}
