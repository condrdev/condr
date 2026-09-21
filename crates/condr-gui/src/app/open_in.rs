//! "Open in": the title bar's split button that opens the presented Workspace's root in
//! an editor or the system file manager.
//!
//! Detection and launching happen in the GUI process, on the GUI's machine, so the
//! button only works against a local Server: a remote Server's root is a path on another
//! machine. Built-in presets are probed at startup; `[[client.editors]]` entries are
//! taken as written; the file manager is always last and always present.

use super::sidebar::CondrIconName;
use super::*;
use gpui_kit::component::Side;
use gpui_kit::component::button::DropdownButton;
use gpui_kit::component::notification::Notification;
use serde::Deserialize;
use std::ffi::OsString;
use std::io;
use std::path::Path;

/// Which mark the button and menu draw for a target.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum OpenTargetIcon {
    Zed,
    VsCode,
    Cursor,
    IntellijIdea,
    /// A `[[client.editors]]` entry: Condr knows nothing about its mark.
    Custom,
    FileManager,
}

impl OpenTargetIcon {
    fn icon(self) -> Icon {
        match self {
            Self::Zed => Icon::new(CondrIconName::Zed),
            Self::VsCode => Icon::new(CondrIconName::VsCode),
            Self::Cursor => Icon::new(CondrIconName::Cursor),
            Self::IntellijIdea => Icon::new(CondrIconName::IntellijIdea),
            Self::Custom => Icon::new(IconName::ExternalLink),
            Self::FileManager => Icon::new(IconName::Folder),
        }
    }
}

/// Something a Workspace root can be opened in: a program plus the arguments that go
/// before the directory, which is always the last argument.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct OpenTarget {
    /// The key saved in `config.toml`: a preset's id, `files`, or a custom entry's name.
    pub(super) id: String,
    pub(super) label: SharedString,
    pub(super) icon: OpenTargetIcon,
    pub(super) program: PathBuf,
    pub(super) args: Vec<OsString>,
}

pub(super) const FILE_MANAGER_ID: &str = "files";

/// One `[[client.editors]]` entry, taken as written and never probed.
#[derive(Clone, Debug, Deserialize, PartialEq, Eq)]
pub(super) struct CustomEditor {
    pub name: String,
    /// The program and its leading arguments; the directory is appended.
    pub command: Vec<String>,
}

impl CustomEditor {
    fn target(&self) -> Option<OpenTarget> {
        let (program, args) = self.command.split_first()?;
        let name = self.name.trim();
        if name.is_empty() || program.trim().is_empty() {
            return None;
        }
        Some(OpenTarget {
            id: name.to_owned(),
            label: name.to_owned().into(),
            icon: OpenTargetIcon::Custom,
            program: PathBuf::from(program),
            args: args.iter().map(OsString::from).collect(),
        })
    }
}

/// The platform whose install layout is probed; a parameter so every layout is testable
/// on the Linux CI box.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Platform {
    Windows,
    MacOs,
    Linux,
}

impl Platform {
    const fn current() -> Self {
        if cfg!(windows) {
            Self::Windows
        } else if cfg!(target_os = "macos") {
            Self::MacOs
        } else {
            Self::Linux
        }
    }
}

/// Where a machine keeps programs: the roots the presets are probed under.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(super) struct InstallRoots {
    pub(super) platform: Option<Platform>,
    /// `PATH`, split.
    pub(super) path: Vec<PathBuf>,
    pub(super) home: Option<PathBuf>,
    /// Windows `%LOCALAPPDATA%`.
    pub(super) local_app_data: Option<PathBuf>,
    /// Windows `%ProgramFiles%`.
    pub(super) program_files: Option<PathBuf>,
    /// macOS `/Applications` and `~/Applications`.
    pub(super) applications: Vec<PathBuf>,
}

impl InstallRoots {
    pub(super) fn from_environment() -> Self {
        let home = std::env::home_dir();
        let applications = if cfg!(target_os = "macos") {
            let mut applications = vec![PathBuf::from("/Applications")];
            applications.extend(home.as_ref().map(|home| home.join("Applications")));
            applications
        } else {
            Vec::new()
        };
        Self {
            platform: Some(Platform::current()),
            path: std::env::var_os("PATH")
                .map(|path| std::env::split_paths(&path).collect())
                .unwrap_or_default(),
            local_app_data: std::env::var_os("LOCALAPPDATA").map(PathBuf::from),
            program_files: std::env::var_os("ProgramFiles").map(PathBuf::from),
            applications,
            home,
        }
    }

    fn platform(&self) -> Platform {
        self.platform.unwrap_or(Platform::current())
    }

    /// The first `PATH` entry holding one of `names` as a file.
    fn on_path(&self, names: &[&str]) -> Option<PathBuf> {
        self.path
            .iter()
            .flat_map(|directory| names.iter().map(move |name| directory.join(name)))
            .find(|candidate| candidate.is_file())
    }

    fn first_file(candidates: impl IntoIterator<Item = PathBuf>) -> Option<PathBuf> {
        candidates.into_iter().find(|candidate| candidate.is_file())
    }

    /// `<root>/<prefix>*/<tail>` for the best-sorting matching directory: JetBrains
    /// installs carry their version in the directory name.
    fn versioned(root: Option<&Path>, prefix: &str, tail: &str) -> Option<PathBuf> {
        let mut matches = std::fs::read_dir(root?)
            .ok()?
            .filter_map(Result::ok)
            .filter_map(|entry| {
                let name = entry.file_name();
                name.to_str()?.starts_with(prefix).then(|| entry.path())
            })
            .collect::<Vec<_>>();
        // The Ultimate edition sorts before "IntelliJ IDEA Community Edition …", and the
        // newest version of either sorts last within its edition; prefer the edition.
        matches.sort();
        let ultimate = matches
            .iter()
            .rfind(|path| !path.to_string_lossy().contains("Community"));
        ultimate
            .or(matches.last())
            .map(|path| path.join(tail))
            .filter(|path| path.is_file())
    }

    fn mac_application(&self, name: &str) -> bool {
        self.applications
            .iter()
            .any(|root| root.join(format!("{name}.app")).is_dir())
    }
}

fn target(
    id: &str,
    label: &str,
    icon: OpenTargetIcon,
    program: PathBuf,
    args: &[&str],
) -> OpenTarget {
    OpenTarget {
        id: id.to_owned(),
        label: label.to_owned().into(),
        icon,
        program,
        args: args.iter().map(OsString::from).collect(),
    }
}

/// `open -na <app> --args`: a new instance that takes the directory as an argument.
/// A plain `open -a` hands a running JetBrains IDE an open-document event, which reopens
/// its last project instead; Cursor routes bare directories to its last window.
fn mac_open(id: &str, label: &str, icon: OpenTargetIcon, app: &str, args: &[&str]) -> OpenTarget {
    let mut open_args = vec!["-n", "-a", app, "--args"];
    open_args.extend_from_slice(args);
    target(id, label, icon, PathBuf::from("/usr/bin/open"), &open_args)
}

fn detect_zed(roots: &InstallRoots) -> Option<OpenTarget> {
    const ID: &str = "zed";
    const LABEL: &str = "Zed";
    let program = match roots.platform() {
        Platform::Windows => InstallRoots::first_file(
            roots
                .local_app_data
                .iter()
                .map(|root| root.join("Programs").join("Zed").join("Zed.exe")),
        )
        .or_else(|| roots.on_path(&["Zed.exe", "zed.exe"])),
        Platform::MacOs => match roots.on_path(&["zed"]) {
            Some(cli) => Some(cli),
            None if roots.mac_application("Zed") => {
                return Some(mac_open(ID, LABEL, OpenTargetIcon::Zed, "Zed", &[]));
            }
            None => None,
        },
        Platform::Linux => roots.on_path(&["zed", "zeditor", "zedit"]),
    }?;
    Some(target(ID, LABEL, OpenTargetIcon::Zed, program, &[]))
}

fn detect_vscode(roots: &InstallRoots) -> Option<OpenTarget> {
    const ID: &str = "vscode";
    const LABEL: &str = "VS Code";
    let program = match roots.platform() {
        // The .exe rather than bin\code.cmd: no console-window flash, no cmd.exe quoting.
        Platform::Windows => InstallRoots::first_file(
            roots
                .local_app_data
                .iter()
                .map(|root| root.join("Programs"))
                .chain(roots.program_files.iter().cloned())
                .map(|root| root.join("Microsoft VS Code").join("Code.exe")),
        )
        .or_else(|| roots.on_path(&["Code.exe", "code.cmd"])),
        Platform::MacOs => match roots.on_path(&["code"]) {
            Some(cli) => Some(cli),
            None if roots.mac_application("Visual Studio Code") => {
                return Some(mac_open(
                    ID,
                    LABEL,
                    OpenTargetIcon::VsCode,
                    "Visual Studio Code",
                    &[],
                ));
            }
            None => None,
        },
        Platform::Linux => roots.on_path(&["code"]),
    }?;
    Some(target(ID, LABEL, OpenTargetIcon::VsCode, program, &[]))
}

fn detect_cursor(roots: &InstallRoots) -> Option<OpenTarget> {
    const ID: &str = "cursor";
    const LABEL: &str = "Cursor";
    const NEW_WINDOW: &[&str] = &["--new-window"];
    let program = match roots.platform() {
        Platform::Windows => InstallRoots::first_file(
            roots
                .local_app_data
                .iter()
                .map(|root| root.join("Programs").join("cursor").join("Cursor.exe"))
                .chain(
                    roots
                        .program_files
                        .iter()
                        .map(|root| root.join("Cursor").join("Cursor.exe")),
                ),
        )
        .or_else(|| roots.on_path(&["Cursor.exe", "cursor.cmd"])),
        Platform::MacOs => match roots.on_path(&["cursor"]) {
            Some(cli) => Some(cli),
            None if roots.mac_application("Cursor") => {
                return Some(mac_open(
                    ID,
                    LABEL,
                    OpenTargetIcon::Cursor,
                    "Cursor",
                    NEW_WINDOW,
                ));
            }
            None => None,
        },
        Platform::Linux => roots.on_path(&["cursor"]),
    }?;
    Some(target(
        ID,
        LABEL,
        OpenTargetIcon::Cursor,
        program,
        NEW_WINDOW,
    ))
}

fn detect_idea(roots: &InstallRoots) -> Option<OpenTarget> {
    const ID: &str = "idea";
    const LABEL: &str = "IntelliJ IDEA";
    let program = match roots.platform() {
        Platform::Windows => {
            const EXE: &str = "bin/idea64.exe";
            // A Program Files install, a Toolbox 2 install, then the Toolbox launcher
            // script; the latter is a .cmd, which std runs through cmd.exe for us.
            let jetbrains = roots
                .program_files
                .as_ref()
                .map(|root| root.join("JetBrains"));
            let programs = roots
                .local_app_data
                .as_ref()
                .map(|root| root.join("Programs"));
            InstallRoots::versioned(jetbrains.as_deref(), LABEL, EXE)
                .or_else(|| InstallRoots::versioned(programs.as_deref(), LABEL, EXE))
                .or_else(|| {
                    InstallRoots::first_file(roots.local_app_data.iter().map(|root| {
                        root.join("JetBrains")
                            .join("Toolbox")
                            .join("scripts")
                            .join("idea.cmd")
                    }))
                })
                .or_else(|| roots.on_path(&["idea64.exe", "idea.cmd"]))
        }
        Platform::MacOs => {
            let toolbox = roots.home.iter().map(|home| {
                home.join("Library")
                    .join("Application Support")
                    .join("JetBrains")
                    .join("Toolbox")
                    .join("scripts")
                    .join("idea")
            });
            match InstallRoots::first_file(toolbox).or_else(|| roots.on_path(&["idea"])) {
                Some(cli) => Some(cli),
                None => {
                    let app = ["IntelliJ IDEA", "IntelliJ IDEA CE"]
                        .into_iter()
                        .find(|app| roots.mac_application(app))?;
                    return Some(mac_open(ID, LABEL, OpenTargetIcon::IntellijIdea, app, &[]));
                }
            }
        }
        Platform::Linux => {
            let toolbox = roots.home.iter().map(|home| {
                home.join(".local")
                    .join("share")
                    .join("JetBrains")
                    .join("Toolbox")
                    .join("scripts")
                    .join("idea")
            });
            InstallRoots::first_file(toolbox).or_else(|| {
                roots.on_path(&["idea", "intellij-idea-ultimate", "intellij-idea-community"])
            })
        }
    }?;
    Some(target(
        ID,
        LABEL,
        OpenTargetIcon::IntellijIdea,
        program,
        &[],
    ))
}

/// The platform's file manager, opened on the directory's contents. Always offered:
/// the programs are part of the OS.
fn file_manager(platform: Platform) -> OpenTarget {
    let (label, program) = match platform {
        Platform::Windows => ("Explorer", "explorer.exe"),
        Platform::MacOs => ("Finder", "/usr/bin/open"),
        Platform::Linux => ("Files", "xdg-open"),
    };
    target(
        FILE_MANAGER_ID,
        label,
        OpenTargetIcon::FileManager,
        PathBuf::from(program),
        &[],
    )
}

/// Everything the menu offers, in menu order: detected presets, custom entries, the
/// file manager. A custom entry named like a preset replaces the preset.
pub(super) fn available_targets(roots: &InstallRoots, custom: &[CustomEditor]) -> Vec<OpenTarget> {
    let custom = custom
        .iter()
        .filter_map(CustomEditor::target)
        .collect::<Vec<_>>();
    let mut targets = [detect_zed, detect_vscode, detect_cursor, detect_idea]
        .into_iter()
        .filter_map(|detect| detect(roots))
        .filter(|preset| !custom.iter().any(|entry| entry.id == preset.id))
        .collect::<Vec<_>>();
    targets.extend(custom);
    targets.push(file_manager(roots.platform()));
    targets
}

/// Shows `path`, a directory on this machine, in the platform's file manager.
pub(super) fn reveal(path: &Path) -> io::Result<()> {
    launch(&file_manager(Platform::current()), path)
}

/// Starts the program on `path`, a directory or a file, and returns once it has been
/// spawned. The child
/// is reaped on a helper thread so a CLI that exits at once leaves no zombie behind.
pub(super) fn launch(target: &OpenTarget, path: &Path) -> io::Result<()> {
    if !path.exists() {
        return Err(io::Error::new(
            io::ErrorKind::NotFound,
            format!("{} does not exist", path.display()),
        ));
    }
    let mut command = std::process::Command::new(&target.program);
    command
        .args(&target.args)
        .arg(path)
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null());
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt as _;
        // A .cmd launcher would otherwise flash a console window.
        const CREATE_NO_WINDOW: u32 = 0x0800_0000;
        command.creation_flags(CREATE_NO_WINDOW);
    }
    let mut child = command.spawn()?;
    thread::spawn(move || {
        let _ = child.wait();
    });
    Ok(())
}

/// The key a Workspace's editor choice is saved under: the repository, so every worktree
/// of it shares one choice.
/// How long a successful launch keeps the button spinning. Spawning returns in
/// milliseconds while the editor's window takes seconds to appear, and nothing tells us
/// when it does; the floor is what makes the click visibly land.
// ponytail: fixed floor; watch the child's first window if editors ever report it.
const OPEN_IN_FEEDBACK: Duration = Duration::from_millis(1500);

pub(super) fn project_root(workspace: &condr_core::Workspace) -> &Path {
    workspace
        .worktree()
        .map_or(workspace.root_directory(), |worktree| {
            worktree.parent_root_directory()
        })
}

impl Condr {
    /// Probes the presets off the GUI thread; until it reports, the button is not shown.
    pub(super) fn scan_open_targets(&mut self, cx: &mut Context<Self>) {
        let custom = self.custom_editors.clone();
        cx.spawn(async move |this, cx| {
            let targets = cx
                .background_spawn(async move {
                    available_targets(&InstallRoots::from_environment(), &custom)
                })
                .await;
            let _ = this.update(cx, |this, cx| {
                this.open_targets = Some(targets);
                cx.notify();
            });
        })
        .detach();
    }

    /// This project's choice, else the last editor used anywhere, else the first target;
    /// a saved choice whose program is gone falls through rather than being rewritten.
    pub(super) fn open_target_for(&self, project_root: &Path) -> Option<&OpenTarget> {
        let targets = self.open_targets.as_deref()?;
        let find = |id: &str| targets.iter().find(|target| target.id == id);
        self.workspace_editors
            .get(project_root)
            .and_then(|id| find(id))
            .or_else(|| self.default_editor.as_deref().and_then(find))
            .or_else(|| targets.first())
    }

    /// The title bar's "Open in" control for the presented Workspace, or nothing while the
    /// scan is still running.
    pub(super) fn render_open_in(
        &self,
        connection: &ServerConnection,
        workspace: &condr_core::Workspace,
        cx: &mut Context<Self>,
    ) -> Option<AnyElement> {
        let targets = self.open_targets.as_deref()?;
        let current = self.open_target_for(project_root(workspace))?;
        let is_local = connection.endpoint.as_local_path().is_some();
        let key = connection.key;
        let workspace_id = workspace.id();
        let tooltip: SharedString = if is_local {
            format!("Open in {}", current.label).into()
        } else {
            "Opening in an editor needs a local Server".into()
        };
        let primary_owner = cx.weak_entity();
        let current_id = current.id.clone();
        let opening = self.opening_workspace == Some((key, workspace_id));
        let primary = Button::new("open-in")
            .debug_selector(|| "open-in".into())
            // The default variant: the border is what tells the split control apart
            // from the Tabs beside it.
            .small()
            .icon(current.icon.icon())
            .tooltip(tooltip.clone())
            .accessibility_label(tooltip)
            .disabled(!is_local)
            .loading(opening)
            .on_click(move |_, window, cx| {
                let _ = primary_owner.update(cx, |this, cx| {
                    this.open_workspace_in(key, workspace_id, current_id.clone(), window, cx)
                });
            });
        let control = if targets.len() > 1 {
            let menu_owner = cx.weak_entity();
            let items = targets
                .iter()
                .map(|target| (target.id.clone(), target.label.clone(), target.icon))
                .collect::<Vec<_>>();
            let checked = current.id.clone();
            DropdownButton::new("open-in-menu")
                .button(primary)
                .disabled(!is_local)
                .dropdown_menu(move |menu, _, _| {
                    items.iter().fold(
                        menu.check_side(Side::Right).min_w(px(180.)),
                        |menu, (id, label, icon)| {
                            let owner = menu_owner.clone();
                            let id = id.clone();
                            menu.item(
                                PopupMenuItem::new(label.clone())
                                    .icon(icon.icon())
                                    .checked(*id == checked)
                                    .on_click(move |_, window, cx| {
                                        let _ = owner.update(cx, |this, cx| {
                                            this.open_workspace_in(
                                                key,
                                                workspace_id,
                                                id.clone(),
                                                window,
                                                cx,
                                            )
                                        });
                                    }),
                            )
                        },
                    )
                })
                .into_any_element()
        } else {
            primary.into_any_element()
        };
        Some(
            h_flex()
                .debug_selector(|| "open-in-control".into())
                .flex_none()
                .h_full()
                .items_center()
                .px_2()
                // Inside the title bar an unclaimed press starts a window move on
                // Windows, and the move swallows the click.
                .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
                .child(control)
                .into_any_element(),
        )
    }

    /// Opens the Workspace's root in `target_id`; on success that becomes the project's
    /// editor and the last one used, and both are saved.
    pub(super) fn open_workspace_in(
        &mut self,
        key: ConnectionKey,
        workspace_id: WorkspaceId,
        target_id: String,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(connection) = self.connection(key) else {
            return;
        };
        let Ok(session) = Session::restore(connection.snapshot.clone()) else {
            return;
        };
        let Some(workspace) = session.workspace(workspace_id) else {
            return;
        };
        let Some(target) = self
            .open_targets
            .as_deref()
            .and_then(|targets| targets.iter().find(|target| target.id == target_id))
            .cloned()
        else {
            return;
        };
        if self.opening_workspace.is_some() {
            return;
        }
        let root = workspace.root_directory().to_path_buf();
        let project = project_root(workspace).to_path_buf();
        self.opening_workspace = Some((key, workspace_id));
        cx.notify();
        cx.spawn_in(window, async move |this, cx| {
            let result = cx
                .background_spawn({
                    let target = target.clone();
                    let root = root.clone();
                    async move { launch(&target, &root) }
                })
                .await;
            let launched = result.is_ok();
            let _ = this.update_in(cx, |this, window, cx| match result {
                Ok(()) => this.remember_open_target(project, target.id, cx),
                Err(error) => window.push_notification(
                    Notification::error(format!(
                        "Couldn't open {} in {}: {error}",
                        root.display(),
                        target.label
                    )),
                    cx,
                ),
            });
            // The error notification is feedback enough; only a launch that went quiet
            // keeps the spinner up.
            if launched {
                cx.background_executor().timer(OPEN_IN_FEEDBACK).await;
            }
            let _ = this.update(cx, |this, cx| {
                this.opening_workspace = None;
                cx.notify();
            });
        })
        .detach();
    }

    fn remember_open_target(&mut self, project: PathBuf, id: String, cx: &mut Context<Self>) {
        let changed_default = self.default_editor.as_deref() != Some(id.as_str());
        let changed_project = self.workspace_editors.get(&project) != Some(&id);
        if !changed_default && !changed_project {
            return;
        }
        self.default_editor = Some(id.clone());
        self.workspace_editors.insert(project, id);
        self.save_open_in_choices(cx);
        cx.notify();
    }
}

#[cfg(test)]
mod tests {
    // Named imports: a glob would pull in GPUI's `test` attribute over std's.
    use super::{
        CustomEditor, FILE_MANAGER_ID, InstallRoots, OpenTarget, OpenTargetIcon, Platform,
        available_targets, launch, target,
    };
    use std::ffi::OsString;
    use std::fs;
    use std::io;
    use std::path::PathBuf;
    use std::thread;

    struct Sandbox(PathBuf);

    impl Sandbox {
        fn new(name: &str) -> Self {
            let root = std::env::temp_dir().join(format!(
                "condr-open-in-{name}-{}-{:?}",
                std::process::id(),
                thread::current().id()
            ));
            let _ = fs::remove_dir_all(&root);
            fs::create_dir_all(&root).unwrap();
            Self(root)
        }

        fn file(&self, relative: &str) -> PathBuf {
            let path = self.0.join(relative);
            fs::create_dir_all(path.parent().unwrap()).unwrap();
            fs::write(&path, b"").unwrap();
            path
        }

        fn directory(&self, relative: &str) -> PathBuf {
            let path = self.0.join(relative);
            fs::create_dir_all(&path).unwrap();
            path
        }
    }

    impl Drop for Sandbox {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    fn ids(targets: &[OpenTarget]) -> Vec<&str> {
        targets.iter().map(|target| target.id.as_str()).collect()
    }

    #[test]
    fn nothing_installed_still_offers_the_file_manager() {
        for (platform, label) in [
            (Platform::Windows, "Explorer"),
            (Platform::MacOs, "Finder"),
            (Platform::Linux, "Files"),
        ] {
            let roots = InstallRoots {
                platform: Some(platform),
                ..InstallRoots::default()
            };
            let targets = available_targets(&roots, &[]);
            assert_eq!(ids(&targets), [FILE_MANAGER_ID]);
            assert_eq!(targets[0].label, label);
            assert_eq!(targets[0].icon, OpenTargetIcon::FileManager);
        }
    }

    #[test]
    fn windows_presets_prefer_the_installed_exe_over_the_path_shim() {
        let sandbox = Sandbox::new("windows");
        let local = sandbox.directory("Local");
        let programs = sandbox.directory("Program Files");
        let bin = sandbox.directory("bin");
        let code_exe = sandbox.file("Local/Programs/Microsoft VS Code/Code.exe");
        sandbox.file("bin/code.cmd");
        let zed_exe = sandbox.file("Local/Programs/Zed/Zed.exe");
        let cursor_exe = sandbox.file("Program Files/Cursor/Cursor.exe");
        // Both editions installed: the Ultimate edition wins, whatever its version.
        sandbox
            .file("Program Files/JetBrains/IntelliJ IDEA Community Edition 2025.1/bin/idea64.exe");
        let idea_exe =
            sandbox.file("Program Files/JetBrains/IntelliJ IDEA 2024.3.2/bin/idea64.exe");
        let roots = InstallRoots {
            platform: Some(Platform::Windows),
            path: vec![bin],
            local_app_data: Some(local),
            program_files: Some(programs),
            ..InstallRoots::default()
        };

        let targets = available_targets(&roots, &[]);

        assert_eq!(
            ids(&targets),
            ["zed", "vscode", "cursor", "idea", FILE_MANAGER_ID]
        );
        assert_eq!(targets[0].program, zed_exe);
        assert_eq!(targets[1].program, code_exe);
        assert_eq!(targets[2].program, cursor_exe);
        assert_eq!(targets[2].args, [OsString::from("--new-window")]);
        assert_eq!(targets[3].program, idea_exe);
        assert_eq!(targets[4].program, PathBuf::from("explorer.exe"));
    }

    #[test]
    fn windows_falls_back_to_path_shims_and_the_toolbox_script() {
        let sandbox = Sandbox::new("windows-path");
        let bin = sandbox.directory("bin");
        let code_cmd = sandbox.file("bin/code.cmd");
        let idea_cmd = sandbox.file("Local/JetBrains/Toolbox/scripts/idea.cmd");
        let roots = InstallRoots {
            platform: Some(Platform::Windows),
            path: vec![bin],
            local_app_data: Some(sandbox.0.join("Local")),
            ..InstallRoots::default()
        };

        let targets = available_targets(&roots, &[]);

        assert_eq!(ids(&targets), ["vscode", "idea", FILE_MANAGER_ID]);
        assert_eq!(targets[0].program, code_cmd);
        assert_eq!(targets[1].program, idea_cmd);
    }

    #[test]
    fn macos_uses_the_cli_when_present_and_open_otherwise() {
        let sandbox = Sandbox::new("macos");
        let bin = sandbox.directory("bin");
        let zed_cli = sandbox.file("bin/zed");
        sandbox.directory("Applications/Visual Studio Code.app");
        sandbox.directory("Applications/IntelliJ IDEA CE.app");
        let roots = InstallRoots {
            platform: Some(Platform::MacOs),
            path: vec![bin],
            applications: vec![sandbox.0.join("Applications")],
            ..InstallRoots::default()
        };

        let targets = available_targets(&roots, &[]);

        assert_eq!(ids(&targets), ["zed", "vscode", "idea", FILE_MANAGER_ID]);
        assert_eq!(targets[0].program, zed_cli);
        assert_eq!(targets[1].program, PathBuf::from("/usr/bin/open"));
        assert_eq!(
            targets[1].args,
            ["-n", "-a", "Visual Studio Code", "--args"].map(OsString::from)
        );
        assert_eq!(
            targets[2].args,
            ["-n", "-a", "IntelliJ IDEA CE", "--args"].map(OsString::from)
        );
        assert_eq!(targets[3].program, PathBuf::from("/usr/bin/open"));
    }

    #[test]
    fn linux_probes_the_path_and_the_toolbox_script_only() {
        let sandbox = Sandbox::new("linux");
        let bin = sandbox.directory("bin");
        let code = sandbox.file("bin/code");
        let idea = sandbox.file("home/.local/share/JetBrains/Toolbox/scripts/idea");
        let roots = InstallRoots {
            platform: Some(Platform::Linux),
            path: vec![bin],
            home: Some(sandbox.0.join("home")),
            ..InstallRoots::default()
        };

        let targets = available_targets(&roots, &[]);

        assert_eq!(ids(&targets), ["vscode", "idea", FILE_MANAGER_ID]);
        assert_eq!(targets[0].program, code);
        assert_eq!(targets[1].program, idea);
        assert_eq!(targets[2].program, PathBuf::from("xdg-open"));
    }

    #[test]
    fn custom_editors_follow_the_presets_and_may_replace_one() {
        let sandbox = Sandbox::new("custom");
        let bin = sandbox.directory("bin");
        sandbox.file("bin/code");
        let roots = InstallRoots {
            platform: Some(Platform::Linux),
            path: vec![bin],
            ..InstallRoots::default()
        };
        let custom = [
            CustomEditor {
                name: "Helix".into(),
                command: vec!["wezterm".into(), "start".into(), "hx".into()],
            },
            CustomEditor {
                name: "vscode".into(),
                command: vec!["code-insiders".into()],
            },
            CustomEditor {
                name: "  ".into(),
                command: vec!["ignored".into()],
            },
            CustomEditor {
                name: "No command".into(),
                command: Vec::new(),
            },
        ];

        let targets = available_targets(&roots, &custom);

        assert_eq!(ids(&targets), ["Helix", "vscode", FILE_MANAGER_ID]);
        assert_eq!(targets[0].icon, OpenTargetIcon::Custom);
        assert_eq!(targets[0].program, PathBuf::from("wezterm"));
        assert_eq!(targets[0].args, ["start", "hx"].map(OsString::from));
        assert_eq!(targets[1].program, PathBuf::from("code-insiders"));
    }

    #[test]
    fn launching_into_a_missing_directory_fails_before_spawning() {
        let sandbox = Sandbox::new("launch");
        let target = target(
            "x",
            "X",
            OpenTargetIcon::Custom,
            PathBuf::from("condr-no-such-program"),
            &[],
        );
        let error = launch(&target, &sandbox.0.join("gone")).unwrap_err();
        assert_eq!(error.kind(), io::ErrorKind::NotFound);
        assert!(error.to_string().contains("does not exist"), "{error}");
        let error = launch(&target, &sandbox.0).unwrap_err();
        assert_eq!(error.kind(), io::ErrorKind::NotFound);
    }
}
