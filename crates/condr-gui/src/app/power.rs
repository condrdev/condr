use super::sidebar::CondrIconName;
use super::*;

impl Condr {
    pub(in crate::app) fn set_keep_awake(&mut self, enabled: bool, cx: &mut Context<Self>) {
        if self.keep_awake == enabled {
            return;
        }
        self.keep_awake = enabled;
        self.save_keep_awake(cx);
        self.apply_keep_awake();
        cx.notify();
    }

    /// Holds or releases the OS request that matches the switch. A laptop left to wait on
    /// agents otherwise blanks, locks and eventually sleeps under them.
    pub(super) fn apply_keep_awake(&mut self) {
        if !self.keep_awake {
            self._keep_awake = None;
            return;
        }
        if self._keep_awake.is_some() {
            return;
        }
        match keepawake::Builder::default()
            .display(true)
            .idle(true)
            .reason("Waiting on agents")
            .app_name(condr_core::APP_NAME)
            .app_reverse_domain("dev.condr.gui")
            .create()
        {
            Ok(handle) => self._keep_awake = Some(handle),
            Err(error) => {
                self.app_error = Some(format!("Failed to keep the screen awake: {error}"));
            }
        }
    }
}

/// The keep-awake toggle: a coffee cup, as caffeinate and its kin draw it, lit while
/// the request is held. It flips the same switch as Settings > Power.
pub(super) fn keep_awake_button(keep_awake: bool, owner: WeakEntity<Condr>, cx: &App) -> Button {
    let label = if keep_awake {
        "Keeping the screen awake"
    } else {
        "Keep the screen awake"
    };
    Button::new("toggle-keep-awake")
        .debug_selector(|| "toggle-keep-awake".into())
        .ghost()
        .small()
        .icon(Icon::new(CondrIconName::Coffee))
        .selected(keep_awake)
        .text_color(if keep_awake {
            cx.theme().success
        } else {
            cx.theme().muted_foreground
        })
        .tooltip(label)
        .accessibility_label(label)
        .on_click(move |_, _, cx| {
            let _ = owner.update(cx, |this, cx| this.set_keep_awake(!this.keep_awake, cx));
        })
}
