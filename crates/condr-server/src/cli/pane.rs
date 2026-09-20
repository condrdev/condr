use super::*;

/// How many rows `pane read` returns by default, as herdr does.
const DEFAULT_READ_LINES: u32 = 80;

#[derive(Subcommand)]
pub(crate) enum PaneCommand {
    /// List Panes, in one Workspace or across all of them
    List {
        #[arg(long, value_name = "ID")]
        workspace: Option<u64>,
    },
    /// Show one Pane
    Get { pane_id: u64 },
    /// Show the Pane this command runs in (`CONDR_PANE_ID`)
    Current,
    /// Describe a Pane's Tab: the split tree, every Pane's edges, and the Pane's neighbors
    Layout {
        /// Defaults to the calling Pane
        pane_id: Option<u64>,
    },
    /// Split a Pane; the new Pane opens a shell in the same directory
    Split {
        /// Defaults to the calling Pane
        pane_id: Option<u64>,
        #[arg(long, value_enum)]
        direction: SplitSide,
        /// Focus the new Pane instead of leaving the GUI where it was
        #[arg(long)]
        focus: bool,
    },
    /// Focus a Pane by id and ask every GUI to show its Tab, or focus the neighbor of a
    /// Pane with --direction
    Focus {
        /// Defaults to the calling Pane when --direction is given
        pane_id: Option<u64>,
        #[arg(long, value_enum)]
        direction: Option<Side>,
    },
    /// Move a Pane's edge; the amount is a fraction of the split
    Resize {
        /// Defaults to the calling Pane
        pane_id: Option<u64>,
        #[arg(long, value_enum)]
        direction: Side,
        #[arg(long, default_value_t = 0.05)]
        amount: f32,
    },
    /// Swap a Pane with its neighbor
    Swap {
        /// Defaults to the calling Pane
        pane_id: Option<u64>,
        #[arg(long, value_enum)]
        direction: Side,
    },
    /// Detach a Pane and reattach it beside another Pane in the same Tab
    Move {
        /// Defaults to the calling Pane
        pane_id: Option<u64>,
        /// The Pane to attach beside
        #[arg(long)]
        to: u64,
        /// Which side of --to the moved Pane lands on
        #[arg(long, value_enum)]
        side: Side,
    },
    /// Zoom a Pane to fill its Tab, or back; toggles unless --on or --off is given
    Zoom {
        /// Defaults to the calling Pane
        pane_id: Option<u64>,
        #[arg(long, conflicts_with = "off")]
        on: bool,
        #[arg(long)]
        off: bool,
    },
    /// Close a Pane and stop its terminal; the last Pane closes its Tab
    Close { pane_id: u64 },
    /// Print the last rows of a Pane's terminal as plain text, scrollback included
    Read {
        pane_id: u64,
        #[arg(long, value_name = "N", default_value_t = DEFAULT_READ_LINES)]
        lines: u32,
    },
    /// Type text into a Pane exactly as given, without Enter
    SendText { pane_id: u64, text: String },
    /// Press keys in a Pane: `enter`, `esc`, `tab`, `up`, `f5`, `ctrl+c`, `alt+shift+x`, `a`…
    SendKeys {
        pane_id: u64,
        #[arg(required = true)]
        keys: Vec<String>,
    },
    /// Paste a command into a Pane and press Enter
    Run { pane_id: u64, command: String },
}

#[derive(Clone, Copy, ValueEnum)]
pub(crate) enum SplitSide {
    /// The new Pane appears to the right
    Right,
    /// The new Pane appears below
    Down,
}

impl From<SplitSide> for SplitDirection {
    fn from(side: SplitSide) -> Self {
        match side {
            SplitSide::Right => Self::Horizontal,
            SplitSide::Down => Self::Vertical,
        }
    }
}

#[derive(Clone, Copy, ValueEnum)]
pub(crate) enum Side {
    Left,
    Right,
    Up,
    Down,
}

impl From<Side> for PaneDirection {
    fn from(side: Side) -> Self {
        match side {
            Side::Left => Self::Left,
            Side::Right => Self::Right,
            Side::Up => Self::Up,
            Side::Down => Self::Down,
        }
    }
}

pub(crate) fn run_pane(device: Option<&str>, command: PaneCommand) -> i32 {
    // `read` prints the text itself so agents can consume it without a JSON step.
    if let PaneCommand::Read { pane_id, lines } = command {
        return match connect(device).and_then(|mut client| {
            find_pane(&client.session()?, pane_id)?;
            Ok(client.read_pane(PaneId::from_u64(pane_id), lines)?)
        }) {
            Ok(text) => {
                if !text.is_empty() {
                    println!("{text}");
                }
                0
            }
            Err(error) => {
                eprintln!("{}", json!({ "error": error }));
                1
            }
        };
    }
    run(device, |client| pane(client, device, command))
}

#[derive(Serialize)]
pub(super) struct PaneInfo {
    pub(super) pane_id: u64,
    pub(super) tab_id: u64,
    pub(super) workspace_id: u64,
    /// The focused Pane of its Tab.
    pub(super) focused: bool,
    /// Fills its Tab, hiding the other Panes.
    pub(super) zoomed: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(super) cwd: Option<PathBuf>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(super) title: Option<String>,
    pub(super) exited: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(super) agent: Option<&'static str>,
    /// `idle`, `working`, `blocked`, or `unknown` for a plain shell.
    pub(super) agent_status: &'static str,
}

pub(super) fn pane_info(
    client: &ClientConnection,
    workspace: &Workspace,
    tab: &Tab,
    pane_id: PaneId,
) -> PaneInfo {
    let overview = client.overview();
    let terminal = overview
        .terminals
        .iter()
        .find(|terminal| terminal.pane_id == pane_id);
    let agent = overview
        .agents
        .iter()
        .find(|agent| agent.pane_id == pane_id)
        .map(|agent| &agent.agent);
    PaneInfo {
        pane_id: pane_id.as_u64(),
        tab_id: tab.id().as_u64(),
        workspace_id: workspace.id().as_u64(),
        focused: tab.focused_pane().is_some_and(|pane| pane.id() == pane_id),
        // Zoom is live state, not part of the structural Snapshot (ADR 0005).
        zoomed: overview.zoomed_panes.contains(&pane_id),
        cwd: tab
            .panes()
            .iter()
            .find(|pane| pane.id() == pane_id)
            .and_then(|pane| pane.cwd())
            .map(Path::to_path_buf),
        title: terminal.and_then(|terminal| terminal.title.clone()),
        exited: terminal.is_some_and(|terminal| terminal.exited),
        agent: agent.map(|agent| agent.kind.id()),
        agent_status: match agent.map(|agent| agent.state) {
            Some(AgentState::Idle) => "idle",
            Some(AgentState::Working) => "working",
            Some(AgentState::Blocked) => "blocked",
            Some(AgentState::Unknown) | None => "unknown",
        },
    }
}

pub(super) fn find_pane(session: &Session, id: u64) -> Result<(&Workspace, &Tab), CliError> {
    let pane_id = PaneId::from_u64(id);
    session
        .workspaces()
        .iter()
        .find_map(|workspace| {
            workspace
                .tabs()
                .iter()
                .find(|tab| tab.panes().iter().any(|pane| pane.id() == pane_id))
                .map(|tab| (workspace, tab))
        })
        .ok_or_else(|| CliError::new("pane_not_found", format!("pane {id} not found")))
}

/// The Pane this process runs in, or a usage-style error when there is none. With
/// `--device` there never is one: `CONDR_PANE_ID` names a Pane of this machine's Server,
/// and the Device numbers its own Panes from 1 too, so the id would silently hit a
/// stranger.
pub(super) fn caller_pane(device: Option<&str>) -> Result<u64, CliError> {
    if let Some(device) = device {
        return Err(CliError::new(
            "no_current_pane",
            format!("the calling Pane is on this machine, not on Device {device}; pass a pane id"),
        ));
    }
    std::env::var(PaneEnvironment::PANE_ID)
        .ok()
        .and_then(|value| value.trim().parse().ok())
        .ok_or_else(|| {
            CliError::new(
                "no_current_pane",
                "CONDR_PANE_ID is not set: this is not a Condr Pane, pass a pane id",
            )
        })
}

/// `ctrl+shift+x`, `alt+enter`, `f5`, `esc`, `a`. Modifier names: ctrl/control, alt/opt,
/// shift, super/cmd/win.
fn parse_key(spec: &str) -> Result<TerminalCommand, CliError> {
    let invalid = || CliError::new("invalid_key", format!("unsupported key {spec}"));
    let mut modifiers = TerminalModifiers::default();
    let mut parts = spec.split('+').collect::<Vec<_>>();
    // A trailing "+" is the plus key itself.
    if spec.ends_with('+') {
        parts.pop();
        parts.pop();
        parts.push("+");
    }
    let key = parts.pop().ok_or_else(invalid)?;
    for modifier in parts {
        match modifier.to_ascii_lowercase().as_str() {
            "ctrl" | "control" => modifiers.control = true,
            "alt" | "opt" | "option" | "meta" => modifiers.alt = true,
            "shift" => modifiers.shift = true,
            "super" | "cmd" | "win" => modifiers.platform = true,
            _ => return Err(invalid()),
        }
    }
    let key = match key.to_ascii_lowercase().as_str() {
        "enter" | "return" => TerminalKey::Enter,
        "tab" => TerminalKey::Tab,
        "backtab" => TerminalKey::BackTab,
        "backspace" => TerminalKey::Backspace,
        "delete" | "del" => TerminalKey::Delete,
        "esc" | "escape" => TerminalKey::Escape,
        "up" => TerminalKey::Up,
        "down" => TerminalKey::Down,
        "left" => TerminalKey::Left,
        "right" => TerminalKey::Right,
        "home" => TerminalKey::Home,
        "end" => TerminalKey::End,
        "pageup" => TerminalKey::PageUp,
        "pagedown" => TerminalKey::PageDown,
        "insert" => TerminalKey::Insert,
        "space" => TerminalKey::Character(" ".into()),
        lower => {
            if let Some(number) = lower
                .strip_prefix('f')
                .and_then(|digits| digits.parse::<u8>().ok())
                .filter(|number| (1..=20).contains(number))
            {
                TerminalKey::Function(number)
            } else if key.chars().count() == 1 {
                TerminalKey::Character(key.into())
            } else {
                return Err(invalid());
            }
        }
    };
    Ok(TerminalCommand::Key { key, modifiers })
}

fn pane(
    client: &mut ClientConnection,
    device: Option<&str>,
    command: PaneCommand,
) -> Result<Value, CliError> {
    match command {
        PaneCommand::List { workspace } => {
            let client = &*client;
            let session = client.session()?;
            let workspaces: Vec<&Workspace> = match workspace {
                Some(id) => vec![find_workspace(&session, id)?],
                None => session.workspaces().iter().collect(),
            };
            let panes: Vec<_> = workspaces
                .iter()
                .flat_map(|workspace| {
                    workspace.tabs().iter().flat_map(move |tab| {
                        tab.panes()
                            .iter()
                            .map(move |pane| pane_info(client, workspace, tab, pane.id()))
                    })
                })
                .collect();
            Ok(json!({ "panes": panes }))
        }
        PaneCommand::Current => {
            let pane_id = caller_pane(device)?;
            let session = client.session()?;
            let (workspace, tab) = find_pane(&session, pane_id)?;
            Ok(json!({ "pane": pane_info(client, workspace, tab, PaneId::from_u64(pane_id)) }))
        }
        PaneCommand::Get { pane_id } => {
            let session = client.session()?;
            let (workspace, tab) = find_pane(&session, pane_id)?;
            Ok(json!({ "pane": pane_info(client, workspace, tab, PaneId::from_u64(pane_id)) }))
        }
        PaneCommand::Split {
            pane_id,
            direction,
            focus,
        } => {
            let pane_id = match pane_id {
                Some(id) => id,
                None => caller_pane(device)?,
            };
            find_pane(&client.session()?, pane_id)?;
            let LayoutResult::PaneCreated { pane_id: new_pane } = apply(
                client,
                LayoutCommand::SplitPane {
                    pane_id: PaneId::from_u64(pane_id),
                    direction: direction.into(),
                    focus,
                },
            )?
            else {
                return Err(CliError::new(
                    "pane_split_failed",
                    "the Server returned no created Pane ID",
                ));
            };
            let session = client.session()?;
            let (workspace, tab) = find_pane(&session, new_pane.as_u64())?;
            Ok(json!({ "pane": pane_info(client, workspace, tab, new_pane) }))
        }
        PaneCommand::Focus {
            pane_id,
            direction: None,
        } => {
            let pane_id = target_pane(device, pane_id)?;
            let before = client.session()?;
            let tab_id = find_pane(&before, pane_id.as_u64())?.1.id();
            apply(client, LayoutCommand::FocusPane { pane_id })?;
            // Pane focus is Session structure; showing the Tab is each viewer's own choice,
            // so ask them (ADR 0021).
            apply(client, LayoutCommand::ActivateTab { tab_id })?;
            let session = client.session()?;
            let (workspace, tab) = find_pane(&session, pane_id.as_u64())?;
            Ok(json!({ "pane": pane_info(client, workspace, tab, pane_id) }))
        }
        PaneCommand::Focus {
            pane_id,
            direction: Some(direction),
        } => {
            let pane_id = target_pane(device, pane_id)?;
            let (changed, tab_id) = arrange(
                client,
                pane_id,
                LayoutCommand::FocusPaneDirection {
                    pane_id,
                    direction: direction.into(),
                },
            )?;
            // The result is whichever Pane holds the focus now.
            let session = client.session()?;
            let tab = session
                .tab(tab_id)
                .ok_or_else(|| CliError::new("pane_not_found", "the Tab vanished"))?;
            let focused = tab
                .focused_pane()
                .ok_or_else(|| CliError::new("pane_not_found", "the Tab shows no Panes"))?
                .id();
            let (workspace, tab) = find_pane(&session, focused.as_u64())?;
            Ok(json!({
                "pane": pane_info(client, workspace, tab, focused),
                "changed": changed,
            }))
        }
        PaneCommand::Resize {
            pane_id,
            direction,
            amount,
        } => {
            let pane_id = target_pane(device, pane_id)?;
            let (changed, _) = arrange(
                client,
                pane_id,
                LayoutCommand::ResizePane {
                    pane_id,
                    direction: direction.into(),
                    amount,
                },
            )?;
            let session = client.session()?;
            let (workspace, tab) = find_pane(&session, pane_id.as_u64())?;
            Ok(json!({
                "pane": pane_info(client, workspace, tab, pane_id),
                "changed": changed,
            }))
        }
        PaneCommand::Swap { pane_id, direction } => {
            let pane_id = target_pane(device, pane_id)?;
            let (changed, _) = arrange(
                client,
                pane_id,
                LayoutCommand::SwapPane {
                    pane_id,
                    direction: direction.into(),
                },
            )?;
            let session = client.session()?;
            let (workspace, tab) = find_pane(&session, pane_id.as_u64())?;
            Ok(json!({
                "pane": pane_info(client, workspace, tab, pane_id),
                "changed": changed,
            }))
        }
        PaneCommand::Move { pane_id, to, side } => {
            let pane_id = target_pane(device, pane_id)?;
            let (changed, _) = arrange(
                client,
                pane_id,
                LayoutCommand::MovePane {
                    pane_id,
                    target_pane_id: PaneId::from_u64(to),
                    side: side.into(),
                },
            )?;
            let session = client.session()?;
            let (workspace, tab) = find_pane(&session, pane_id.as_u64())?;
            Ok(json!({
                "pane": pane_info(client, workspace, tab, pane_id),
                "changed": changed,
            }))
        }
        PaneCommand::Zoom { pane_id, on, off } => {
            let pane_id = target_pane(device, pane_id)?;
            find_pane(&client.session()?, pane_id.as_u64())?;
            let zoomed = client.overview().zoomed_panes.contains(&pane_id);
            // The protocol only toggles; --on and --off skip the toggle when already there.
            let changed = !(on && zoomed || off && !zoomed);
            if changed {
                apply(client, LayoutCommand::TogglePaneZoom { pane_id })?;
            }
            let session = client.session()?;
            let (workspace, tab) = find_pane(&session, pane_id.as_u64())?;
            Ok(json!({
                "pane": pane_info(client, workspace, tab, pane_id),
                "changed": changed,
            }))
        }
        PaneCommand::Layout { pane_id } => {
            let pane_id = target_pane(device, pane_id)?;
            let session = client.session()?;
            let (workspace, tab) = find_pane(&session, pane_id.as_u64())?;
            let rects = tab.pane_rects();
            let edges = rects.iter().find(|rect| rect.id == pane_id).map(|rect| {
                json!({
                    "left": rect.left, "top": rect.top, "right": rect.right, "bottom": rect.bottom,
                })
            });
            let neighbor = |direction| tab.neighbor(pane_id, direction).map(PaneId::as_u64);
            Ok(json!({
                "workspace_id": workspace.id().as_u64(),
                "tab_id": tab.id().as_u64(),
                "focused_pane_id": tab.focused_pane().map(|pane| pane.id().as_u64()),
                "zoomed_pane_id": client.overview().zoomed_panes.iter()
                    .find(|zoomed| tab.panes().iter().any(|pane| pane.id() == **zoomed))
                    .map(|zoomed| zoomed.as_u64()),
                "pane": {
                    "pane_id": pane_id.as_u64(),
                    "edges": edges,
                    "neighbors": {
                        "left": neighbor(PaneDirection::Left),
                        "right": neighbor(PaneDirection::Right),
                        "up": neighbor(PaneDirection::Up),
                        "down": neighbor(PaneDirection::Down),
                    },
                },
                "panes": rects.iter().map(|rect| json!({
                    "pane_id": rect.id.as_u64(),
                    "left": rect.left, "top": rect.top, "right": rect.right, "bottom": rect.bottom,
                })).collect::<Vec<_>>(),
                "tree": tab.layout().map(layout_tree),
            }))
        }
        PaneCommand::Close { pane_id } => {
            find_pane(&client.session()?, pane_id)?;
            apply(
                client,
                LayoutCommand::ClosePane {
                    pane_id: PaneId::from_u64(pane_id),
                },
            )?;
            Ok(json!({ "ok": true }))
        }
        PaneCommand::Read { .. } => unreachable!("read prints text, see run_pane"),
        PaneCommand::SendText { pane_id, text } => {
            find_pane(&client.session()?, pane_id)?;
            send(client, pane_id, [TerminalCommand::Text(text)])?;
            Ok(json!({ "ok": true }))
        }
        PaneCommand::SendKeys { pane_id, keys } => {
            find_pane(&client.session()?, pane_id)?;
            // Every key is validated before anything is typed.
            let commands = keys
                .iter()
                .map(|key| parse_key(key))
                .collect::<Result<Vec<_>, _>>()?;
            send(client, pane_id, commands)?;
            Ok(json!({ "ok": true }))
        }
        PaneCommand::Run { pane_id, command } => {
            find_pane(&client.session()?, pane_id)?;
            // Paste, so a program with bracketed paste on sees one unit of text, then Enter.
            send(
                client,
                pane_id,
                [
                    TerminalCommand::Paste(command),
                    TerminalCommand::Key {
                        key: TerminalKey::Enter,
                        modifiers: TerminalModifiers::default(),
                    },
                ],
            )?;
            Ok(json!({ "ok": true }))
        }
    }
}

/// The split tree as JSON: `{"pane": id}` leaves under `{"split": "horizontal"|"vertical",
/// "ratio": r, "first": …, "second": …}` nodes; `first` is the left or top side.
fn layout_tree(layout: &PaneLayout) -> Value {
    match layout {
        PaneLayout::Pane(id) => json!({ "pane": id.as_u64() }),
        PaneLayout::Split {
            direction,
            ratio,
            first,
            second,
        } => json!({
            "split": match direction {
                SplitDirection::Horizontal => "horizontal",
                SplitDirection::Vertical => "vertical",
            },
            "ratio": ratio,
            "first": layout_tree(first),
            "second": layout_tree(second),
        }),
    }
}

/// An explicit Pane id, or the calling Pane.
fn target_pane(device: Option<&str>, pane_id: Option<u64>) -> Result<PaneId, CliError> {
    Ok(PaneId::from_u64(match pane_id {
        Some(id) => id,
        None => caller_pane(device)?,
    }))
}

/// Applies a rearrangement inside a Pane's Tab and reports whether the Tab changed at
/// all: the Server accepts a resize or swap with no neighbor as a no-op.
fn arrange(
    client: &mut ClientConnection,
    pane_id: PaneId,
    command: LayoutCommand,
) -> Result<(bool, TabId), CliError> {
    let before = client.session()?;
    let (_, tab) = find_pane(&before, pane_id.as_u64())?;
    let tab_id = tab.id();
    let previous = (
        tab.layout().cloned(),
        tab.focused_pane().map(|pane| pane.id()),
    );
    apply(client, command)?;
    let after = client.session()?;
    let tab = after
        .tab(tab_id)
        .ok_or_else(|| CliError::new("pane_not_found", "the Tab vanished"))?;
    let changed = (
        tab.layout().cloned(),
        tab.focused_pane().map(|pane| pane.id()),
    ) != previous;
    Ok((changed, tab_id))
}

pub(super) fn send(
    client: &mut ClientConnection,
    pane_id: u64,
    commands: impl IntoIterator<Item = TerminalCommand>,
) -> Result<(), CliError> {
    for command in commands {
        client
            .terminal(PaneId::from_u64(pane_id), command)
            .map_err(|error| CliError::new("pane_send_failed", error.to_string()))?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_calling_pane_never_stands_in_for_a_pane_on_a_device() {
        // Whatever CONDR_PANE_ID says, that id belongs to this machine's Server.
        let error = caller_pane(Some("lab")).unwrap_err();
        assert_eq!(error.code, "no_current_pane");
        assert!(error.message.contains("Device lab"), "{}", error.message);
    }

    #[test]
    fn function_keys_are_validated_within_the_terminal_encoders_range() {
        for number in 1..=20 {
            assert!(matches!(
                parse_key(&format!("ctrl+f{number}")),
                Ok(TerminalCommand::Key { key: TerminalKey::Function(n), .. }) if n == number
            ));
        }
        for number in [0, 21, 22, 23, 24, 255] {
            let keys = ["a".to_owned(), format!("f{number}")];
            assert!(
                keys.iter()
                    .map(|key| parse_key(key))
                    .collect::<Result<Vec<_>, _>>()
                    .is_err()
            );
        }
    }
}
