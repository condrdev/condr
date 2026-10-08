//! What the Preview Tab renders besides text (ADR 0037): images, decoded here with their
//! frames split so Condr rather than GPUI's `img` decides when an animation moves on. Every
//! image comes from the Server through `ReadFile`, so a remote Workspace renders the same.

use super::*;
use image::AnimationDecoder as _;
use std::hash::{DefaultHasher, Hash as _, Hasher as _};
use std::io::Cursor;

/// The decoded pixels one image may keep: an animation stops gaining frames past it.
// ponytail: a longer animation shows only the frames that fit; stream frames if one matters.
const MAX_DECODED_BYTES: usize = 256 << 20;
/// A frame asking for less shows for [`SHORT_FRAME_DELAY`], as browsers do.
const MIN_FRAME_DELAY: Duration = Duration::from_millis(20);
const SHORT_FRAME_DELAY: Duration = Duration::from_millis(100);

/// A file the Preview Tab can render instead of showing its text.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Rendering {
    Svg,
}

pub(super) fn rendering_for(path: &RelativePath) -> Option<Rendering> {
    match path.extension()?.to_ascii_lowercase().as_str() {
        "svg" => Some(Rendering::Svg),
        _ => None,
    }
}

/// How to decode a file by its extension: the Server's image list (`condr_core::is_image`)
/// and SVG.
fn image_format(path: &RelativePath) -> Option<ImageFormat> {
    Some(match path.extension()?.to_ascii_lowercase().as_str() {
        "png" => ImageFormat::Png,
        "jpg" | "jpeg" => ImageFormat::Jpeg,
        "gif" => ImageFormat::Gif,
        "webp" => ImageFormat::Webp,
        "bmp" => ImageFormat::Bmp,
        "ico" => ImageFormat::Ico,
        "tif" | "tiff" => ImageFormat::Tiff,
        "svg" => ImageFormat::Svg,
        _ => return None,
    })
}

/// A decoded image, one `RenderImage` per frame.
pub(super) struct DecodedImage {
    frames: Vec<(Arc<RenderImage>, Duration)>,
    /// The sum of the frame delays: one loop of the animation.
    period: Duration,
    started: Instant,
    /// When the redraw that shows the next frame is due, so drawing again in between
    /// does not schedule another one.
    wake: Mutex<Option<Instant>>,
    /// In image pixels, for the Preview Tab's header.
    pub(super) size: Size<u32>,
    pub(super) file_bytes: usize,
}

impl DecodedImage {
    /// The frame to draw now. An animation also schedules the redraw that shows the next
    /// one, at the animation's own pace rather than every display frame, and keeps going
    /// whatever the system's reduce-motion setting says: GPUI's `img` stops there, but
    /// this is a picture the user opened to watch.
    pub(super) fn frame(&self, window: &mut Window, cx: &mut App) -> Arc<RenderImage> {
        let [(first, _), ..] = self.frames.as_slice() else {
            unreachable!("a decoded image has a frame");
        };
        if self.frames.len() == 1 {
            return first.clone();
        }
        let (frame, left) = frame_at(&self.frames, self.period, self.started.elapsed());
        let now = Instant::now();
        let mut wake = self.wake.lock().unwrap_or_else(|error| error.into_inner());
        if wake.is_none_or(|due| due <= now) {
            *wake = Some(now + left);
            let view = window.current_view();
            window
                .spawn(cx, async move |cx| {
                    cx.background_executor().timer(left).await;
                    cx.update(|_, cx| cx.notify(view)).ok();
                })
                .detach();
        }
        frame
    }
}

/// The frame showing `elapsed` into a looping animation, and how long it has left.
fn frame_at(
    frames: &[(Arc<RenderImage>, Duration)],
    period: Duration,
    elapsed: Duration,
) -> (Arc<RenderImage>, Duration) {
    let mut at = Duration::from_nanos((elapsed.as_nanos() % period.as_nanos().max(1)) as u64);
    for (frame, delay) in frames {
        if at < *delay {
            return (frame.clone(), *delay - at);
        }
        at -= *delay;
    }
    (frames[0].0.clone(), frames[0].1)
}

fn decode(bytes: Vec<u8>, format: ImageFormat, svg: SvgRenderer) -> Result<DecodedImage, String> {
    let file_bytes = bytes.len();
    let frames = match format {
        ImageFormat::Gif => animation(
            image::codecs::gif::GifDecoder::new(Cursor::new(&bytes))
                .map_err(|error| error.to_string())?
                .into_frames(),
        )?,
        ImageFormat::Webp => {
            let decoder = image::codecs::webp::WebPDecoder::new(Cursor::new(&bytes))
                .map_err(|error| error.to_string())?;
            if decoder.has_animation() {
                animation(decoder.into_frames())?
            } else {
                still(bytes, format, svg)?
            }
        }
        _ => still(bytes, format, svg)?,
    };
    let first = &frames[0].0;
    let pixels = first.size(0);
    // GPUI rasterises SVG at twice its size, to stay sharp when scaled.
    let scale = if format == ImageFormat::Svg {
        SMOOTH_SVG_SCALE_FACTOR
    } else {
        1.
    };
    let size = size(
        (pixels.width.0 as f32 / scale).round() as u32,
        (pixels.height.0 as f32 / scale).round() as u32,
    );
    Ok(DecodedImage {
        period: frames.iter().map(|(_, delay)| *delay).sum(),
        frames,
        started: Instant::now(),
        wake: Mutex::new(None),
        size,
        file_bytes,
    })
}

fn still(
    bytes: Vec<u8>,
    format: ImageFormat,
    svg: SvgRenderer,
) -> Result<Vec<(Arc<RenderImage>, Duration)>, String> {
    let image = Image::from_bytes(format, bytes)
        .to_image_data(svg)
        .map_err(|error| error.to_string())?;
    Ok(vec![(image, Duration::ZERO)])
}

fn animation(frames: image::Frames<'_>) -> Result<Vec<(Arc<RenderImage>, Duration)>, String> {
    let mut decoded = Vec::new();
    let mut total = 0;
    for frame in frames {
        let frame = frame.map_err(|error| error.to_string())?;
        let delay = Duration::from(frame.delay());
        let mut buffer = frame.into_buffer();
        total += buffer.len();
        if total > MAX_DECODED_BYTES && !decoded.is_empty() {
            break;
        }
        // GPUI draws BGRA.
        for pixel in buffer.as_chunks_mut::<4>().0 {
            pixel.swap(0, 2);
        }
        let delay = if delay < MIN_FRAME_DELAY {
            SHORT_FRAME_DELAY
        } else {
            delay
        };
        decoded.push((
            Arc::new(RenderImage::new([image::Frame::new(buffer)])),
            delay,
        ));
    }
    if decoded.is_empty() {
        return Err("the image has no frames".into());
    }
    Ok(decoded)
}

/// One file of one Workspace on one connection.
pub(super) type ImageKey = (ConnectionKey, WorkspaceId, RelativePathBuf);

/// The Preview Tab's images: the file it shows when that is one.
#[derive(Clone, Default)]
pub(super) struct PreviewImages(Arc<Mutex<HashMap<ImageKey, ImageSlot>>>);

#[derive(Default)]
struct ImageSlot {
    /// What is drawn; `None` until the first decode lands. A refetch keeps it on screen.
    shown: Option<Result<Arc<DecodedImage>, SharedString>>,
    /// The generation of the file answer decoded, or being decoded.
    fed: Option<u64>,
    /// The bytes `shown` was decoded from, so refetching the same picture keeps it.
    digest: Option<u64>,
    /// Drawn since the last refresh: the loader asks `Condr` for it once, not per frame.
    wanted: bool,
}

impl PreviewImages {
    fn slots(&self) -> std::sync::MutexGuard<'_, HashMap<ImageKey, ImageSlot>> {
        self.0.lock().unwrap_or_else(|error| error.into_inner())
    }

    /// What to draw for one file, `None` while it loads.
    pub(super) fn get(&self, key: &ImageKey) -> Option<Result<Arc<DecodedImage>, SharedString>> {
        self.slots().get(key).and_then(|slot| slot.shown.clone())
    }

    /// Marks the file as drawn; `true` when it was not, so it still has to be asked for.
    pub(super) fn want(&self, key: ImageKey) -> bool {
        !std::mem::replace(&mut self.slots().entry(key).or_default().wanted, true)
    }

    /// Every image of the Workspace but `shown` is asked for again the next time it is
    /// drawn, as the Server's watcher wants; what it shows stays until the answer lands.
    pub(super) fn refresh(
        &self,
        connection: ConnectionKey,
        workspace_id: WorkspaceId,
        shown: Option<&RelativePath>,
    ) {
        for ((key, workspace, path), slot) in self.slots().iter_mut() {
            if *key == connection && *workspace == workspace_id && shown != Some(path) {
                slot.wanted = false;
            }
        }
    }

    /// Keeps the slots `keep` accepts and hands back the images of the others.
    pub(super) fn retain(&self, keep: impl Fn(&ImageKey) -> bool) -> Vec<Arc<DecodedImage>> {
        let mut dropped = Vec::new();
        self.slots().retain(|key, slot| {
            let kept = keep(key);
            if !kept && let Some(Ok(image)) = slot.shown.take() {
                dropped.push(image);
            }
            kept
        });
        dropped
    }

    /// Keeps, of `shown`'s Workspace, only `shown` itself: a Preview Tab that shows
    /// another file lets the last one's images go.
    pub(super) fn retain_shown(&self, shown: &ImageKey) -> Vec<Arc<DecodedImage>> {
        self.retain(|image| image.0 != shown.0 || image.1 != shown.1 || image == shown)
    }

    fn wanted(&self, connection: ConnectionKey, workspace_id: WorkspaceId) -> Vec<RelativePathBuf> {
        self.slots()
            .iter()
            .filter(|((key, workspace, _), slot)| {
                *key == connection && *workspace == workspace_id && slot.wanted
            })
            .map(|((_, _, path), _)| path.clone())
            .collect()
    }

    /// Records that `generation` of the file's answer is being used; `false` when it
    /// already was, or nothing draws the file any more.
    fn feed(&self, key: &ImageKey, generation: u64) -> bool {
        self.slots()
            .get_mut(key)
            .is_some_and(|slot| slot.fed.replace(generation) != Some(generation))
    }

    fn digest(&self, key: &ImageKey) -> Option<u64> {
        self.slots().get(key).and_then(|slot| slot.digest)
    }

    /// Shows a decode of `generation`, unless a newer answer has replaced it since; hands
    /// back the image it replaces.
    fn land(
        &self,
        key: &ImageKey,
        generation: u64,
        digest: Option<u64>,
        shown: Result<Arc<DecodedImage>, SharedString>,
    ) -> Option<Arc<DecodedImage>> {
        let mut slots = self.slots();
        let slot = slots
            .get_mut(key)
            .filter(|slot| slot.fed == Some(generation))?;
        slot.digest = digest;
        slot.shown.replace(shown)?.ok()
    }
}

/// Frees the frames of images nothing draws any more from every window's atlas.
pub(super) fn release_images(images: Vec<Arc<DecodedImage>>, cx: &mut App) {
    if images.is_empty() {
        return;
    }
    // Deferred: a window being updated is out of `App`'s list until it is done.
    cx.defer(move |cx| {
        for image in images {
            for (frame, _) in &image.frames {
                cx.drop_image(frame.clone(), None);
            }
        }
    });
}

impl Condr {
    /// Gives every image of the Workspace that something draws its file's answer: decodes
    /// what changed and asks the Server for what is missing.
    pub(super) fn feed_images(
        &mut self,
        key: ConnectionKey,
        workspace_id: WorkspaceId,
        cx: &mut Context<Self>,
    ) {
        for path in self.files_view.images.wanted(key, workspace_id) {
            let Some(connection) = self.connection(key) else {
                return;
            };
            let slot = (workspace_id, path.clone());
            let Some((generation, answer)) = connection.resources.files.get(&slot).cloned() else {
                if !connection.resources.pending_files.contains_key(&slot) {
                    self.request_file(key, workspace_id, path);
                }
                continue;
            };
            let image_key = (key, workspace_id, path);
            if !self.files_view.images.feed(&image_key, generation) {
                continue;
            }
            let decodable = match (answer, image_format(&image_key.2)) {
                (Ok(FileContent::Image { bytes }), Some(format)) => Ok((bytes, format)),
                (Ok(FileContent::Text { text }), Some(ImageFormat::Svg)) => {
                    Ok((text.into_bytes(), ImageFormat::Svg))
                }
                (Ok(FileContent::TooLarge { bytes }), _) => Err(format!(
                    "Image too large to show ({})",
                    file_size_text(bytes as usize)
                )),
                (Ok(_), _) => Err("Not an image".to_owned()),
                (Err(reason), _) => Err(reason),
            };
            let (bytes, format) = match decodable {
                Ok(decodable) => decodable,
                Err(reason) => {
                    let replaced = self.files_view.images.land(
                        &image_key,
                        generation,
                        None,
                        Err(reason.into()),
                    );
                    release_images(replaced.into_iter().collect(), cx);
                    continue;
                }
            };
            let mut hasher = DefaultHasher::new();
            bytes.hash(&mut hasher);
            let digest = hasher.finish();
            if self.files_view.images.digest(&image_key) == Some(digest) {
                continue;
            }
            let svg = cx.svg_renderer();
            cx.spawn(async move |this, cx| {
                let decoded = cx
                    .background_spawn(async move { decode(bytes, format, svg) })
                    .await;
                this.update(cx, |this, cx| {
                    let shown = decoded.map(Arc::new).map_err(SharedString::from);
                    let replaced =
                        this.files_view
                            .images
                            .land(&image_key, generation, Some(digest), shown);
                    release_images(replaced.into_iter().collect(), cx);
                    cx.notify();
                })
                .ok();
            })
            .detach();
        }
    }
}

/// An image file, scaled down to fit the Tab and never up.
pub(super) fn render_image(image: Arc<DecodedImage>) -> AnyElement {
    div()
        .debug_selector(|| "file-image".into())
        .flex_1()
        .min_h_0()
        .w_full()
        .p_4()
        .flex()
        .items_center()
        .justify_center()
        .overflow_hidden()
        .child(
            img(ImageSource::Custom(Arc::new(move |window, cx| {
                Some(Ok(image.frame(window, cx)))
            })))
            .max_w_full()
            .max_h_full()
            .object_fit(ObjectFit::ScaleDown),
        )
        .into_any_element()
}

/// An image's pixel size and file size, as the Preview Tab's header shows them.
pub(super) fn image_summary(image: &DecodedImage) -> String {
    format!(
        "{}×{} · {}",
        image.size.width,
        image.size.height,
        file_size_text(image.file_bytes)
    )
}

pub(super) fn file_size_text(bytes: usize) -> String {
    const KIB: f64 = 1024.;
    match bytes as f64 {
        size if size < KIB => format!("{bytes} B"),
        size if size < KIB * KIB => format!("{:.1} KiB", size / KIB),
        size => format!("{:.1} MiB", size / (KIB * KIB)),
    }
}

#[cfg(test)]
mod tests {
    // Not `super::*`: GPUI's own `test` attribute would shadow the standard one.
    use super::{
        Arc, Cursor, Duration, RenderImage, SHORT_FRAME_DELAY, animation, file_size_text, frame_at,
    };
    use image::AnimationDecoder as _;

    #[test]
    fn an_animation_loops_through_its_frames_by_their_delays() {
        let frame = || {
            Arc::new(RenderImage::new([image::Frame::new(
                image::RgbaImage::new(1, 1),
            )]))
        };
        let frames = vec![
            (frame(), Duration::from_millis(100)),
            (frame(), Duration::from_millis(50)),
        ];
        let period = Duration::from_millis(150);
        let at = |ms| {
            let (shown, left) = frame_at(&frames, period, Duration::from_millis(ms));
            let index = frames
                .iter()
                .position(|(frame, _)| Arc::ptr_eq(frame, &shown));
            (index.unwrap(), left.as_millis())
        };
        assert_eq!(at(0), (0, 100));
        assert_eq!(at(120), (1, 30));
        assert_eq!(at(160), (0, 90), "it loops");
    }

    #[test]
    fn short_frame_delays_slow_down_as_in_a_browser() {
        // A two-frame GIF whose frames ask for no delay at all.
        let mut gif = Vec::new();
        {
            let mut encoder = image::codecs::gif::GifEncoder::new(&mut gif);
            for _ in 0..2 {
                encoder
                    .encode_frame(image::Frame::new(image::RgbaImage::new(2, 1)))
                    .unwrap();
            }
        }
        let frames = animation(
            image::codecs::gif::GifDecoder::new(Cursor::new(&gif))
                .unwrap()
                .into_frames(),
        )
        .unwrap();
        assert_eq!(frames.len(), 2);
        assert!(frames.iter().all(|(_, delay)| *delay == SHORT_FRAME_DELAY));
    }

    /// GPUI's `img` freezes an animation when the system asks for reduced motion; the
    /// Preview Tab plays it anyway, redrawing at the animation's own pace (ADR 0037).
    #[cfg(feature = "test-support")]
    #[test]
    fn an_animation_keeps_playing_under_reduced_motion() {
        use super::{DecodedImage, Instant, Mutex, render_image};
        use gpui_kit::{
            Context, IntoElement, ParentElement as _, Render, Styled as _, TestAppContext, Window,
            div, size,
        };

        struct Shown(Arc<DecodedImage>);
        impl Render for Shown {
            fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
                div().size_full().flex().child(render_image(self.0.clone()))
            }
        }

        let frame = || {
            Arc::new(RenderImage::new([image::Frame::new(
                image::RgbaImage::new(2, 1),
            )]))
        };
        let image = Arc::new(DecodedImage {
            frames: vec![
                (frame(), Duration::from_millis(30)),
                (frame(), Duration::from_millis(30)),
            ],
            period: Duration::from_millis(60),
            started: Instant::now(),
            wake: Mutex::new(None),
            size: size(2, 1),
            file_bytes: 0,
        });
        let mut cx = TestAppContext::single();
        cx.update(|cx| cx.set_reduce_motion(true));
        let (_, window) = cx.add_window_view(|_, _| Shown(image.clone()));
        window.update(|window, cx| _ = window.draw(cx));
        let due = image
            .wake
            .lock()
            .unwrap()
            .expect("the redraw for the next frame is scheduled");
        assert!(due <= Instant::now() + Duration::from_millis(30));

        // Drawing again before then does not pile up another redraw.
        window.update(|window, cx| _ = window.draw(cx));
        assert_eq!(*image.wake.lock().unwrap(), Some(due));
    }

    #[test]
    fn sizes_read_in_the_largest_whole_unit() {
        assert_eq!(file_size_text(512), "512 B");
        assert_eq!(file_size_text(1536), "1.5 KiB");
        assert_eq!(file_size_text(3 << 20), "3.0 MiB");
    }
}
