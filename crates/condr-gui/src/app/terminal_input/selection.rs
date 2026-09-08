use super::*;

impl Condr {
    pub(crate) fn begin_selection(
        &mut self,
        key: ConnectionKey,
        pane_id: PaneId,
        position: TerminalPosition,
        click_count: usize,
        cx: &mut Context<Self>,
    ) {
        let Some(terminal) = self.terminal(key, pane_id) else {
            return;
        };
        let display_offset = terminal.view.display_offset;
        let server_selection = terminal.view.selection.is_some();
        let unit = match click_count {
            2 => Some(TerminalSelectionUnit::Word),
            3.. => Some(TerminalSelectionUnit::Line),
            _ => None,
        };
        self.terminal_selection = Some(LocalTerminalSelection {
            connection_key: key,
            pane_id,
            range: TerminalSelection {
                start: position,
                end: position,
                display_offset,
            },
            dragging: click_count == 1,
            committed: false,
        });
        // A single click clears the Server-tracked selection (when there is one to clear).
        // A word or line is the Server's to expand: it follows soft wraps and brackets, and
        // the next frame brings it back as the tracked selection.
        let command = match unit {
            Some(unit) => Some(TerminalCommand::SelectAt {
                position,
                display_offset,
                unit,
            }),
            None if server_selection => Some(TerminalCommand::Select(None)),
            None => None,
        };
        if let Some(command) = command {
            let sent = self.terminal_command(key, pane_id, command);
            if sent && let Some(selection) = &mut self.terminal_selection {
                selection.committed = unit.is_some();
            }
        }
        cx.notify();
    }

    pub(crate) fn update_selection(
        &mut self,
        key: ConnectionKey,
        pane_id: PaneId,
        position: TerminalPosition,
        cx: &mut Context<Self>,
    ) {
        if let Some(selection) = &mut self.terminal_selection
            && selection.dragging
            && selection.connection_key == key
            && selection.pane_id == pane_id
            && selection.range.end != position
        {
            selection.range.end = position;
            cx.notify();
        }
    }

    pub(crate) fn is_selecting(&self, key: ConnectionKey, pane_id: PaneId) -> bool {
        self.terminal_selection.is_some_and(|selection| {
            selection.dragging && selection.connection_key == key && selection.pane_id == pane_id
        })
    }

    pub(crate) fn end_selection(
        &mut self,
        key: ConnectionKey,
        pane_id: PaneId,
        position: TerminalPosition,
        cx: &mut Context<Self>,
    ) {
        self.update_selection(key, pane_id, position, cx);
        if let Some(selection) = &mut self.terminal_selection
            && selection.connection_key == key
            && selection.pane_id == pane_id
            && selection.dragging
        {
            selection.dragging = false;
            let range = selection.range;
            let columns = self
                .terminal(key, pane_id)
                .map_or(0, |terminal| terminal.view.size.columns);
            // The Server keeps the finished drag attached to its text; the local copy
            // stays as the fallback for a Client that may not mutate.
            let sent = self.terminal_command(
                key,
                pane_id,
                TerminalCommand::Select(range.selected_cell_range(columns).map(|_| range)),
            );
            if sent && let Some(selection) = &mut self.terminal_selection {
                selection.committed = true;
            }
        }
    }

    /// The selection to draw and copy: the local one while dragging, otherwise the
    /// Server-tracked one, which follows the text as output scrolls.
    pub(in crate::app) fn selection_for(
        &self,
        key: ConnectionKey,
        pane_id: PaneId,
    ) -> Option<TerminalSelection> {
        self.effective_selection(key, pane_id)
            .map(|(selection, _)| selection)
    }

    /// The selection and whether the Server tracks it.
    pub(super) fn effective_selection(
        &self,
        key: ConnectionKey,
        pane_id: PaneId,
    ) -> Option<(TerminalSelection, bool)> {
        let local = self
            .terminal_selection
            .filter(|selection| selection.connection_key == key && selection.pane_id == pane_id);
        // A drag in progress, or a selection the Server never received (a Client that
        // cannot mutate), is local; a committed one only bridges until the next frame.
        if let Some(local) = local.filter(|selection| selection.dragging || !selection.committed) {
            return Some((local.range, false));
        }
        self.terminal(key, pane_id)
            .and_then(|terminal| terminal.view.selection)
            .map(|selection| (selection, true))
            .or(local.map(|local| (local.range, false)))
    }

    pub(in crate::app) fn clear_selection(&mut self, cx: &mut Context<Self>) {
        if self.terminal_selection.take().is_some() {
            cx.notify();
        }
    }
}

#[derive(Clone, Copy)]
pub(in crate::app) struct LocalTerminalSelection {
    pub(in crate::app) connection_key: ConnectionKey,
    pub(in crate::app) pane_id: PaneId,
    pub(in crate::app) range: TerminalSelection,
    pub(in crate::app) dragging: bool,
    /// Sent to the Server; the next frame for this Pane replaces it with the Server's.
    pub(in crate::app) committed: bool,
}
