//! Underlines and overlines, drawn apart from the shaped text: each SGR 4 style gets its
//! own look and colour (SGR 58), and the shaping cache never keys on them (ADR 0051).

use super::*;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum LineStyle {
    Single,
    Double,
    Curly,
    Dotted,
    Dashed,
    Over,
}

/// One line along `start..end` columns of `row`.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct Decoration {
    pub(crate) row: u16,
    pub(crate) start: u16,
    pub(crate) end: u16,
    pub(crate) style: LineStyle,
    pub(crate) color: Hsla,
}

/// A frame's lines, each run of cells with the same style and colour merged into one.
#[derive(Debug, Default)]
pub(crate) struct Decorations(pub(crate) Vec<Decoration>);

impl Decorations {
    /// The lines `flags` asks for over `span` cells from `column`: the underline in
    /// `underline`, the overline in the text's `foreground`.
    pub(super) fn add(
        &mut self,
        (row, column, span): (u16, u16, u16),
        flags: u32,
        underline: Hsla,
        foreground: Hsla,
    ) {
        let style = match flags & CELL_UNDERLINE {
            0 => None,
            CELL_UNDERLINE_DOUBLE => Some(LineStyle::Double),
            CELL_UNDERLINE_CURLY => Some(LineStyle::Curly),
            CELL_UNDERLINE_DOTTED => Some(LineStyle::Dotted),
            CELL_UNDERLINE_DASHED => Some(LineStyle::Dashed),
            _ => Some(LineStyle::Single),
        };
        if let Some(style) = style {
            self.push(row, column, span, style, underline);
        }
        if flags & CELL_OVERLINE != 0 {
            self.push(row, column, span, LineStyle::Over, foreground);
        }
    }

    fn push(&mut self, row: u16, column: u16, span: u16, style: LineStyle, color: Hsla) {
        // A cell adds at most two lines, so the run it continues is among the last two.
        if let Some(run) = self.0.iter_mut().rev().take(2).find(|run| {
            run.row == row && run.end == column && run.style == style && run.color == color
        }) {
            run.end = column + span;
            return;
        }
        self.0.push(Decoration {
            row,
            start: column,
            end: column + span,
            style,
            color,
        });
    }

    /// Paints every line on a grid whose top left is `origin`, underlines `underline_y`
    /// below each row's top, where shaped text would put them.
    pub(super) fn paint(
        &self,
        origin: Point<Pixels>,
        cell: Size<Pixels>,
        underline_y: Pixels,
        window: &mut Window,
    ) {
        let line = |color| UnderlineStyle {
            thickness: px(1.),
            color: Some(color),
            wavy: false,
        };
        for run in &self.0 {
            let left = origin.x + cell.width * usize::from(run.start);
            let top = origin.y + cell.height * usize::from(run.row);
            let width = cell.width * usize::from(run.end - run.start);
            let y = top + underline_y;
            match run.style {
                LineStyle::Single => {
                    window.paint_underline(point(left, y), width, &line(run.color))
                }
                LineStyle::Double => {
                    window.paint_underline(point(left, y - px(1.)), width, &line(run.color));
                    window.paint_underline(point(left, y + px(1.)), width, &line(run.color));
                }
                LineStyle::Curly => window.paint_underline(
                    point(left, y),
                    width,
                    &UnderlineStyle {
                        wavy: true,
                        ..line(run.color)
                    },
                ),
                LineStyle::Dotted => {
                    let mut x = left;
                    while x < left + width {
                        window.paint_quad(fill(
                            Bounds::new(point(x, y), size(px(1.), px(1.))),
                            run.color,
                        ));
                        x += px(2.);
                    }
                }
                LineStyle::Dashed => {
                    for column in run.start..run.end {
                        let cell_left = origin.x + cell.width * usize::from(column);
                        window.paint_quad(fill(
                            Bounds::new(
                                point(cell_left + cell.width * 0.15, y),
                                size(cell.width * 0.7, px(1.)),
                            ),
                            run.color,
                        ));
                    }
                }
                LineStyle::Over => window.paint_quad(fill(
                    Bounds::new(point(left, top), size(width, px(1.))),
                    run.color,
                )),
            }
        }
    }
}
