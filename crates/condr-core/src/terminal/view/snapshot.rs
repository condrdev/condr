use super::*;

pub(in crate::terminal) fn publish_view(
    revision: &AtomicU64,
    updates: &mpsc::Sender<TerminalUpdate>,
) {
    let revision = revision.fetch_add(1, Ordering::AcqRel) + 1;
    let _ = updates.send(TerminalUpdate::View(revision));
}

pub(in crate::terminal) fn blank_cell() -> TerminalCell {
    TerminalCell {
        text: SmolStr::new_static(" "),
        foreground: TerminalColor::Named(256),
        background: TerminalColor::Named(257),
        flags: 0,
        hyperlink: None,
    }
}

impl From<TerminalSize> for PtySize {
    fn from(size: TerminalSize) -> Self {
        Self {
            rows: size.rows,
            cols: size.columns,
            pixel_width: size.columns.saturating_mul(size.cell_width),
            pixel_height: size.rows.saturating_mul(size.cell_height),
        }
    }
}
