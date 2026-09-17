//! Run `cargo run -p condr-gui --example generate_icons` after editing the SVG.
//! Generated assets are committed; normal builds need no image conversion tools.

use image::ExtendedColorType;
use image::codecs::ico::{IcoEncoder, IcoFrame};
use resvg::{tiny_skia, usvg};
use std::{collections::BTreeMap, error::Error, fs, path::Path};

const LOGO_VIEWBOX: &str = "0 0 128 128";
const PLATFORM_CORNER_RADIUS: &str = "16";

fn rounded_platform_svg(source: &str) -> String {
    let body_start = source
        .find('>')
        .expect("condr.svg must have an opening svg element")
        + 1;
    let body_end = source
        .rfind("</svg>")
        .expect("condr.svg must have a closing svg element");
    format!(
        r#"<svg xmlns="http://www.w3.org/2000/svg" viewBox="{LOGO_VIEWBOX}">
<defs><clipPath id="platform-corners"><rect width="128" height="128" rx="{PLATFORM_CORNER_RADIUS}"/></clipPath></defs>
<g clip-path="url(#platform-corners)">{}</g>
</svg>"#,
        &source[body_start..body_end]
    )
}

fn main() -> Result<(), Box<dyn Error>> {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../assets/brand");
    let source = fs::read_to_string(root.join("condr.svg"))?;
    let source_tree = usvg::Tree::from_str(&source, &Default::default())?;
    // The checked-in SVG is the platform-neutral source. Desktop application icon formats
    // conventionally mask their corners, so rasterize a clipped wrapper for macOS while leaving
    // the source SVG itself untouched for Linux packaging. PNG and ICO are also used by the GUI
    // title bar, X11 and Windows Shell, where the unmasked source is the appropriate
    // representation.
    let platform_tree = usvg::Tree::from_str(&rounded_platform_svg(&source), &Default::default())?;
    let mut images = BTreeMap::new();
    for size in [16, 32, 48, 64, 128, 256, 512, 1024] {
        let mut pixmap = tiny_skia::Pixmap::new(size, size).ok_or("allocate icon pixels")?;
        resvg::render(
            &source_tree,
            tiny_skia::Transform::from_scale(
                size as f32 / source_tree.size().width(),
                size as f32 / source_tree.size().height(),
            ),
            &mut pixmap.as_mut(),
        );
        images.insert(size, pixmap.encode_png()?);
    }
    fs::write(root.join("condr.png"), &images[&256])?;

    let mut platform_images = BTreeMap::new();
    for size in [16, 32, 48, 64, 128, 256, 512, 1024] {
        let mut pixmap = tiny_skia::Pixmap::new(size, size).ok_or("allocate icon pixels")?;
        resvg::render(
            &platform_tree,
            tiny_skia::Transform::from_scale(
                size as f32 / platform_tree.size().width(),
                size as f32 / platform_tree.size().height(),
            ),
            &mut pixmap.as_mut(),
        );
        platform_images.insert(size, pixmap.encode_png()?);
    }

    let frames = [16, 32, 48, 64, 128, 256]
        .into_iter()
        .map(|size| {
            IcoFrame::with_encoded(
                images[&size].as_slice(),
                size,
                size,
                ExtendedColorType::Rgba8,
            )
        })
        .collect::<Result<Vec<_>, _>>()?;
    IcoEncoder::new(fs::File::create(root.join("condr.ico"))?).encode_images(&frames)?;

    // iconutil consumes this standard 1x/2x iconset when packaging on macOS.
    let iconset = root.join("condr.iconset");
    fs::create_dir_all(&iconset)?;
    for size in [16, 32, 128, 256, 512] {
        fs::write(
            iconset.join(format!("icon_{size}x{size}.png")),
            &platform_images[&size],
        )?;
        fs::write(
            iconset.join(format!("icon_{size}x{size}@2x.png")),
            &platform_images[&(size * 2)],
        )?;
    }
    println!("Generated PNG, ICO and macOS iconset in {}", root.display());
    Ok(())
}
