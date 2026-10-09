use super::*;

/// How long "Saved" stays beside a field after its value went out.
const SAVED_HINT: Duration = Duration::from_secs(2);

/// A free-text setting. Typing only moves `draft`; leaving the field, pressing Enter or
/// the Save button shown while the draft is unsent commits it. So a half-typed value
/// never reaches the config file or a Server, and the field is never rewritten under
/// the user. `committed` is the last accepted draft (including a staged Listen address),
/// not the normalized value that came back.
pub(in crate::app) struct TextField {
    pub(super) input: Entity<InputState>,
    pub(super) draft: SharedString,
    pub(super) committed: SharedString,
}

impl TextField {
    pub(super) fn dirty(&self) -> bool {
        self.draft != self.committed
    }

    /// Picking another Server replaces the value under the field.
    pub(super) fn reset(&mut self, value: SharedString) {
        self.draft = value.clone();
        self.committed = value;
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(in crate::app) enum TextFieldId {
    FontFamily,
    Shell,
    Listen,
    ProxyUrl,
}

/// A commit may validate a local draft without sending it, or fail before it queues.
pub(super) enum CommitOutcome {
    Invalid,
    Unavailable,
    Staged,
    Queued,
}

impl CommitOutcome {
    pub(super) fn from_queued(queued: bool) -> Self {
        if queued {
            Self::Queued
        } else {
            Self::Unavailable
        }
    }
}

impl SettingsWindow {
    pub(super) fn text_field(
        id: TextFieldId,
        initial: SharedString,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> TextField {
        let input = cx.new(|cx| InputState::new(window, cx).default_value(initial.clone()));
        cx.subscribe(
            &input,
            move |this, input, event: &InputEvent, cx| match event {
                InputEvent::Change => {
                    this.field_mut(id).draft = input.read(cx).value();
                    cx.notify();
                }
                InputEvent::Blur | InputEvent::PressEnter { .. } => this.commit(id, cx),
                InputEvent::Focus => {}
            },
        )
        .detach();
        TextField {
            input,
            draft: initial.clone(),
            committed: initial,
        }
    }

    pub(super) fn field(&self, id: TextFieldId) -> &TextField {
        match id {
            TextFieldId::FontFamily => &self.font_family,
            TextFieldId::Shell => &self.shell,
            TextFieldId::Listen => &self.listen,
            TextFieldId::ProxyUrl => &self.proxy_url,
        }
    }

    pub(super) fn field_mut(&mut self, id: TextFieldId) -> &mut TextField {
        match id {
            TextFieldId::FontFamily => &mut self.font_family,
            TextFieldId::Shell => &mut self.shell,
            TextFieldId::Listen => &mut self.listen,
            TextFieldId::ProxyUrl => &mut self.proxy_url,
        }
    }

    /// Why the draft was refused, beside the field until it is fixed.
    pub(super) fn refusal(&self, id: TextFieldId) -> Option<SharedString> {
        match id {
            TextFieldId::Listen if self.listen_refused => Some("Enter host:port".into()),
            TextFieldId::ProxyUrl => self.proxy_url_refused.clone(),
            _ => None,
        }
    }

    /// Only an accepted draft becomes committed. A disabled listener stages its address
    /// locally; an unavailable Server leaves the draft dirty so it can be retried.
    pub(super) fn commit(&mut self, id: TextFieldId, cx: &mut Context<Self>) {
        let draft = self.field(id).draft.clone();
        let outcome = match id {
            TextFieldId::FontFamily => {
                self.font_draft.family = draft.clone();
                CommitOutcome::from_queued(self.commit_font(cx))
            }
            TextFieldId::Shell => {
                let key = self.selected_server;
                let setting = ServerSetting::Shell(draft.to_string());
                let queued = self
                    .owner
                    .update(cx, |owner, _| owner.set_server_setting(key, setting))
                    .unwrap_or(false);
                CommitOutcome::from_queued(queued)
            }
            TextFieldId::Listen => self.commit_listen(cx),
            TextFieldId::ProxyUrl => self.commit_proxy_url(cx),
        };
        self.saved = None;
        self._saved_clear = None;
        match outcome {
            CommitOutcome::Invalid | CommitOutcome::Unavailable => {}
            CommitOutcome::Staged => self.field_mut(id).committed = draft,
            CommitOutcome::Queued => {
                self.field_mut(id).committed = draft;
                self.saved = Some(id);
                self._saved_clear = Some(cx.spawn(async move |this, cx| {
                    cx.background_executor().timer(SAVED_HINT).await;
                    let _ = this.update(cx, |this, cx| {
                        if this.saved == Some(id) {
                            this.saved = None;
                            cx.notify();
                        }
                    });
                }));
            }
        }
        cx.notify();
    }
}

/// The field with, to its left, Save while the draft is unsent and "Saved" for a moment
/// after; the input itself stays put. Reset All returns the field to `default`.
pub(super) fn text_field_row(
    settings: &Entity<SettingsWindow>,
    id: TextFieldId,
    default: SharedString,
) -> SettingField<SharedString> {
    let render = settings.clone();
    let dirty = settings.clone();
    let reset = settings.clone();
    let default_dirty = default.clone();
    SettingField::render(move |options, window, cx| {
        let (input, draft, unsent, saved, refusal) = {
            let this = render.read(cx);
            let field = this.field(id);
            (
                field.input.clone(),
                field.draft.clone(),
                field.dirty(),
                this.saved == Some(id),
                this.refusal(id),
            )
        };
        if input.read(cx).value() != draft {
            input.update(cx, |input, cx| input.set_value(draft, window, cx));
        }
        let settings = render.clone();
        h_flex()
            .gap_2()
            .items_center()
            .when_some(refusal, |row, refusal| {
                row.child(div().text_sm().text_color(cx.theme().danger).child(refusal))
            })
            .when(unsent, |row| {
                row.child(
                    Button::new(format!("save-{id:?}"))
                        .debug_selector(move || format!("settings-save-{id:?}"))
                        .label("Save")
                        .small()
                        .outline()
                        .on_click(move |_, _, cx| {
                            settings.update(cx, |this, cx| this.commit(id, cx));
                        }),
                )
            })
            .when(saved && !unsent, |row| {
                row.child(
                    div()
                        .debug_selector(move || format!("settings-saved-{id:?}"))
                        .text_sm()
                        .text_color(cx.theme().muted_foreground)
                        .child("Saved"),
                )
            })
            .child(
                // Only a field for how other devices reach a Device is ever disabled.
                div()
                    .id("text-field")
                    .when(options.is_disabled(), |field| {
                        field.tooltip(|window, cx| {
                            Tooltip::new(super::server::REACHABILITY_LOCKED).build(window, cx)
                        })
                    })
                    .child(Input::new(&input).w_64().disabled(options.is_disabled())),
            )
    })
    .on_reset(
        move |cx| dirty.read(cx).field(id).committed != default_dirty,
        move |_, cx| {
            reset.update(cx, |this, cx| {
                this.field_mut(id).draft = default.clone();
                this.commit(id, cx);
            });
        },
    )
}
