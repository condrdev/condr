use super::*;

#[derive(Clone, PartialEq)]
pub(super) struct TerminalRenderCacheKey {
    pub(super) runtime_epoch: Option<RuntimeEpoch>,
    pub(super) size: TerminalSize,
    pub(super) style: TextStyle,
    pub(super) font_size: Pixels,
    pub(super) palette: TerminalPalette,
}

/// What decides a cell's shaped glyphs: keyed by content, not grid position, so a line
/// that scrolls up one row, or the same prompt on another Pane row, hits the cache
/// (issue #22 kept per-cell shaping; this is its "cache by content" follow-up).
#[derive(Clone, PartialEq, Eq, Hash)]
pub(super) struct TerminalCellShapeKey {
    pub(super) text: SmolStr,
    /// `Hsla` bit patterns; colors reaching here are already exact.
    pub(super) foreground: [u32; 4],
    pub(super) flags: u16,
}

pub(super) struct CachedShape {
    pub(super) line: ShapedLine,
    /// The last `prepare` generation that used this entry.
    pub(super) used: u64,
}

/// Entries unused for this many frames are dropped once the table grows past the grid.
const SHAPE_CACHE_RETAIN_FRAMES: u64 = 120;

#[derive(Default)]
pub(crate) struct TerminalRenderCache {
    pub(super) key: Option<TerminalRenderCacheKey>,
    pub(super) shapes: HashMap<TerminalCellShapeKey, CachedShape>,
    pub(super) generation: u64,
    pub(super) cell_count: usize,
    /// Reused across frames so a redraw does not reallocate the positioned-cell list.
    pub(super) scratch_cells: Vec<ShapedCell>,
    #[cfg(all(test, feature = "test-support"))]
    pub(super) shaped_cells: usize,
}

#[cfg(all(test, feature = "test-support"))]
impl TerminalRenderCache {
    pub(crate) fn shaped_cells(&self) -> usize {
        self.shaped_cells
    }
}

impl TerminalRenderCache {
    pub(super) fn prepare(&mut self, key: TerminalRenderCacheKey, cell_count: usize) {
        if self.key.as_ref() != Some(&key) {
            self.key = Some(key);
            self.shapes.clear();
        }
        self.cell_count = cell_count;
        self.generation += 1;
        // Bounded: sweep stale entries only when the table outgrows a few screens.
        if self.shapes.len() > cell_count.saturating_mul(4).max(1024) {
            let stale_before = self.generation.saturating_sub(SHAPE_CACHE_RETAIN_FRAMES);
            self.shapes.retain(|_, shape| shape.used >= stale_before);
        }
    }

    pub(super) fn shaped_line(
        &mut self,
        text: &SmolStr,
        foreground: Hsla,
        flags: u16,
        shape: impl FnOnce() -> ShapedLine,
    ) -> ShapedLine {
        let key = TerminalCellShapeKey {
            text: text.clone(),
            foreground: [foreground.h, foreground.s, foreground.l, foreground.a].map(f32::to_bits),
            flags,
        };
        if let Some(cached) = self.shapes.get_mut(&key) {
            cached.used = self.generation;
            return cached.line.clone();
        }

        let line = shape();
        self.shapes.insert(
            key,
            CachedShape {
                line: line.clone(),
                used: self.generation,
            },
        );
        #[cfg(all(test, feature = "test-support"))]
        {
            self.shaped_cells += 1;
        }
        line
    }

    /// An empty positioned-cell list with last frame's capacity.
    pub(super) fn take_cells(&mut self) -> Vec<ShapedCell> {
        let mut cells = std::mem::take(&mut self.scratch_cells);
        cells.clear();
        cells.reserve(self.cell_count);
        cells
    }
}

pub(super) struct ShapedCell {
    pub(super) origin: Point<Pixels>,
    pub(super) line: ShapedLine,
}

pub(super) fn cell_font(mut font: Font, flags: u16) -> Font {
    if flags & BOLD != 0 {
        font = font.bold();
    }
    if flags & ITALIC != 0 {
        font = font.italic();
    }
    font
}

#[cfg(all(test, feature = "test-support"))]
mod tests {
    use super::*;

    #[test]
    fn render_cache_reuses_unchanged_cells_across_terminal_frames() {
        let mut cache = TerminalRenderCache::default();
        let key = TerminalRenderCacheKey {
            runtime_epoch: None,
            size: TerminalSize::new(24, 80),
            style: TextStyle::default(),
            font_size: px(14.),
            palette: TerminalPalette::default(),
        };
        let foreground: Hsla = rgb(0xffffff).into();
        cache.prepare(key.clone(), 2);
        cache.shaped_line(
            &SmolStr::new_inline("x"),
            foreground,
            0,
            ShapedLine::default,
        );
        assert_eq!(cache.shaped_cells(), 1);

        // Content-keyed: the same cell one frame later, or after scrolling to another
        // row, reuses its shaped line; only new content shapes.
        cache.prepare(key.clone(), 2);
        cache.shaped_line(&SmolStr::new_inline("x"), foreground, 0, || {
            panic!("unchanged cell should reuse its shaped line")
        });
        assert_eq!(cache.shaped_cells(), 1);
        cache.shaped_line(
            &SmolStr::new_inline("y"),
            foreground,
            0,
            ShapedLine::default,
        );
        assert_eq!(cache.shaped_cells(), 2);

        // A different color or style is different content.
        let dim: Hsla = rgb(0x888888).into();
        cache.shaped_line(&SmolStr::new_inline("x"), dim, 0, ShapedLine::default);
        cache.shaped_line(
            &SmolStr::new_inline("x"),
            foreground,
            BOLD,
            ShapedLine::default,
        );
        assert_eq!(cache.shaped_cells(), 4);

        // Entries fall out only after they go unused for many frames.
        for _ in 0..SHAPE_CACHE_RETAIN_FRAMES + 1 {
            cache.prepare(key.clone(), 1);
            cache.shaped_line(&SmolStr::new_inline("y"), foreground, 0, || {
                panic!("a hot entry must survive the sweep")
            });
        }
        for index in 0..2048u32 {
            cache.shaped_line(
                &SmolStr::from(index.to_string()),
                foreground,
                0,
                ShapedLine::default,
            );
        }
        cache.prepare(key, 1);
        cache.shaped_line(
            &SmolStr::new_inline("x"),
            foreground,
            0,
            ShapedLine::default,
        );
        assert_eq!(
            cache.shaped_cells(),
            4 + 2048 + 1,
            "stale x was swept and reshaped"
        );
    }

    /// The burst benchmark AGENTS.md asks for, GUI side: a full screen where every cell differs,
    /// then thousands of frames in which only a spinner cell changes, then a one-row scroll.
    /// Shaping work must follow the changed content, not the screen area. `--nocapture` prints
    /// the timing.
    #[test]
    fn a_spinner_burst_reshapes_only_the_changed_cells() {
        const FRAMES: usize = 2_000;
        let size = TerminalSize::new(24, 80);
        let (rows, columns) = (usize::from(size.rows), usize::from(size.columns));
        let cells = rows * columns;
        let mut cache = TerminalRenderCache::default();
        let key = TerminalRenderCacheKey {
            runtime_epoch: None,
            size,
            style: TextStyle::default(),
            font_size: px(14.),
            palette: TerminalPalette::default(),
        };
        let foreground: Hsla = rgb(0xffffff).into();
        let screen = (0..cells)
            .map(|index| SmolStr::from(index.to_string()))
            .collect::<Vec<_>>();
        let spinner = ["⠋", "⠙", "⠹", "⠸", "⠼", "⠴", "⠦", "⠧", "⠇", "⠏"].map(SmolStr::new_inline);
        let spinner_cell = cells - columns;

        let started = std::time::Instant::now();
        let mut shaped_per_frame = Vec::with_capacity(FRAMES);
        for frame in 0..FRAMES {
            cache.prepare(key.clone(), cells);
            let before = cache.shaped_cells();
            for (index, text) in screen.iter().enumerate() {
                let text = if index == spinner_cell {
                    &spinner[frame % spinner.len()]
                } else {
                    text
                };
                cache.shaped_line(text, foreground, 0, ShapedLine::default);
            }
            shaped_per_frame.push(cache.shaped_cells() - before);
        }
        let elapsed = started.elapsed();

        assert_eq!(
            shaped_per_frame[0], cells,
            "the first frame shapes the whole screen"
        );
        assert!(
            shaped_per_frame[1..spinner.len()]
                .iter()
                .all(|shaped| *shaped == 1),
            "each new spinner glyph shapes once: {:?}",
            &shaped_per_frame[..spinner.len()]
        );
        assert!(
            shaped_per_frame[spinner.len()..]
                .iter()
                .all(|shaped| *shaped == 0),
            "a full spinner cycle later nothing shapes"
        );

        // Output scrolled up one row: every surviving cell keeps its shaped line; the one
        // shaping is the text the spinner had covered, shown for the first time.
        cache.prepare(key, cells);
        let before = cache.shaped_cells();
        for index in 0..cells {
            let text = if index < cells - columns {
                &screen[index + columns]
            } else {
                &screen[index]
            };
            cache.shaped_line(text, foreground, 0, ShapedLine::default);
        }
        assert_eq!(
            cache.shaped_cells() - before,
            1,
            "a scroll reuses every row"
        );
        cache.prepare(
            TerminalRenderCacheKey {
                font_size: px(15.),
                ..cache.key.clone().unwrap()
            },
            cells,
        );
        assert!(
            cache.shapes.is_empty(),
            "a font change is the one whole-screen invalidation"
        );
        eprintln!("burst: {FRAMES} frames over {cells} cells, {elapsed:.2?}");
    }
}
