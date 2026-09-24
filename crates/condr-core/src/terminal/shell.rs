use super::*;

/// Interactive shells a Pane may run, by lower-cased file name without `.exe`: the
/// ones that accept a command line typed at their prompt. Shared by launch (idle
/// shell check) and process identification (a shell wrapping an agent).
pub(crate) const KNOWN_SHELLS: &[&str] = &[
    "sh",
    "bash",
    "dash",
    "zsh",
    "fish",
    "ksh",
    "mksh",
    "csh",
    "tcsh",
    "elvish",
    "xonsh",
    "nu",
    "pwsh",
    "powershell",
    "cmd",
];

#[cfg(any(windows, test))]
const WINDOWS_POWERSHELL_CWD_HOOK: &str = r"if ($null -eq $global:__CondrOriginalPrompt) { $global:__CondrOriginalPrompt = $function:prompt; function global:prompt { $out = @(& $global:__CondrOriginalPrompt) -join ' '; $loc = $ExecutionContext.SessionState.Path.CurrentLocation; if ($loc.Provider.Name -eq 'FileSystem') { try { [Environment]::CurrentDirectory = $loc.ProviderPath } catch {}; $esc = [string][char]27; $out += $esc + ']9;9;' + $loc.ProviderPath + $esc + '\' }; $out } }";

#[cfg(target_os = "linux")]
const LINUX_BASH_CWD_WRAPPER: &str = r#"exec 3<<'__CONDR_BASHRC__'
if [[ -r "$HOME/.bashrc" ]]; then source "$HOME/.bashrc"; fi
__condr_user_exit=
__condr_trap=$(trap -p EXIT)
if [[ -n $__condr_trap ]]; then
  __condr_trap=${__condr_trap% EXIT}
  eval "__condr_user_exit=${__condr_trap#trap -- }"
fi
__condr_return_status(){ return "$1"; }
trap '__condr_status=$?; printf "\033]9;9;%s\033\\" "$PWD"; if [[ -n $__condr_user_exit ]]; then __condr_return_status "$__condr_status"; eval "$__condr_user_exit"; fi; __condr_return_status "$__condr_status"' EXIT
unset __condr_trap
__CONDR_BASHRC__
exec "$1" --rcfile /dev/fd/3 -i
"#;

/// What a Pane's shell and everything it starts learn about their surroundings, as
/// environment variables. herdr does the same with `HERDR_*`; a `condr` CLI or an agent
/// hook running inside the Pane uses these to reach the Server and name its Pane.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PaneEnvironment {
    pub pane_id: PaneId,
    /// The Server's endpoint as `CONDR_SOCKET_PATH`: a local socket or pipe path, or
    /// `tcp://host:port`.
    pub socket_path: String,
}

impl PaneEnvironment {
    pub const ENV: &'static str = "CONDR_ENV";
    pub const PANE_ID: &'static str = "CONDR_PANE_ID";
    pub const SOCKET_PATH: &'static str = "CONDR_SOCKET_PATH";
    pub const BIN_PATH: &'static str = "CONDR_BIN_PATH";

    /// Sets the `CONDR_*` variables and puts the directory holding this executable in
    /// front of `PATH`, so a portable or `cargo run` build has `condr` on the Pane's PATH
    /// without an install step.
    pub fn apply(&self, command: &mut CommandBuilder) {
        command.env(Self::ENV, "1");
        command.env(Self::PANE_ID, self.pane_id.as_u64().to_string());
        command.env(Self::SOCKET_PATH, &self.socket_path);
        if let Ok(executable) = std::env::current_exe() {
            command.env(Self::BIN_PATH, executable);
        }
        if let Some(path) = prepend_executable_directory(
            command
                .get_env("PATH")
                .map(ToOwned::to_owned)
                .or_else(|| std::env::var_os("PATH")),
        ) {
            command.env("PATH", path);
        }
    }
}

fn prepend_executable_directory(path: Option<std::ffi::OsString>) -> Option<std::ffi::OsString> {
    let directory = std::env::current_exe().ok()?.parent()?.to_path_buf();
    let rest = path.unwrap_or_default();
    let mut entries = vec![directory.clone()];
    entries.extend(
        std::env::split_paths(&rest)
            .filter(|entry| !entry.as_os_str().is_empty() && *entry != directory),
    );
    std::env::join_paths(entries).ok()
}

/// The shell a terminal runs when none is configured: `$SHELL`, then the password
/// database, on Unix (as herdr and Zed do); on Windows PowerShell 7 when installed,
/// else Windows PowerShell, else `%ComSpec%`, in Zed's order.
pub fn default_shell_program() -> String {
    #[cfg(windows)]
    {
        // ponytail: PATH lookup only; Zed also probes Program Files, MSIX, scoop and
        // dotnet tools for pwsh. Add those when a real install goes unnoticed.
        if is_on_path("pwsh.exe") {
            return "pwsh.exe".to_owned();
        }
        if is_on_path("powershell.exe") {
            return "powershell.exe".to_owned();
        }
        std::env::var("ComSpec")
            .ok()
            .map(|comspec| comspec.trim().to_owned())
            .filter(|comspec| !comspec.is_empty())
            .unwrap_or_else(|| "cmd.exe".to_owned())
    }
    #[cfg(not(windows))]
    CommandBuilder::new_default_prog().get_shell()
}

/// A Server started by the app inherits launchd's environment, which names no locale, so
/// the shell would fall back to C and mangle UTF-8 input. Name the system locale the way
/// Terminal.app does, or only the UTF-8 encoding when that locale is not installed (an
/// `en_CN` system); a locale the environment already names wins.
#[cfg(target_os = "macos")]
pub(super) fn set_missing_locale(command: &mut CommandBuilder) {
    if ["LC_ALL", "LC_CTYPE", "LANG"]
        .iter()
        .any(|name| command.get_env(name).is_some_and(|value| !value.is_empty()))
    {
        return;
    }
    match system_locale().filter(|locale| Path::new("/usr/share/locale").join(locale).is_dir()) {
        Some(locale) => command.env("LANG", locale),
        None => command.env("LC_CTYPE", "UTF-8"),
    }
}

/// The user's locale as `language_COUNTRY.UTF-8`.
#[cfg(target_os = "macos")]
#[allow(unsafe_code)] // CFLocale has no safe wrapper in the CoreFoundation crates GPUI builds.
fn system_locale() -> Option<String> {
    use core_foundation_sys::base::CFRelease;
    use core_foundation_sys::locale::{
        CFLocaleCopyCurrent, CFLocaleGetValue, CFLocaleKey, CFLocaleRef, kCFLocaleCountryCode,
        kCFLocaleLanguageCode,
    };
    use core_foundation_sys::string::{CFStringGetCString, kCFStringEncodingUTF8};

    let code = |locale: CFLocaleRef, key: CFLocaleKey| {
        let mut buffer = [0; 16];
        // SAFETY: `locale` is live and owns the CFString it returns, which is copied into
        // `buffer` within its length and NUL-terminated when the copy succeeds.
        unsafe {
            let value = CFLocaleGetValue(locale, key);
            (!value.is_null()
                && CFStringGetCString(
                    value.cast(),
                    buffer.as_mut_ptr(),
                    buffer.len() as _,
                    kCFStringEncodingUTF8,
                ) != 0)
                .then(|| {
                    std::ffi::CStr::from_ptr(buffer.as_ptr())
                        .to_string_lossy()
                        .into_owned()
                })
        }
    };
    // SAFETY: the copied locale is released once, after its codes were copied out.
    unsafe {
        let locale = CFLocaleCopyCurrent();
        if locale.is_null() {
            return None;
        }
        let name = code(locale, kCFLocaleLanguageCode)
            .zip(code(locale, kCFLocaleCountryCode))
            .map(|(language, country)| format!("{language}_{country}.UTF-8"));
        CFRelease(locale.cast());
        name
    }
}

#[cfg(windows)]
fn is_on_path(file_name: &str) -> bool {
    std::env::var_os("PATH").is_some_and(|path| {
        std::env::split_paths(&path).any(|directory| directory.join(file_name).is_file())
    })
}

/// The shell's file name without `.exe`, lower-cased; both separators count so a
/// Windows path classifies the same on every host.
fn shell_name(program: &str) -> String {
    let name = program
        .rsplit(['/', '\\'])
        .next()
        .unwrap_or(program)
        .to_ascii_lowercase();
    name.strip_suffix(".exe").unwrap_or(&name).to_owned()
}

/// The interactive shell command, with cwd reporting for the shells we know how to
/// hook: bash on Linux through a wrapper rc file, PowerShell on Windows through a
/// prompt hook. Any other shell starts plain.
pub(super) fn shell_command(program: &str) -> CommandBuilder {
    match shell_name(program).as_str() {
        #[cfg(target_os = "linux")]
        "bash" => {
            let mut command = CommandBuilder::new("/bin/sh");
            command.args(["-c", LINUX_BASH_CWD_WRAPPER, "condr-shell", program]);
            command
        }
        #[cfg(windows)]
        "pwsh" | "powershell" => {
            let mut command = CommandBuilder::new(program);
            command.args([
                "-NoLogo",
                "-NoExit",
                "-Command",
                WINDOWS_POWERSHELL_CWD_HOOK,
            ]);
            command
        }
        _ => CommandBuilder::new(program),
    }
}

#[cfg(test)]
mod shell_tests {
    use super::*;

    #[test]
    fn shell_names_ignore_directories_extensions_and_case() {
        assert_eq!(shell_name("/usr/bin/bash"), "bash");
        assert_eq!(
            shell_name(r"C:\Program Files\PowerShell\7\pwsh.exe"),
            "pwsh"
        );
        assert_eq!(shell_name("PowerShell.EXE"), "powershell");
        assert_eq!(shell_name("fish"), "fish");
    }

    #[test]
    fn unknown_shells_start_without_extra_arguments() {
        assert_eq!(shell_command("fish").get_argv(), &["fish"]);
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn bash_is_wrapped_for_cwd_reporting() {
        let command = shell_command("/usr/local/bin/bash");
        let argv = command.get_argv();
        assert_eq!(argv[0], "/bin/sh");
        assert_eq!(argv.last().unwrap(), "/usr/local/bin/bash");
    }

    #[cfg(windows)]
    #[test]
    fn powershell_gets_the_prompt_hook() {
        for program in [
            "pwsh.exe",
            r"C:\Windows\System32\WindowsPowerShell\v1.0\powershell.exe",
        ] {
            let command = shell_command(program);
            let argv = command.get_argv();
            assert_eq!(argv[0], program);
            assert_eq!(argv.last().unwrap(), WINDOWS_POWERSHELL_CWD_HOOK);
        }
        assert_eq!(shell_command("cmd.exe").get_argv(), &["cmd.exe"]);
    }

    #[test]
    fn the_pane_environment_names_the_server_and_pane_and_leads_path() {
        let mut command = CommandBuilder::new("sh");
        command.env("PATH", "/usr/bin");
        PaneEnvironment {
            pane_id: PaneId::from_u64(42),
            socket_path: "/run/condr.sock".into(),
        }
        .apply(&mut command);
        assert_eq!(command.get_env("CONDR_ENV").unwrap(), "1");
        assert_eq!(command.get_env("CONDR_PANE_ID").unwrap(), "42");
        assert_eq!(
            command.get_env("CONDR_SOCKET_PATH").unwrap(),
            "/run/condr.sock"
        );
        assert_eq!(
            command.get_env("CONDR_BIN_PATH").unwrap(),
            std::env::current_exe().unwrap().as_os_str()
        );
        let path = command.get_env("PATH").unwrap().to_owned();
        let mut entries = std::env::split_paths(&path);
        assert_eq!(
            entries.next().as_deref(),
            std::env::current_exe().unwrap().parent()
        );
        assert_eq!(entries.next().as_deref(), Some(Path::new("/usr/bin")));
    }

    #[test]
    fn the_default_shell_is_never_blank() {
        assert!(!default_shell_program().trim().is_empty());
    }
    #[test]
    fn powershell_cwd_hook_syncs_win32_before_reporting() {
        let sync = WINDOWS_POWERSHELL_CWD_HOOK
            .find("[Environment]::CurrentDirectory = $loc.ProviderPath")
            .unwrap();
        let report = WINDOWS_POWERSHELL_CWD_HOOK.find("]9;9;").unwrap();
        assert!(sync < report);
    }
}
