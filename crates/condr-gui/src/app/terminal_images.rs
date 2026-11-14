//! Fetching the kitty graphics images a Device's terminals show (ADR 0065): over the
//! Device's Stream link, by content hash, at most four at a time from one Device, decoded
//! off the UI thread into the cache every window draws from.

use super::*;
use crate::terminal_element::{TerminalImages, release_terminal_images};
use condr_core::TerminalImageKey;
use gpui_kit::RenderImage;
use std::sync::Arc;

/// Images fetched from one Device at once.
const FETCHES_PER_DEVICE: usize = 4;
/// When a failed fetch is tried again; the cache refuses a retry sooner.
const RETRY_AFTER: std::time::Duration = crate::terminal_element::TERMINAL_IMAGE_RETRY;

impl Condr {
    /// Starts fetching what the connection's terminals show and the GUI lacks.
    pub(super) fn fetch_terminal_images(&mut self, key: ConnectionKey, cx: &mut Context<Self>) {
        let events = self.ports_events(cx);
        let Some(connection) = self.connection_mut(key) else {
            return;
        };
        if connection.status != ConnectionStatus::Connected {
            return;
        }
        let free = FETCHES_PER_DEVICE.saturating_sub(connection.image_fetches);
        if free == 0 {
            return;
        }
        let shown: Vec<TerminalImageKey> = connection
            .model
            .terminals
            .values()
            .flat_map(|terminal| terminal.view.images.iter().map(|image| image.image))
            .collect();
        if shown.is_empty() {
            return;
        }
        let keys = cx.default_global::<TerminalImages>().claim(shown, free);
        if keys.is_empty() {
            return;
        }
        let Some(connection) = self.connection_mut(key) else {
            return;
        };
        connection.image_fetches += keys.len();
        let link = connection
            .forwards
            .link(&connection.label, &connection.endpoint, &events);
        let hold = connection.forwards.image_hold();
        for image in keys {
            let link = Arc::clone(&link);
            let hold = Arc::clone(&hold);
            cx.spawn(async move |this, cx| {
                let fetched = cx
                    .background_spawn(async move { fetch(&link, &hold, image) })
                    .await;
                let failed = fetched.is_none();
                let _ = this.update(cx, |this, cx| {
                    let evicted = cx.global_mut::<TerminalImages>().finish(image, fetched);
                    release_terminal_images(evicted, cx);
                    if let Some(connection) = this.connection_mut(key) {
                        connection.image_fetches = connection.image_fetches.saturating_sub(1);
                    }
                    this.fetch_terminal_images(key, cx);
                    cx.notify();
                });
                // A still image may never send another frame to retry it on.
                if failed {
                    cx.background_executor().timer(RETRY_AFTER).await;
                    let _ = this.update(cx, |this, cx| this.fetch_terminal_images(key, cx));
                }
            })
            .detach();
        }
    }
}

/// Holds the Device's Stream connection open for images from the first fetch on, since
/// an SSH Device logs in again for each opening (ADR 0065), then fetches and decodes.
fn fetch(
    link: &Arc<condr_client::StreamLink>,
    hold: &super::ports::ImageHold,
    image: TerminalImageKey,
) -> Option<Arc<RenderImage>> {
    {
        let mut held = hold
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if held.is_none() {
            if let Err(error) = link.retain() {
                tracing::debug!("terminal images cannot be fetched: {error}");
                return None;
            }
            *held = Some(super::ports::Retained::new(Arc::clone(link)));
        }
    }
    let png = match condr_client::fetch_image(link.as_ref(), image) {
        Ok(png) => png,
        Err(failure) => {
            tracing::debug!(?image, "terminal image fetch failed: {}", failure.detail);
            return None;
        }
    };
    decode(&png)
}

/// The PNG as a BGRA `RenderImage`, which is what GPUI draws.
fn decode(png: &[u8]) -> Option<Arc<RenderImage>> {
    let mut buffer = image::load_from_memory_with_format(png, image::ImageFormat::Png)
        .ok()?
        .into_rgba8();
    for pixel in buffer.as_chunks_mut::<4>().0 {
        pixel.swap(0, 2);
    }
    Some(Arc::new(RenderImage::new([image::Frame::new(buffer)])))
}

#[cfg(test)]
mod tests {
    use super::decode;

    #[test]
    fn a_png_decodes_to_bgra() {
        let mut png = Vec::new();
        image::RgbaImage::from_raw(1, 1, vec![10, 20, 30, 255])
            .unwrap()
            .write_to(&mut std::io::Cursor::new(&mut png), image::ImageFormat::Png)
            .unwrap();
        let image = decode(&png).unwrap();
        assert_eq!(image.as_bytes(0).unwrap(), [30, 20, 10, 255]);
    }
}
