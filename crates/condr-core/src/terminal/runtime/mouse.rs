use super::*;

impl TerminalRuntime {
    pub(super) fn handle_mouse(&self, event: TerminalMouseEvent) -> io::Result<()> {
        let (modes, display_offset, screen_lines) = {
            let terminal = self.terminal.lock().expect("terminal state lock poisoned");
            (
                *terminal.mode(),
                terminal.grid().display_offset(),
                terminal.screen_lines(),
            )
        };
        let shifted_wheel = matches!(
            event,
            TerminalMouseEvent::Wheel { modifiers, .. } if modifiers.shift
        );
        if !shifted_wheel && modes.intersects(TermMode::MOUSE_MODE) {
            return self.handle_application_mouse(event, modes, display_offset, screen_lines);
        }
        if let TerminalMouseEvent::Button {
            button,
            pressed: false,
            ..
        } = event
        {
            self.clear_reported_mouse_press(button);
        }

        let TerminalMouseEvent::Wheel {
            direction, amount, ..
        } = event
        else {
            return Ok(());
        };
        if amount == 0 || (!shifted_wheel && modes.intersects(TermMode::MOUSE_MODE)) {
            return Ok(());
        }

        if !shifted_wheel && modes.contains(TermMode::ALT_SCREEN | TermMode::ALTERNATE_SCROLL) {
            let key = match direction {
                TerminalMouseWheel::Up => TerminalKey::Up,
                TerminalMouseWheel::Down => TerminalKey::Down,
                TerminalMouseWheel::Left | TerminalMouseWheel::Right => return Ok(()),
            };
            let bytes = encode_key(
                &key,
                TerminalModifiers::default(),
                modes.contains(TermMode::APP_CURSOR),
            )?
            .repeat(usize::from(amount.min(MAX_MOUSE_WHEEL_STEPS)));
            return self.input.try_write(bytes);
        }

        let lines = match direction {
            TerminalMouseWheel::Up => i32::from(amount),
            TerminalMouseWheel::Down => -i32::from(amount),
            TerminalMouseWheel::Left | TerminalMouseWheel::Right => return Ok(()),
        };
        self.scroll(TerminalScroll::Lines(lines));
        Ok(())
    }

    pub(super) fn handle_application_mouse(
        &self,
        event: TerminalMouseEvent,
        modes: TermMode,
        display_offset: usize,
        screen_lines: usize,
    ) -> io::Result<()> {
        match event {
            TerminalMouseEvent::Button {
                button,
                pressed: true,
                position,
                modifiers,
            } => {
                let Some(position) = live_mouse_position(position, display_offset, screen_lines)
                else {
                    return Ok(());
                };
                let event = TerminalMouseEvent::Button {
                    button,
                    pressed: true,
                    position,
                    modifiers,
                };
                if self.write_mouse_report(event, modes)? {
                    *self
                        .reported_mouse_press
                        .lock()
                        .expect("reported mouse press lock poisoned") = Some(ReportedMousePress {
                        button,
                        position,
                        modifiers,
                    });
                }
            }
            TerminalMouseEvent::Button {
                button,
                pressed: false,
                position,
                modifiers,
            } => {
                let mut reported = self
                    .reported_mouse_press
                    .lock()
                    .expect("reported mouse press lock poisoned");
                let Some(press) = *reported else {
                    return Ok(());
                };
                if press.button != button {
                    return Ok(());
                }
                let position = live_mouse_position(position, display_offset, screen_lines)
                    .unwrap_or(press.position);
                let event = TerminalMouseEvent::Button {
                    button,
                    pressed: false,
                    position,
                    modifiers,
                };
                if let Some(bytes) = encode_mouse(event, modes) {
                    self.input.try_write_control(bytes)?;
                }
                *reported = None;
            }
            TerminalMouseEvent::Motion {
                button: Some(button),
                position,
                modifiers,
            } => {
                let Some(position) = live_mouse_position(position, display_offset, screen_lines)
                else {
                    return Ok(());
                };
                let mut reported = self
                    .reported_mouse_press
                    .lock()
                    .expect("reported mouse press lock poisoned");
                let Some(press) = reported.as_mut() else {
                    return Ok(());
                };
                if press.button != button {
                    return Ok(());
                }
                let event = TerminalMouseEvent::Motion {
                    button: Some(button),
                    position,
                    modifiers,
                };
                if let Some(bytes) = encode_mouse(event, modes) {
                    self.input.try_write(bytes)?;
                    press.position = position;
                    press.modifiers = modifiers;
                }
            }
            TerminalMouseEvent::Motion {
                button: None,
                position,
                modifiers,
            } => {
                let Some(position) = live_mouse_position(position, display_offset, screen_lines)
                else {
                    return Ok(());
                };
                self.write_mouse_report(
                    TerminalMouseEvent::Motion {
                        button: None,
                        position,
                        modifiers,
                    },
                    modes,
                )?;
            }
            TerminalMouseEvent::Wheel {
                direction,
                amount,
                position,
                modifiers,
            } => {
                let Some(position) = live_mouse_position(position, display_offset, screen_lines)
                else {
                    return Ok(());
                };
                self.write_mouse_report(
                    TerminalMouseEvent::Wheel {
                        direction,
                        amount,
                        position,
                        modifiers,
                    },
                    modes,
                )?;
            }
        }
        Ok(())
    }

    pub(super) fn write_mouse_report(
        &self,
        event: TerminalMouseEvent,
        modes: TermMode,
    ) -> io::Result<bool> {
        let Some(bytes) = encode_mouse(event, modes) else {
            return Ok(false);
        };
        self.input.try_write(bytes)?;
        Ok(true)
    }

    pub(super) fn clear_reported_mouse_press(&self, button: TerminalMouseButton) {
        let mut reported = self
            .reported_mouse_press
            .lock()
            .expect("reported mouse press lock poisoned");
        if reported.is_some_and(|press| press.button == button) {
            *reported = None;
        }
    }

    pub fn release_mouse(&self) -> io::Result<()> {
        let modes = *self
            .terminal
            .lock()
            .expect("terminal state lock poisoned")
            .mode();
        let mut reported = self
            .reported_mouse_press
            .lock()
            .expect("reported mouse press lock poisoned");
        let Some(press) = *reported else {
            return Ok(());
        };
        if let Some(bytes) = encode_mouse(
            TerminalMouseEvent::Button {
                button: press.button,
                pressed: false,
                position: press.position,
                modifiers: press.modifiers,
            },
            modes,
        ) {
            self.input.try_write_control(bytes)?;
        }
        *reported = None;
        Ok(())
    }
}
