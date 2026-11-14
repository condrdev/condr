//! The kitty graphics images the Pane on screen shows (ADR 0065): fetched by their key over
//! the Device's Stream connection, which the first fetch opens and the Device's connection
//! keeps, and kept as PNG for the app to decode.

use condr_client::{RemoteEndpoint, StreamConnection, StreamLink};
use condr_core::TerminalImageKey;
use condr_core::protocol::Hello;
use std::collections::HashMap;
use std::sync::{Arc, Mutex, PoisonError};
use std::time::{Duration, Instant};

/// PNG kept for the app; the image drawn longest ago goes first.
const MAX_BYTES: usize = 64 * 1024 * 1024;
/// A failed fetch is tried again after this, while the screen still shows its image.
pub(crate) const RETRY_AFTER: Duration = Duration::from_secs(2);
/// Images fetched from one Device at once.
pub(crate) const FETCHES_PER_DEVICE: usize = 4;

#[derive(Default)]
pub(crate) struct ImageStore {
    entries: HashMap<TerminalImageKey, Entry>,
    bytes: usize,
    clock: u64,
}

enum Entry {
    Loading,
    Failed(Instant),
    Ready { png: Arc<Vec<u8>>, used: u64 },
}

impl ImageStore {
    /// Marks at most `limit` of `keys` the store neither holds nor is fetching as fetching.
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

    pub(crate) fn finish(&mut self, key: TerminalImageKey, png: Option<Vec<u8>>) {
        let Some(png) = png else {
            self.entries.insert(key, Entry::Failed(Instant::now()));
            return;
        };
        self.clock += 1;
        self.bytes += png.len();
        let ready = Entry::Ready {
            png: Arc::new(png),
            used: self.clock,
        };
        if let Some(Entry::Ready { png, .. }) = self.entries.insert(key, ready) {
            self.bytes -= png.len();
        }
        while self.bytes > MAX_BYTES {
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
            if let Some(Entry::Ready { png, .. }) = self.entries.remove(&oldest) {
                self.bytes -= png.len();
            }
        }
    }

    pub(crate) fn png(&mut self, key: &TerminalImageKey) -> Option<Arc<Vec<u8>>> {
        self.clock += 1;
        match self.entries.get_mut(key)? {
            Entry::Ready { png, used } => {
                *used = self.clock;
                Some(Arc::clone(png))
            }
            _ => None,
        }
    }
}

/// A Device's Stream connection for images: opened by the first fetch and held until the
/// Device's connection ends, when it is dropped and closes.
pub(crate) struct ImageLink {
    link: StreamLink,
    held: Mutex<bool>,
}

impl ImageLink {
    pub(crate) fn new(label: &str, endpoint: RemoteEndpoint, hello: Hello) -> Arc<Self> {
        Arc::new(Self {
            link: StreamLink::new(
                label,
                move || StreamConnection::connect(&endpoint, hello.clone()).map(|(c, _)| c),
                || {},
            ),
            held: Mutex::new(false),
        })
    }

    /// The image's PNG, or `None` when it cannot be had now.
    pub(crate) fn fetch(&self, key: TerminalImageKey) -> Option<Vec<u8>> {
        {
            let mut held = self.held.lock().unwrap_or_else(PoisonError::into_inner);
            if !*held {
                if let Err(error) = self.link.retain() {
                    tracing::debug!("terminal images cannot be fetched: {error}");
                    return None;
                }
                *held = true;
            }
        }
        condr_client::fetch_image(&self.link, key)
            .inspect_err(|failure| {
                tracing::debug!(?key, "terminal image fetch failed: {}", failure.detail)
            })
            .ok()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_key_is_fetched_once_and_the_oldest_png_goes_first() {
        let mut store = ImageStore::default();
        let keys: Vec<_> = (0..3).map(|seed| TerminalImageKey([seed; 32])).collect();
        assert_eq!(store.claim(keys.clone(), 2), keys[..2]);
        assert!(store.claim(keys[..2].to_vec(), 4).is_empty(), "in flight");
        store.finish(keys[0], None);
        assert!(store.claim([keys[0]], 4).is_empty(), "too soon to retry");
        store.finish(keys[1], Some(vec![0; MAX_BYTES / 2]));
        store.claim([keys[2]], 1);
        store.finish(keys[2], Some(vec![0; MAX_BYTES / 2 + 1]));
        assert!(store.png(&keys[1]).is_none(), "evicted");
        assert!(store.png(&keys[2]).is_some());
    }
}
