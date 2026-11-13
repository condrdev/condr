//! The Server's kitty graphics images (ADR 0065): every Pane's, by content hash, kept so a
//! Client that fetches one over a Stream gets it as PNG even after its Pane moved on, as
//! an animation does every frame. Content-addressed, so one cache serves every Server a
//! process runs.

use condr_core::{TerminalImage, TerminalImageKey};
use std::collections::HashMap;
use std::sync::{Arc, LazyLock, Mutex};

/// What the cache holds at most, counted as each entry is held now: pixels until the
/// first fetch encodes them, PNG after.
const MAX_BYTES: usize = 128 * 1024 * 1024;

pub(super) static IMAGES: LazyLock<ImageCache> = LazyLock::new(|| ImageCache::new(MAX_BYTES));

pub(super) struct ImageCache {
    max_bytes: usize,
    inner: Mutex<Inner>,
}

#[derive(Default)]
struct Inner {
    entries: HashMap<TerminalImageKey, Entry>,
    bytes: usize,
    /// Bumped by every insert and touch; the entry with the oldest goes first.
    clock: u64,
}

struct Entry {
    data: Data,
    used: u64,
}

enum Data {
    /// Encoded on the first fetch, so an animation frame nobody fetches costs no PNG.
    Pixels(Arc<TerminalImage>),
    Png(Arc<Vec<u8>>),
}

impl Data {
    fn len(&self) -> usize {
        match self {
            Self::Pixels(image) => image.len(),
            Self::Png(png) => png.len(),
        }
    }
}

impl ImageCache {
    fn new(max_bytes: usize) -> Self {
        Self {
            max_bytes,
            inner: Mutex::new(Inner::default()),
        }
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, Inner> {
        // Nothing here leaves the map half-changed, so a panic elsewhere cannot spoil it.
        self.inner
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
    }

    /// Keeps an image a Pane showed, unless the cache already has its content.
    pub(super) fn insert(&self, image: TerminalImage) {
        let mut inner = self.lock();
        inner.clock += 1;
        let used = inner.clock;
        if let Some(entry) = inner.entries.get_mut(&image.key) {
            entry.used = used;
            return;
        }
        let key = image.key;
        let data = Data::Pixels(Arc::new(image));
        inner.bytes += data.len();
        inner.entries.insert(key, Entry { data, used });
        self.evict(&mut inner, key);
    }

    /// Marks images a Pane still shows as just used, so the ones gone go first.
    pub(super) fn touch<'a>(&self, keys: impl IntoIterator<Item = &'a TerminalImageKey>) {
        let mut inner = self.lock();
        inner.clock += 1;
        let used = inner.clock;
        for key in keys {
            if let Some(entry) = inner.entries.get_mut(key) {
                entry.used = used;
            }
        }
    }

    /// The image as PNG, encoding it outside the lock the first time it is asked for.
    pub(super) fn png(&self, key: &TerminalImageKey) -> Option<Arc<Vec<u8>>> {
        let image = match &self.lock().entries.get(key)?.data {
            Data::Png(png) => return Some(Arc::clone(png)),
            Data::Pixels(image) => Arc::clone(image),
        };
        let png = Arc::new(image.encode_png()?);
        let mut inner = self.lock();
        let Inner { entries, bytes, .. } = &mut *inner;
        if let Some(entry) = entries.get_mut(key)
            && matches!(entry.data, Data::Pixels(_))
        {
            *bytes = *bytes - entry.data.len() + png.len();
            entry.data = Data::Png(Arc::clone(&png));
        }
        Some(png)
    }

    /// Drops the least recently used entries until the cache fits, never `keep`: one image
    /// larger than the cache stays until the next arrives.
    fn evict(&self, inner: &mut Inner, keep: TerminalImageKey) {
        while inner.bytes > self.max_bytes {
            let Some(oldest) = inner
                .entries
                .iter()
                .filter(|(key, _)| **key != keep)
                .min_by_key(|(_, entry)| entry.used)
                .map(|(key, _)| *key)
            else {
                break;
            };
            if let Some(entry) = inner.entries.remove(&oldest) {
                inner.bytes -= entry.data.len();
            }
        }
    }

    #[cfg(test)]
    pub(super) fn with_max_bytes(max_bytes: usize) -> Self {
        Self::new(max_bytes)
    }

    #[cfg(test)]
    pub(super) fn contains(&self, key: &TerminalImageKey) -> bool {
        self.lock().entries.contains_key(key)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn image(seed: u8, side: u32) -> TerminalImage {
        TerminalImage::new(side, side, 4, vec![seed; (side * side * 4) as usize])
    }

    #[test]
    fn the_least_recently_used_image_goes_first_and_the_newest_stays() {
        let cache = ImageCache::with_max_bytes(2 * 16 * 16 * 4);
        let (first, second, third) = (image(1, 16), image(2, 16), image(3, 16));
        let keys = [first.key, second.key, third.key];
        cache.insert(first);
        cache.insert(second);
        cache.touch(&[keys[0]]);
        cache.insert(third);
        assert!(cache.contains(&keys[0]), "touched since");
        assert!(!cache.contains(&keys[1]), "least recently used");
        assert!(cache.contains(&keys[2]));
        let huge = image(4, 64);
        let huge_key = huge.key;
        cache.insert(huge);
        assert!(
            cache.contains(&huge_key),
            "an image larger than the cache stays alone"
        );
        assert!(!cache.contains(&keys[0]) && !cache.contains(&keys[2]));
    }

    #[test]
    fn a_fetched_image_is_held_as_its_png() {
        let cache = ImageCache::with_max_bytes(1 << 20);
        let image = image(9, 32);
        let key = image.key;
        cache.insert(image);
        let pixels = cache.lock().bytes;
        let png = cache.png(&key).unwrap();
        assert!(png.starts_with(b"\x89PNG"));
        assert_eq!(cache.lock().bytes, png.len());
        assert!(png.len() < pixels, "one flat colour compresses");
        assert!(Arc::ptr_eq(&png, &cache.png(&key).unwrap()));
    }
}
