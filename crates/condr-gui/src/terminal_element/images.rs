//! Kitty graphics images in the terminal (ADR 0065): the GUI's decoded images, shared by
//! every window and keyed by content, and where a placement draws in a Pane.

use condr_core::{TerminalImageKey, TerminalImagePlacement, TerminalSize};
use gpui_kit::{App, Bounds, Global, Pixels, Point, RenderImage, Size, point, px, size};
use std::collections::HashMap;
use std::sync::Arc;
use std::time::{Duration, Instant};

/// Decoded pixels kept for the whole GUI; the image drawn longest ago goes first.
const MAX_DECODED_BYTES: usize = 256 * 1024 * 1024;
/// A fetch that failed is tried again once a frame still names its image after this.
pub(crate) const RETRY_AFTER: Duration = Duration::from_secs(2);

#[derive(Default)]
pub(crate) struct TerminalImages {
    entries: HashMap<TerminalImageKey, Entry>,
    bytes: usize,
    clock: u64,
}

impl Global for TerminalImages {}

enum Entry {
    Loading,
    Failed(Instant),
    Ready {
        image: Arc<RenderImage>,
        bytes: usize,
        used: u64,
    },
}

impl TerminalImages {
    /// Marks at most `limit` of `keys` this GUI neither holds nor is fetching as fetching,
    /// and returns them for the caller to fetch.
    pub(crate) fn claim(
        &mut self,
        keys: impl IntoIterator<Item = TerminalImageKey>,
        limit: usize,
    ) -> Vec<TerminalImageKey> {
        let mut claimed = Vec::new();
        for key in keys {
            if claimed.len() == limit {
                break;
            }
            let wanted = match self.entries.get(&key) {
                None => true,
                Some(Entry::Failed(at)) => at.elapsed() >= RETRY_AFTER,
                Some(Entry::Loading | Entry::Ready { .. }) => false,
            };
            if wanted {
                self.entries.insert(key, Entry::Loading);
                claimed.push(key);
            }
        }
        claimed
    }

    /// A fetch ended. Returns the images evicted to make room, for the caller to free.
    pub(crate) fn finish(
        &mut self,
        key: TerminalImageKey,
        image: Option<Arc<RenderImage>>,
    ) -> Vec<Arc<RenderImage>> {
        let Some(image) = image else {
            self.entries.insert(key, Entry::Failed(Instant::now()));
            return Vec::new();
        };
        let size = image.size(0);
        let bytes = size.width.0 as usize * size.height.0 as usize * 4;
        self.clock += 1;
        if let Some(Entry::Ready { bytes: old, .. }) = self.entries.insert(
            key,
            Entry::Ready {
                image,
                bytes,
                used: self.clock,
            },
        ) {
            self.bytes -= old;
        }
        self.bytes += bytes;
        let mut evicted = Vec::new();
        while self.bytes > MAX_DECODED_BYTES {
            let oldest = self
                .entries
                .iter()
                .filter(|(other, _)| **other != key)
                .filter_map(|(other, entry)| match entry {
                    Entry::Ready { used, .. } => Some((*used, *other)),
                    _ => None,
                })
                .min();
            let Some((_, oldest)) = oldest else {
                break;
            };
            if let Some(Entry::Ready { image, bytes, .. }) = self.entries.remove(&oldest) {
                self.bytes -= bytes;
                evicted.push(image);
            }
        }
        evicted
    }

    /// Whether the image is decoded and ready to draw.
    #[cfg(feature = "test-support")]
    pub(crate) fn ready(&self, key: &TerminalImageKey) -> bool {
        matches!(self.entries.get(key), Some(Entry::Ready { .. }))
    }

    /// The image if decoded, marked as just drawn.
    pub(crate) fn draw(&mut self, key: &TerminalImageKey) -> Option<Arc<RenderImage>> {
        self.clock += 1;
        match self.entries.get_mut(key)? {
            Entry::Ready { image, used, .. } => {
                *used = self.clock;
                Some(Arc::clone(image))
            }
            _ => None,
        }
    }
}

/// Frees images no longer kept from every window's atlas.
pub(crate) fn release(images: Vec<Arc<RenderImage>>, cx: &mut App) {
    if images.is_empty() {
        return;
    }
    // Deferred: a window being updated is out of `App`'s list until it is done.
    cx.defer(move |cx| {
        for image in images {
            cx.drop_image(image, None);
        }
    });
}

/// Where a placement draws in a Pane whose grid starts at `origin` with `cell` cells: the
/// destination, and the whole image's bounds so that its source rectangle fills that
/// destination. The VT measured it in its own cells, which may not be this window's.
pub(crate) fn placement_bounds(
    placement: &TerminalImagePlacement,
    terminal: TerminalSize,
    origin: Point<Pixels>,
    cell: Size<Pixels>,
) -> Option<(Bounds<Pixels>, Bounds<Pixels>)> {
    if terminal.cell_width == 0
        || terminal.cell_height == 0
        || placement.source_width == 0
        || placement.source_height == 0
    {
        return None;
    }
    let scale_x = cell.width.as_f32() / f32::from(terminal.cell_width);
    let scale_y = cell.height.as_f32() / f32::from(terminal.cell_height);
    let left = origin.x.as_f32()
        + cell.width.as_f32() * placement.column as f32
        + placement.offset_x as f32 * scale_x;
    let top = origin.y.as_f32()
        + cell.height.as_f32() * placement.row as f32
        + placement.offset_y as f32 * scale_y;
    let width = placement.width as f32 * scale_x;
    let height = placement.height as f32 * scale_y;
    let per_source_x = width / placement.source_width as f32;
    let per_source_y = height / placement.source_height as f32;
    let destination = Bounds::new(point(px(left), px(top)), size(px(width), px(height)));
    let image = Bounds::new(
        point(
            px(left - placement.source_x as f32 * per_source_x),
            px(top - placement.source_y as f32 * per_source_y),
        ),
        size(
            px(placement.image_width as f32 * per_source_x),
            px(placement.image_height as f32 * per_source_y),
        ),
    );
    Some((destination, image))
}

#[cfg(test)]
mod tests {
    use super::*;
    use condr_core::TerminalImageLayer;

    fn placement() -> TerminalImagePlacement {
        TerminalImagePlacement {
            image: TerminalImageKey([1; 32]),
            image_width: 40,
            image_height: 20,
            layer: TerminalImageLayer::AboveText,
            z: 0,
            row: -1,
            column: 2,
            offset_x: 4,
            offset_y: 0,
            width: 20,
            height: 20,
            source_x: 20,
            source_y: 0,
            source_width: 20,
            source_height: 20,
        }
    }

    #[test]
    fn the_sizing_window_draws_device_pixels_at_its_scale() {
        // A 2× window: 8×16 logical cells the VT holds as 16×32 device pixels.
        let terminal = TerminalSize::new(10, 10).with_cell_size(16, 32);
        let cell = size(px(8.), px(16.));
        let (destination, image) =
            placement_bounds(&placement(), terminal, point(px(100.), px(50.)), cell).unwrap();
        assert_eq!(destination.origin, point(px(118.), px(34.)));
        assert_eq!(destination.size, size(px(10.), px(10.)));
        // The right half of the image is the source: the whole image starts a half left.
        assert_eq!(image.origin, point(px(108.), px(34.)));
        assert_eq!(image.size, size(px(20.), px(10.)));
    }

    #[test]
    fn a_window_with_larger_cells_scales_up() {
        let terminal = TerminalSize::new(10, 10).with_cell_size(8, 16);
        let cell = size(px(12.), px(24.));
        let (destination, _) =
            placement_bounds(&placement(), terminal, point(px(0.), px(0.)), cell).unwrap();
        assert_eq!(destination.size, size(px(30.), px(30.)));
        assert_eq!(destination.origin, point(px(30.), px(-24.)));
    }

    #[test]
    fn nothing_draws_without_a_cell_size() {
        let terminal = TerminalSize::new(10, 10);
        assert!(
            placement_bounds(
                &placement(),
                terminal,
                point(px(0.), px(0.)),
                size(px(8.), px(16.))
            )
            .is_none()
        );
    }

    fn render_image(side: u32) -> Arc<RenderImage> {
        let buffer = image::RgbaImage::new(side, side);
        Arc::new(RenderImage::new([image::Frame::new(buffer)]))
    }

    #[test]
    fn a_key_is_fetched_once_and_a_failure_waits_before_a_retry() {
        let mut images = TerminalImages::default();
        let (first, second) = (TerminalImageKey([1; 32]), TerminalImageKey([2; 32]));
        assert_eq!(
            images.claim([first, second], 1),
            [first],
            "within the limit"
        );
        assert_eq!(images.claim([first, second], 4), [second]);
        assert!(
            images.claim([first, second], 4).is_empty(),
            "both in flight"
        );
        images.finish(first, None);
        assert!(images.claim([first], 4).is_empty(), "too soon to retry");
        images.finish(second, Some(render_image(1)));
        assert!(images.draw(&second).is_some());
        assert!(images.draw(&first).is_none());
    }

    #[test]
    fn the_image_drawn_longest_ago_goes_first() {
        let mut images = TerminalImages::default();
        // 4096² RGBA is 64 MiB: four fill the cache.
        let keys: Vec<_> = (0..5).map(|seed| TerminalImageKey([seed; 32])).collect();
        for key in &keys[..4] {
            images.claim([*key], 1);
            assert!(images.finish(*key, Some(render_image(4096))).is_empty());
        }
        images.draw(&keys[0]);
        images.claim([keys[4]], 1);
        let evicted = images.finish(keys[4], Some(render_image(4096)));
        assert_eq!(evicted.len(), 1);
        assert!(images.draw(&keys[0]).is_some(), "drawn since");
        assert!(images.draw(&keys[1]).is_none(), "drawn longest ago");
    }
}
