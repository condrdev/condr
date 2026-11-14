//! A kitty graphics image's pixels as the Server holds them (ADR 0065): named by their
//! SHA-256, and sent to a Client that asks as PNG.

use super::TerminalImageKey;
use sha2::{Digest, Sha256};

/// One image a Pane's VT showed, copied out of it: the VT may change or evict it at its
/// next write.
pub struct TerminalImage {
    pub key: TerminalImageKey,
    pub width: u32,
    pub height: u32,
    /// One byte each: gray, gray and alpha, RGB or RGBA.
    channels: u8,
    pixels: Vec<u8>,
}

impl TerminalImage {
    /// `channels` bytes per pixel: 1 gray, 2 gray and alpha, 3 RGB, 4 RGBA.
    pub fn new(width: u32, height: u32, channels: u8, pixels: Vec<u8>) -> Self {
        let mut hash = Sha256::new();
        hash.update(width.to_le_bytes());
        hash.update(height.to_le_bytes());
        hash.update([channels]);
        hash.update(&pixels);
        Self {
            key: TerminalImageKey(hash.finalize().into()),
            width,
            height,
            channels,
            pixels,
        }
    }

    /// The bytes the pixels take.
    pub fn len(&self) -> usize {
        self.pixels.len()
    }

    pub fn is_empty(&self) -> bool {
        self.pixels.is_empty()
    }

    /// The image as PNG at the fast level, in its own channels: every Client decodes PNG
    /// natively, and lossless is what a program drew.
    pub fn encode_png(&self) -> Option<Vec<u8>> {
        let mut png = Vec::new();
        let mut encoder = png::Encoder::new(&mut png, self.width, self.height);
        encoder.set_color(match self.channels {
            1 => png::ColorType::Grayscale,
            2 => png::ColorType::GrayscaleAlpha,
            3 => png::ColorType::Rgb,
            _ => png::ColorType::Rgba,
        });
        encoder.set_compression(png::Compression::Fast);
        let mut writer = encoder.write_header().ok()?;
        writer.write_image_data(&self.pixels).ok()?;
        writer.finish().ok()?;
        Some(png)
    }
}
