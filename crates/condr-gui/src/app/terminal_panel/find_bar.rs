//! The find bar over a Pane's top right (ADR 0077). The search is the Pane's, on the
//! Server; the bar only edits its needle and steps through its matches.

use super::*;
use condr_core::{TerminalSearch, TerminalSearchStep};
use gpui_kit::component::input::{Escape, Input, InputEvent, InputState};

pub(super) struct FindBar {
    input: Entity<InputState>,
    _events: Subscription,
}

impl TerminalPanel {
    /// Opens the bar with the keyboard in its field and the field's text selected.
    pub(super) fn open_find(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let input = self.find_bar(window, cx).input.clone();
        input.update(cx, |input, cx| {
            input.focus(window, cx);
            input.select_all(window, cx);
        });
        cx.notify();
    }

    /// Ends the Pane's search and gives the keyboard back to the terminal.
    fn close_find(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.find = None;
        self.send_search(TerminalCommand::Search(String::new()), cx);
        self.focus_handle.focus(window, cx);
        cx.notify();
    }

    fn find_bar(&mut self, window: &mut Window, cx: &mut Context<Self>) -> &FindBar {
        if self.find.is_none() {
            let input = cx
                .new(|cx| InputState::new(window, cx).placeholder(t!("terminal.find.placeholder")));
            let events = cx.subscribe_in(&input, window, Self::find_input_event);
            self.find = Some(FindBar {
                input,
                _events: events,
            });
        }
        self.find.as_ref().expect("made above")
    }

    fn find_input_event(
        &mut self,
        input: &Entity<InputState>,
        event: &InputEvent,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let command = match event {
            InputEvent::Change => TerminalCommand::Search(input.read(cx).value().to_string()),
            // Enter goes up from the prompt, into the history.
            InputEvent::PressEnter { shift, .. } => TerminalCommand::SearchStep(if *shift {
                TerminalSearchStep::Newer
            } else {
                TerminalSearchStep::Older
            }),
            InputEvent::Focus | InputEvent::Blur => return,
        };
        self.send_search(command, cx);
    }

    fn send_search(&self, command: TerminalCommand, cx: &mut App) {
        let (key, pane_id) = (self.connection_key, self.pane_id);
        let _ = self
            .owner
            .update(cx, |app, _| app.terminal_command(key, pane_id, command));
    }

    /// A bar without the keyboard follows the Pane's needle, which another window may set
    /// or end; one with it is the person's, whose typing runs ahead of the frames.
    pub(super) fn follow_search(
        &mut self,
        needle: &str,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.find.as_ref().is_some_and(|bar| {
            bar.input
                .read(cx)
                .focus_handle(cx)
                .contains_focused(window, cx)
        }) {
            return;
        }
        let shown = self
            .find
            .as_ref()
            .map(|bar| bar.input.read(cx).value().to_string());
        match shown {
            None if needle.is_empty() => {}
            // An empty bar was opened here and not typed in yet.
            Some(text) if needle.is_empty() => {
                if !text.is_empty() {
                    self.find = None;
                }
            }
            Some(text) if text == needle => {}
            _ => {
                let input = self.find_bar(window, cx).input.clone();
                let needle = needle.to_owned();
                input.update(cx, |input, cx| input.set_value(needle, window, cx));
            }
        }
    }

    pub(super) fn render_find_bar(
        &self,
        search: &TerminalSearch,
        cx: &mut Context<Self>,
    ) -> Option<AnyElement> {
        let bar = self.find.as_ref()?;
        let pane_id = self.pane_id;
        let count = match_count(search);
        let found = search.total > 0;
        let step = |id: &'static str, icon, label, step| {
            Button::new(id)
                .icon(icon)
                .xsmall()
                .ghost()
                .tab_stop(false)
                .disabled(!found)
                .tooltip(label)
                .on_click(cx.listener(move |this, _, _, cx| {
                    this.send_search(TerminalCommand::SearchStep(step), cx);
                }))
        };
        Some(
            h_flex()
                .id("terminal-find")
                .debug_selector(move || format!("terminal-find-{}", pane_id.as_u64()))
                .key_context("TerminalFind")
                .absolute()
                .top(px(6.))
                // Clear of the scrollbar, which stays while the Pane has a search.
                .right(px(16.))
                .occlude()
                .gap_1()
                .p_1()
                .bg(cx.theme().popover)
                .border_1()
                .border_color(cx.theme().border)
                .rounded(cx.theme().radius)
                .shadow_md()
                .font_family(cx.theme().font_family.clone())
                // The Pane's body takes a press to focus the terminal.
                .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
                .on_action(cx.listener(|this, _: &Escape, window, cx| this.close_find(window, cx)))
                .child(div().w(rems(12.)).child(Input::new(&bar.input).xsmall()))
                .child(
                    div()
                        .min_w(rems(3.))
                        .text_xs()
                        .text_color(cx.theme().muted_foreground)
                        .child(count),
                )
                .child(step(
                    "terminal-find-older",
                    IconName::ChevronUp,
                    t!("terminal.find.older"),
                    TerminalSearchStep::Older,
                ))
                .child(step(
                    "terminal-find-newer",
                    IconName::ChevronDown,
                    t!("terminal.find.newer"),
                    TerminalSearchStep::Newer,
                ))
                .child(
                    Button::new("terminal-find-close")
                        .icon(IconName::Close)
                        .xsmall()
                        .ghost()
                        .tab_stop(false)
                        .tooltip(t!("terminal.find.close"))
                        .on_click(cx.listener(|this, _, window, cx| this.close_find(window, cx))),
                )
                .into_any_element(),
        )
    }
}

impl Condr {
    /// `find`: opens the find bar on the Pane the window's actions apply to.
    pub(in crate::app) fn open_find(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(panel) = self
            .terminal_input
            .target
            .and_then(|target| self.panels.get(&target))
            .cloned()
        else {
            return;
        };
        panel.update(cx, |panel, cx| panel.open_find(window, cx));
    }
}

/// "3/17" with a match selected and "17" without; a "+" while the search has not caught
/// up, since the count still grows; "No results" once it has caught up with none.
fn match_count(search: &TerminalSearch) -> SharedString {
    let more = if search.complete { "" } else { "+" };
    match (search.selected, search.total) {
        (_, 0) if search.complete && search.is_active() => t!("terminal.find.no_results").into(),
        (_, 0) => SharedString::default(),
        (Some(selected), total) => format!("{}/{total}{more}", selected + 1).into(),
        (None, total) => format!("{total}{more}").into(),
    }
}

#[cfg(test)]
mod tests {
    use super::{TerminalSearch, match_count};

    #[test]
    fn the_count_says_which_match_and_whether_more_may_come() {
        let search = |total, selected, complete| TerminalSearch {
            needle: "error".into(),
            total,
            selected,
            complete,
            ..TerminalSearch::default()
        };
        assert_eq!(match_count(&search(17, Some(2), true)), "3/17");
        assert_eq!(match_count(&search(17, Some(2), false)), "3/17+");
        assert_eq!(match_count(&search(17, None, true)), "17");
        assert_eq!(match_count(&search(17, None, false)), "17+");
        assert_eq!(match_count(&search(0, None, false)), "");
        assert_eq!(match_count(&search(0, None, true)), "No results");
    }
}
