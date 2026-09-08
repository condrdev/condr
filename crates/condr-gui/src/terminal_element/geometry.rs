use super::*;

// Block characters are described on an integer subcell grid so that the
// regions of neighboring cells line up exactly and can merge: 8 subcolumns
// cover the eighth-width bars and 24 sublines cover eighths, halves and
// thirds of the cell height.
pub(super) const BLOCK_SUBCOLUMNS: i64 = 8;

pub(super) const BLOCK_SUBLINES: i64 = 24;

/// A block-element rectangle on the terminal-wide subcell grid, with
/// inclusive extents.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(super) struct BlockRegion {
    pub(super) start_line: i64,
    pub(super) start_col: i64,
    pub(super) end_line: i64,
    pub(super) end_col: i64,
    pub(super) color: Hsla,
}

impl BlockRegion {
    /// Two regions can merge when their union is again a rectangle of one
    /// color: equal line extents with touching or overlapping columns, or
    /// equal column extents with touching or overlapping lines.
    pub(super) fn can_merge_with(&self, other: &Self) -> bool {
        if self.color != other.color {
            return false;
        }
        let same_lines = self.start_line == other.start_line && self.end_line == other.end_line;
        let same_cols = self.start_col == other.start_col && self.end_col == other.end_col;
        let cols_touch = self.start_col <= other.end_col + 1 && other.start_col <= self.end_col + 1;
        let lines_touch =
            self.start_line <= other.end_line + 1 && other.start_line <= self.end_line + 1;
        (same_lines && cols_touch) || (same_cols && lines_touch)
    }

    pub(super) fn merge_with(&mut self, other: &Self) {
        self.start_line = self.start_line.min(other.start_line);
        self.start_col = self.start_col.min(other.start_col);
        self.end_line = self.end_line.max(other.end_line);
        self.end_col = self.end_col.max(other.end_col);
    }
}

/// Collapses adjacent same-color regions into fewer, larger rectangles so a
/// logo or QR code drawn from block characters does not cost one quad per
/// subcell.
pub(super) fn merge_block_regions(regions: Vec<BlockRegion>) -> Vec<BlockRegion> {
    // Cells are visited in row-major order, so one linear pass already
    // collapses the dominant case of horizontal runs (and vertical stacks
    // that end up adjacent in the vector).
    let mut merged: Vec<BlockRegion> = Vec::with_capacity(regions.len());
    for region in regions {
        match merged.last_mut() {
            Some(last) if last.can_merge_with(&region) => last.merge_with(&region),
            _ => merged.push(region),
        }
    }
    // ponytail: the exhaustive fixpoint is quadratic, so screens that still
    // hold thousands of unmergeable regions after the linear pass skip it
    // and just paint more quads; switch to a grid-based merge if profiling
    // ever shows quad count as the bottleneck.
    if merged.len() <= 256 {
        let mut changed = true;
        while changed {
            changed = false;
            let mut i = 0;
            while i < merged.len() {
                let mut j = i + 1;
                while j < merged.len() {
                    if merged[i].can_merge_with(&merged[j]) {
                        let other = merged.remove(j);
                        merged[i].merge_with(&other);
                        changed = true;
                    } else {
                        j += 1;
                    }
                }
                i += 1;
            }
        }
    }
    merged
}

/// Collects the filled rectangles of a block character into `regions`,
/// placed on the subcell grid at the given terminal cell. These characters
/// are painted as quads instead of font glyphs: glyphs only cover the em
/// box, which is shorter than the line height, so glyph-rendered blocks
/// leave horizontal seams and pixel art falls apart. Returns false when the
/// character is not a block element.
pub(super) fn collect_block_element_regions(
    regions: &mut Vec<BlockRegion>,
    character: char,
    row: u16,
    column: u16,
    color: Hsla,
) -> bool {
    const UPPER_LEFT: u8 = 1;
    const UPPER_RIGHT: u8 = 1 << 1;
    const LOWER_LEFT: u8 = 1 << 2;
    const LOWER_RIGHT: u8 = 1 << 3;
    // U+2596..=U+259F in code point order.
    const QUADRANTS: [u8; 10] = [
        LOWER_LEFT,
        LOWER_RIGHT,
        UPPER_LEFT,
        UPPER_LEFT | LOWER_LEFT | LOWER_RIGHT,
        UPPER_LEFT | LOWER_RIGHT,
        UPPER_LEFT | UPPER_RIGHT | LOWER_LEFT,
        UPPER_LEFT | UPPER_RIGHT | LOWER_RIGHT,
        UPPER_RIGHT,
        UPPER_RIGHT | LOWER_LEFT,
        UPPER_RIGHT | LOWER_LEFT | LOWER_RIGHT,
    ];

    let base_line = i64::from(row) * BLOCK_SUBLINES;
    let base_col = i64::from(column) * BLOCK_SUBCOLUMNS;
    let mut push = |x: i64, y: i64, width: i64, height: i64, color: Hsla| {
        regions.push(BlockRegion {
            start_line: base_line + y,
            start_col: base_col + x,
            end_line: base_line + y + height - 1,
            end_col: base_col + x + width - 1,
            color,
        });
    };
    match character {
        // Upper half.
        '\u{2580}' => push(0, 0, 8, 12, color),
        // Lower one eighth through the full block.
        '\u{2581}'..='\u{2588}' => {
            let eighths = i64::from(character as u32 - 0x2580);
            push(0, 24 - eighths * 3, 8, eighths * 3, color);
        }
        // Left seven eighths down to the left one eighth.
        '\u{2589}'..='\u{258F}' => push(0, 0, i64::from(0x2590 - character as u32), 24, color),
        // Right half.
        '\u{2590}' => push(4, 0, 4, 24, color),
        // Light, medium and dark shades, approximated with the foreground
        // color at reduced opacity instead of the fonts' stipple patterns,
        // trading pattern fidelity for seamless cell coverage.
        '\u{2591}' => push(0, 0, 8, 24, color.opacity(0.25)),
        '\u{2592}' => push(0, 0, 8, 24, color.opacity(0.5)),
        '\u{2593}' => push(0, 0, 8, 24, color.opacity(0.75)),
        // Upper one eighth.
        '\u{2594}' => push(0, 0, 8, 3, color),
        // Right one eighth.
        '\u{2595}' => push(7, 0, 1, 24, color),
        '\u{2596}'..='\u{259F}' => {
            let mask = QUADRANTS[character as usize - 0x2596];
            for (bit, x, y) in [
                (UPPER_LEFT, 0, 0),
                (UPPER_RIGHT, 4, 0),
                (LOWER_LEFT, 0, 12),
                (LOWER_RIGHT, 4, 12),
            ] {
                if mask & bit != 0 {
                    push(x, y, 4, 12, color);
                }
            }
        }
        // Legacy Computing sextants: a 2x3 grid of subcells, with bit
        // `row * 2 + column` filled. The code points enumerate every fill
        // combination in binary order, except the four that already exist
        // as Block Elements: empty, ▌ (0b010101), ▐ (0b101010) and █, so
        // the enumeration skips those values.
        '\u{1FB00}'..='\u{1FB3B}' => {
            let mut mask = character as u32 - 0x1FB00 + 1;
            if mask >= 0b010101 {
                mask += 1;
            }
            if mask >= 0b101010 {
                mask += 1;
            }
            for subrow in 0..3i64 {
                for subcolumn in 0..2i64 {
                    if mask & (1 << (subrow * 2 + subcolumn)) != 0 {
                        push(subcolumn * 4, subrow * 8, 4, 8, color);
                    }
                }
            }
        }
        _ => return false,
    }
    true
}

pub(super) fn push_selection_quads(
    quads: &mut Vec<PaintQuad>,
    selection: TerminalSelection,
    terminal_size: TerminalSize,
    bounds: Bounds<Pixels>,
    cell_size: Size<Pixels>,
    color: Hsla,
) {
    let columns = u32::from(terminal_size.columns);
    let cell_count = columns * u32::from(terminal_size.rows);
    let Some(last_cell) = cell_count.checked_sub(1) else {
        return;
    };
    let Some((start, end)) = selection.selected_cell_range(terminal_size.columns) else {
        return;
    };
    if start > last_cell {
        // A Server selection scrolled entirely out of view: exists, paints nothing.
        return;
    }
    let start = start.min(last_cell);
    let end = end.min(last_cell);

    for row in (start / columns)..=(end / columns) {
        let start_column = if row == start / columns {
            start % columns
        } else {
            0
        };
        let end_column = if row == end / columns {
            end % columns
        } else {
            columns - 1
        };
        quads.push(fill(
            Bounds::new(
                point(
                    bounds.left() + cell_size.width * start_column as usize,
                    bounds.top() + cell_size.height * row as usize,
                ),
                size(
                    cell_size.width * (end_column - start_column + 1) as usize,
                    cell_size.height,
                ),
            ),
            color,
        ));
    }
}

pub(super) fn terminal_grid_extent(extent: Pixels, cell: Pixels, minimum: u16) -> u16 {
    ((extent / cell).next_up().floor() as u16).max(minimum)
}
