//! Run `cargo run -p condr-gui --example generate_icons` after editing the SVG.
//! Generated assets are committed; normal builds need no image conversion tools.

use image::ExtendedColorType;
use image::codecs::ico::{IcoEncoder, IcoFrame};
use resvg::{tiny_skia, usvg};
use std::{collections::BTreeMap, error::Error, fs, path::Path};

fn main() -> Result<(), Box<dyn Error>> {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../packaging/icons");
    let tree = usvg::Tree::from_data(&fs::read(root.join("condr.svg"))?, &Default::default())?;
    let mut images = BTreeMap::new();
    for size in [16, 32, 48, 64, 128, 256, 512, 1024] {
        let mut pixmap = tiny_skia::Pixmap::new(size, size).ok_or("allocate icon pixels")?;
        resvg::render(
            &tree,
            tiny_skia::Transform::from_scale(
                size as f32 / tree.size().width(),
                size as f32 / tree.size().height(),
            ),
            &mut pixmap.as_mut(),
        );
        images.insert(size, pixmap.encode_png()?);
    }
    fs::write(root.join("condr.png"), &images[&256])?;

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
            &images[&size],
        )?;
        fs::write(
            iconset.join(format!("icon_{size}x{size}@2x.png")),
            &images[&(size * 2)],
        )?;
    }
    println!("Generated PNG, ICO and macOS iconset in {}", root.display());
    Ok(())
}
