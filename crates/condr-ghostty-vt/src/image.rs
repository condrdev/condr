//! Kitty graphics (ADR 0065): the process-wide PNG decoder the library calls, and the
//! images a terminal holds. Where they are drawn comes from the render state.

use super::{Terminal, check, sys};
use std::ffi::c_void;
use std::panic::{self, AssertUnwindSafe};
use std::ptr;
use std::slice;

/// How a stored image's pixels are laid out, one byte per channel.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub enum ImageFormat {
    Gray,
    GrayAlpha,
    Rgb,
    Rgba,
}

impl ImageFormat {
    pub fn channels(self) -> usize {
        match self {
            Self::Gray => 1,
            Self::GrayAlpha => 2,
            Self::Rgb => 3,
            Self::Rgba => 4,
        }
    }
}

/// An image's current pixels, borrowed from the terminal until it is next changed.
#[derive(Clone, Copy, Debug)]
pub struct Image<'a> {
    pub width: u32,
    pub height: u32,
    pub format: ImageFormat,
    pub pixels: &'a [u8],
}

impl Terminal {
    fn kitty_graphics(&self) -> Option<sys::GhosttyKittyGraphics> {
        let mut graphics: sys::GhosttyKittyGraphics = ptr::null_mut();
        self.get(
            sys::GhosttyTerminalData::GHOSTTY_TERMINAL_DATA_KITTY_GRAPHICS,
            &mut graphics,
        )
        .ok()?;
        (!graphics.is_null()).then_some(graphics)
    }

    /// The image `id` of the active screen with the pixels it shows now, or `None` while
    /// they are still arriving.
    pub fn image(&self, id: u32) -> Option<Image<'_>> {
        let graphics = self.kitty_graphics()?;
        // SAFETY: a live storage handle; the image handle is used only below, before any
        // call that changes the terminal, which `&self` excludes.
        let image = unsafe { sys::ghostty_kitty_graphics_image(graphics, id) };
        if image.is_null() {
            return None;
        }
        use sys::GhosttyKittyGraphicsImageData as Data;
        let (mut width, mut height, mut len) = (0u32, 0u32, 0usize);
        let mut format = sys::GhosttyKittyImageFormat::GHOSTTY_KITTY_IMAGE_FORMAT_RGBA;
        let mut data: *const u8 = ptr::null();
        let keys = [
            Data::GHOSTTY_KITTY_IMAGE_DATA_WIDTH,
            Data::GHOSTTY_KITTY_IMAGE_DATA_HEIGHT,
            Data::GHOSTTY_KITTY_IMAGE_DATA_FORMAT,
            Data::GHOSTTY_KITTY_IMAGE_DATA_DATA_LEN,
            Data::GHOSTTY_KITTY_IMAGE_DATA_DATA_PTR,
        ];
        let mut values: [*mut c_void; 5] = [
            (&raw mut width).cast(),
            (&raw mut height).cast(),
            (&raw mut format).cast(),
            (&raw mut len).cast(),
            (&raw mut data).cast(),
        ];
        // SAFETY: each value points at the type the header names for its key.
        check(unsafe {
            sys::ghostty_kitty_graphics_image_get_multi(
                image,
                keys.len(),
                keys.as_ptr(),
                values.as_mut_ptr(),
                ptr::null_mut(),
            )
        })
        .ok()?;
        use sys::GhosttyKittyImageFormat as Format;
        let format = match format {
            Format::GHOSTTY_KITTY_IMAGE_FORMAT_GRAY => ImageFormat::Gray,
            Format::GHOSTTY_KITTY_IMAGE_FORMAT_GRAY_ALPHA => ImageFormat::GrayAlpha,
            Format::GHOSTTY_KITTY_IMAGE_FORMAT_RGB => ImageFormat::Rgb,
            Format::GHOSTTY_KITTY_IMAGE_FORMAT_RGBA => ImageFormat::Rgba,
            // PNG is decoded before it is stored; anything newer is not drawn.
            _ => return None,
        };
        let expected = (width as usize)
            .checked_mul(height as usize)?
            .checked_mul(format.channels())?;
        if data.is_null() || len != expected {
            return None;
        }
        Some(Image {
            width,
            height,
            format,
            // SAFETY: the library keeps `len` bytes at `data` until the terminal changes,
            // which the borrow of `self` rules out.
            pixels: unsafe { slice::from_raw_parts(data, len) },
        })
    }

    /// Moves playing animations to the frame due at `now_ms` and returns when the next is
    /// due, on the same clock, or `None` when nothing plays.
    pub fn animation_tick(&mut self, now_ms: u64) -> Option<u64> {
        let graphics = self.kitty_graphics()?;
        let mut next = 0;
        // SAFETY: a live storage handle; `&mut self` serializes the call.
        check(unsafe { sys::ghostty_kitty_graphics_animation_tick(graphics, now_ms, &mut next) })
            .ok()?;
        Some(next)
    }
}

/// Installs the PNG decoder the library calls for every terminal in the process.
pub(super) fn install_png_decoder() {
    // SAFETY: a callback of the type the option names.
    unsafe {
        sys::ghostty_sys_set(
            sys::GhosttySysOption::GHOSTTY_SYS_OPT_DECODE_PNG,
            decode_png as *const c_void,
        );
    }
}

/// Decodes into a buffer from the library's allocator, so its per-image cap holds; a
/// panic is caught and rejects the image (docs/research/ffi-callback-panics.md).
extern "C" fn decode_png(
    _: *mut c_void,
    allocator: *const sys::GhosttyAllocator,
    data: *const u8,
    len: usize,
    out: *mut sys::GhosttySysImage,
) -> bool {
    panic::catch_unwind(AssertUnwindSafe(|| {
        // SAFETY: the library passes `len` readable bytes for the call's duration.
        let png = unsafe { super::borrowed(data, len) };
        let Some((width, height, pixels, pixels_len)) = decode_into(png, allocator) else {
            return false;
        };
        // SAFETY: a writable out value for the call's duration; the library now owns the
        // buffer it allocated.
        unsafe {
            *out = sys::GhosttySysImage {
                width,
                height,
                data: pixels,
                data_len: pixels_len,
            };
        }
        true
    }))
    .unwrap_or(false)
}

/// The PNG as RGBA in a buffer `allocator` gave, or `None` with nothing left allocated.
fn decode_into(
    png: &[u8],
    allocator: *const sys::GhosttyAllocator,
) -> Option<(u32, u32, *mut u8, usize)> {
    let mut decoder = png::Decoder::new(std::io::Cursor::new(png));
    decoder.set_transformations(png::Transformations::normalize_to_color8());
    let mut reader = decoder.read_info().ok()?;
    let (width, height) = (reader.info().width, reader.info().height);
    let len = (width as usize)
        .checked_mul(height as usize)?
        .checked_mul(4)?;
    // Asked for first, so an image over the library's cap is refused before decoding.
    // SAFETY: the allocator the library passed for this call.
    let pixels = unsafe { sys::ghostty_alloc(allocator, len) };
    if pixels.is_null() {
        return None;
    }
    let free = || {
        // SAFETY: allocated above with this allocator and length, and not handed over.
        unsafe { sys::ghostty_free(allocator, pixels, len) }
    };
    let Some(mut frame) = reader
        .output_buffer_size()
        .filter(|size| *size <= len)
        .map(|size| vec![0; size])
    else {
        free();
        return None;
    };
    let decoded = reader.next_frame(&mut frame);
    let Ok(info) = decoded else {
        free();
        return None;
    };
    // SAFETY: `len` bytes allocated above, written once here.
    let target = unsafe { slice::from_raw_parts_mut(pixels, len) };
    let channels = match info.color_type {
        png::ColorType::Grayscale => 1,
        png::ColorType::GrayscaleAlpha => 2,
        png::ColorType::Rgb => 3,
        png::ColorType::Rgba => 4,
        // EXPAND turns an indexed image into RGB or RGBA.
        png::ColorType::Indexed => {
            free();
            return None;
        }
    };
    let rows = frame[..info.buffer_size()].chunks_exact(info.line_size);
    for (row, out) in rows.zip(target.chunks_exact_mut(width as usize * 4)) {
        for (pixel, out) in row.chunks_exact(channels).zip(out.as_chunks_mut::<4>().0) {
            *out = match *pixel {
                [gray] => [gray, gray, gray, 255],
                [gray, alpha] => [gray, gray, gray, alpha],
                [r, g, b] => [r, g, b, 255],
                [r, g, b, a] => [r, g, b, a],
                _ => unreachable!("chunks of one to four channels"),
            };
        }
    }
    Some((width, height, pixels, len))
}
