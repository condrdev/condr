use super::*;

// Terminals accept IME text, but printable chords must reach keybindings first.
pub(super) struct TerminalInputHandler {
    pub(super) inner: ElementInputHandler<Condr>,
}

impl TerminalInputHandler {
    pub(super) fn new(bounds: Bounds<Pixels>, view: Entity<Condr>) -> Self {
        Self {
            inner: ElementInputHandler::new(bounds, view),
        }
    }
}

impl InputHandler for TerminalInputHandler {
    fn selected_text_range(
        &mut self,
        ignore_disabled_input: bool,
        window: &mut Window,
        cx: &mut App,
    ) -> Option<UTF16Selection> {
        self.inner
            .selected_text_range(ignore_disabled_input, window, cx)
    }

    fn marked_text_range(&mut self, window: &mut Window, cx: &mut App) -> Option<Range<usize>> {
        self.inner.marked_text_range(window, cx)
    }

    fn text_for_range(
        &mut self,
        range: Range<usize>,
        adjusted_range: &mut Option<Range<usize>>,
        window: &mut Window,
        cx: &mut App,
    ) -> Option<String> {
        self.inner.text_for_range(range, adjusted_range, window, cx)
    }

    fn replace_text_in_range(
        &mut self,
        range: Option<Range<usize>>,
        text: &str,
        window: &mut Window,
        cx: &mut App,
    ) {
        self.inner.replace_text_in_range(range, text, window, cx);
    }

    fn replace_and_mark_text_in_range(
        &mut self,
        range: Option<Range<usize>>,
        new_text: &str,
        new_selected_range: Option<Range<usize>>,
        window: &mut Window,
        cx: &mut App,
    ) {
        self.inner
            .replace_and_mark_text_in_range(range, new_text, new_selected_range, window, cx);
    }

    fn unmark_text(&mut self, window: &mut Window, cx: &mut App) {
        self.inner.unmark_text(window, cx);
    }

    fn paste(&mut self, item: ClipboardItem, window: &mut Window, cx: &mut App) {
        self.inner.paste(item, window, cx);
    }

    fn bounds_for_range(
        &mut self,
        range: Range<usize>,
        window: &mut Window,
        cx: &mut App,
    ) -> Option<Bounds<Pixels>> {
        self.inner.bounds_for_range(range, window, cx)
    }

    fn character_index_for_point(
        &mut self,
        point: Point<Pixels>,
        window: &mut Window,
        cx: &mut App,
    ) -> Option<usize> {
        self.inner.character_index_for_point(point, window, cx)
    }

    fn element_bounds(&mut self, window: &mut Window, cx: &mut App) -> Option<Bounds<Pixels>> {
        self.inner.element_bounds(window, cx)
    }

    fn accepts_text_input(&mut self, window: &mut Window, cx: &mut App) -> bool {
        self.inner.accepts_text_input(window, cx)
    }

    fn prefers_ime_for_printable_keys(&mut self, _: &mut Window, _: &mut App) -> bool {
        false
    }

    fn text_input_configuration(
        &mut self,
        window: &mut Window,
        cx: &mut App,
    ) -> TextInputConfiguration {
        self.inner.text_input_configuration(window, cx)
    }
}

/// The link under a cell: the run of neighbours sharing its URI. The Server fills in
/// `hyperlink` from OSC 8 and from plain-text URLs alike, so this is all the Client knows.
pub(crate) fn link_at(view: &TerminalView, row: u16, column: u16) -> Option<HoveredTerminalLink> {
    let columns = u32::from(view.size.columns);
    let index = u32::from(row) * columns + u32::from(column);
    let uri = view.cell(row, column)?.hyperlink.as_ref()?;
    let same = |index: u32| {
        view.cells
            .get(index as usize)
            .is_some_and(|cell| cell.hyperlink.as_ref() == Some(uri))
    };
    let mut start = index;
    while start > 0 && same(start - 1) {
        start -= 1;
    }
    let mut end = index + 1;
    while same(end) {
        end += 1;
    }
    Some(HoveredTerminalLink {
        range: start..end,
        uri: uri.clone(),
        position: TerminalMousePosition { row, column },
    })
}

pub(super) fn terminal_position(
    point: Point<Pixels>,
    bounds: Bounds<Pixels>,
    cell_size: Size<Pixels>,
    terminal_size: TerminalSize,
) -> TerminalPosition {
    let x = (point.x - bounds.left()) / cell_size.width;
    let y = ((point.y - bounds.top()) / cell_size.height).max(0.);
    let side = if x <= 0. {
        TerminalSide::Left
    } else if x >= f32::from(terminal_size.columns) || x.fract() >= 0.5 {
        TerminalSide::Right
    } else {
        TerminalSide::Left
    };
    TerminalPosition {
        row: (y.floor() as u16).min(terminal_size.rows.saturating_sub(1)),
        column: (x.max(0.).floor() as u16).min(terminal_size.columns.saturating_sub(1)),
        side,
    }
}

pub(super) fn pointer_event_hits(
    hitbox: &Hitbox,
    position: Point<Pixels>,
    window: &Window,
) -> bool {
    hitbox.bounds.contains(&position) && hitbox.should_handle_scroll(window)
}

pub(super) fn terminal_mouse_position(
    point: Point<Pixels>,
    bounds: Bounds<Pixels>,
    cell_size: Size<Pixels>,
    terminal_size: TerminalSize,
) -> TerminalMousePosition {
    let position = terminal_position(point, bounds, cell_size, terminal_size);
    TerminalMousePosition {
        row: position.row,
        column: position.column,
    }
}

pub(super) fn terminal_mouse_button(button: MouseButton) -> Option<TerminalMouseButton> {
    match button {
        MouseButton::Left => Some(TerminalMouseButton::Left),
        MouseButton::Middle => Some(TerminalMouseButton::Middle),
        MouseButton::Right => Some(TerminalMouseButton::Right),
        MouseButton::Navigate(_) => None,
    }
}

pub(super) fn terminal_modifiers(modifiers: GpuiModifiers) -> TerminalModifiers {
    TerminalModifiers {
        shift: modifiers.shift,
        alt: modifiers.alt,
        control: modifiers.control,
        platform: modifiers.platform,
    }
}

pub(super) fn wheel_steps(delta: f32) -> u16 {
    if delta == 0. {
        0
    } else {
        delta.abs().round().max(1.) as u16
    }
}

pub(super) fn accumulated_wheel_steps(remainder: &mut f32, delta: f32) -> f32 {
    if delta == 0. {
        return 0.;
    }
    if *remainder != 0. && remainder.signum() != delta.signum() {
        *remainder = 0.;
    }
    *remainder += delta;
    let steps = remainder.trunc();
    *remainder -= steps;
    steps
}
