//! Where IME composition text sits before it is committed. As in Ghostty, it takes the
//! cursor's cells on the cursor's row, as wide as the terminal's own width table makes
//! each character, shifted left to fit the row; as in Windows Terminal, the cursor shows
//! at the IME's insertion point within it.

use condr_core::codepoint_width;
use std::ops::Range;

/// The text an IME is composing in a Pane.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct MarkedText {
    pub(crate) text: String,
    /// The IME's insertion point, a byte offset into `text`.
    pub(crate) caret: usize,
}

#[derive(Debug, Eq, PartialEq)]
pub(crate) struct PreeditChar {
    pub(crate) bytes: Range<usize>,
    pub(crate) column: u16,
    pub(crate) width: u16,
}

#[derive(Debug, Eq, PartialEq)]
pub(crate) struct PreeditLayout {
    /// The characters shown, left to right; a text wider than the row loses its first ones.
    pub(crate) chars: Vec<PreeditChar>,
    pub(crate) start: u16,
    /// The column after the last character.
    pub(crate) end: u16,
    /// The column the cursor shows at.
    pub(crate) caret: u16,
}

impl PreeditLayout {
    pub(crate) fn new(marked: &MarkedText, cursor_column: u16, columns: u16) -> Self {
        let columns = usize::from(columns.max(1));
        // A zero-width character has no cell to show in, so Ghostty leaves it out too.
        let mut chars: Vec<(Range<usize>, usize)> = marked
            .text
            .char_indices()
            .map(|(at, character)| {
                let width = usize::from(codepoint_width(character));
                (at..at + character.len_utf8(), width)
            })
            .filter(|(_, width)| *width > 0)
            .collect();
        // An insertion point after the text takes a cell of its own.
        let mut width = chars.iter().map(|(_, width)| width).sum::<usize>()
            + usize::from(marked.caret >= marked.text.len());
        while width > columns && !chars.is_empty() {
            width -= chars.remove(0).1;
        }
        let start = usize::from(cursor_column).min(columns.saturating_sub(width));
        let mut column = start;
        let chars: Vec<PreeditChar> = chars
            .into_iter()
            .map(|(bytes, width)| {
                let shown = PreeditChar {
                    bytes,
                    column: column as u16,
                    width: width as u16,
                };
                column += width;
                shown
            })
            .collect();
        let mut layout = Self {
            chars,
            start: start as u16,
            end: column as u16,
            caret: 0,
        };
        layout.caret = layout.column_of(marked.caret);
        layout
    }

    /// The column of the character starting at `byte`, or after the text.
    pub(crate) fn column_of(&self, byte: usize) -> u16 {
        self.chars
            .iter()
            .find(|shown| shown.bytes.start >= byte)
            .map_or(self.end, |shown| shown.column)
    }

    pub(crate) fn char_at(&self, column: u16) -> Option<&PreeditChar> {
        self.chars.iter().find(|shown| shown.column == column)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn marked(text: &str, caret: usize) -> MarkedText {
        MarkedText {
            text: text.into(),
            caret,
        }
    }

    fn columns(layout: &PreeditLayout) -> Vec<(u16, u16)> {
        layout
            .chars
            .iter()
            .map(|shown| (shown.column, shown.width))
            .collect()
    }

    #[test]
    fn composition_starts_at_the_cursor_and_the_cursor_follows_the_insertion_point() {
        let layout = PreeditLayout::new(&marked("ni hc", 5), 8, 80);
        assert_eq!((layout.start, layout.end, layout.caret), (8, 13, 13));

        let layout = PreeditLayout::new(&marked("你好a", 3), 2, 80);
        assert_eq!(columns(&layout), [(2, 2), (4, 2), (6, 1)]);
        assert_eq!(layout.caret, 4, "between the two characters");
        assert_eq!(layout.char_at(4).unwrap().bytes, 3..6);
    }

    #[test]
    fn composition_past_the_rows_end_shifts_left_with_room_for_the_cursor() {
        let layout = PreeditLayout::new(&marked("abcd", 4), 7, 10);
        assert_eq!((layout.start, layout.end, layout.caret), (5, 9, 9));

        let layout = PreeditLayout::new(&marked("abcd", 0), 7, 10);
        assert_eq!((layout.start, layout.end, layout.caret), (6, 10, 6));
    }

    #[test]
    fn widths_come_from_the_terminals_own_table() {
        // A combining acute accent takes no cell; a CJK ideograph and an emoji take two.
        let layout = PreeditLayout::new(&marked("e\u{301}字😀", 8), 0, 80);
        assert_eq!(columns(&layout), [(0, 1), (1, 2), (3, 2)]);
        assert_eq!(layout.caret, 5);
    }

    #[test]
    fn composition_wider_than_the_row_loses_its_first_characters() {
        let layout = PreeditLayout::new(&marked("你好世界", 12), 3, 6);
        assert_eq!(columns(&layout), [(1, 2), (3, 2)]);
        assert_eq!(&"你好世界"[layout.chars[0].bytes.clone()], "世");
        assert_eq!(layout.caret, 5);
        assert_eq!(
            layout.column_of(0),
            1,
            "a dropped character maps to the start"
        );
    }
}
