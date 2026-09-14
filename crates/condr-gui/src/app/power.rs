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
