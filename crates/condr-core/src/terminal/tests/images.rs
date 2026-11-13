//! Kitty graphics in the VT and the frames it makes (ADR 0065).

use super::*;
use base64::Engine as _;

const STORAGE: u64 = 1 << 20;

fn size() -> TerminalSize {
    TerminalSize::new(4, 10).with_cell_size(8, 16)
}

fn transmit(vt: &mut Vt, control: &str, pixels: &[u8]) {
    let payload = base64::engine::general_purpose::STANDARD.encode(pixels);
    vt.write(format!("\x1b_G{control};{payload}\x1b\\").as_bytes());
}

#[test]
fn a_placed_image_reaches_the_view_named_by_its_pixels() {
    let mut vt = Vt::with_image_storage(size(), STORAGE).unwrap();
    vt.write(b"\r\n ");
    transmit(
        &mut vt,
        "a=T,q=2,f=32,s=2,v=1,i=5",
        &[1, 2, 3, 255, 4, 5, 6, 255],
    );
    let view = vt.view(size(), 1, None);
    let [placement] = view.images.as_slice() else {
        panic!("{:?}", view.images)
    };
    assert_eq!((placement.row, placement.column), (1, 1));
    assert_eq!((placement.image_width, placement.image_height), (2, 1));
    assert_eq!((placement.width, placement.height), (2, 1));
    assert_eq!(placement.layer, TerminalImageLayer::AboveText);

    let images = vt.take_images();
    let [image] = images.as_slice() else {
        panic!("one image first shown")
    };
    assert_eq!(image.key, placement.image);
    assert!(vt.take_images().is_empty(), "taken once");
    vt.view(size(), 2, None);
    assert!(vt.take_images().is_empty(), "a generation is copied once");

    // The same pixels under another id are the same image to every cache.
    transmit(
        &mut vt,
        "a=T,q=2,f=32,s=2,v=1,i=6",
        &[1, 2, 3, 255, 4, 5, 6, 255],
    );
    let view = vt.view(size(), 3, None);
    assert_eq!(view.images.len(), 2);
    assert!(
        view.images
            .iter()
            .all(|image| image.image == placement.image)
    );
}

#[test]
fn images_stay_off_without_storage() {
    let mut vt = Vt::new(size()).unwrap();
    transmit(&mut vt, "a=T,q=2,f=32,s=1,v=1,i=5", &[1, 2, 3, 255]);
    assert!(vt.view(size(), 1, None).images.is_empty());
    assert!(vt.take_images().is_empty());
}

#[test]
fn placeholder_cells_leave_blank_and_their_image_is_placed() {
    let mut vt = Vt::with_image_storage(size(), STORAGE).unwrap();
    transmit(&mut vt, "a=T,U=1,q=2,f=32,s=2,v=1,i=1,c=2,r=1", &[255; 8]);
    vt.write("\x1b[38;5;1mab\u{10EEEE}\u{305}\u{305}\u{10EEEE}".as_bytes());
    let view = vt.view(size(), 1, None);
    let text: Vec<&str> = (0..4)
        .map(|column| view.cell(0, column).unwrap().text.as_str())
        .collect();
    assert_eq!(text, ["a", "b", " ", " "]);
    let [placement] = view.images.as_slice() else {
        panic!("{:?}", view.images)
    };
    assert_eq!((placement.row, placement.column), (0, 2));
}

#[test]
fn a_frame_carries_the_images_only_when_they_changed() {
    let mut vt = Vt::with_image_storage(size(), STORAGE).unwrap();
    let first = vt.view(size(), 1, None);
    transmit(&mut vt, "a=T,q=2,f=32,s=1,v=1,i=5", &[1, 2, 3, 255]);
    let second = vt.view(size(), 2, Some(&first));
    let Some(TerminalViewFrame::Delta(delta)) = TerminalView::frame_from(Some(&first), &second)
    else {
        panic!("a delta")
    };
    assert_eq!(delta.images.as_deref(), Some(second.images.as_slice()));

    vt.write(b"x");
    let third = vt.view(size(), 3, Some(&second));
    let Some(TerminalViewFrame::Delta(delta)) = TerminalView::frame_from(Some(&second), &third)
    else {
        panic!("a delta")
    };
    assert_eq!(delta.images, None, "text alone leaves the images out");
    let mut applied = second.clone();
    applied
        .apply_frame(TerminalViewFrame::Delta(delta))
        .unwrap();
    assert_eq!(applied.images, third.images);
}

#[test]
fn placements_round_trip_on_the_wire() {
    let mut vt = Vt::with_image_storage(size(), STORAGE).unwrap();
    vt.write(b"\r\n\r\n\r\n\r\n");
    transmit(&mut vt, "a=T,q=2,f=32,s=2,v=40,i=5,z=-1", &[9; 2 * 40 * 4]);
    let view = vt.view(size(), 1, None);
    assert_eq!(view.images[0].layer, TerminalImageLayer::BelowText);
    assert_eq!(view.images[0].z, -1);
    let wire = crate::protocol::pb::TerminalView::from(&view);
    assert_eq!(TerminalView::try_from(wire).unwrap(), view);
}

#[test]
fn an_animation_turns_its_frames_when_ticked() {
    let mut vt = Vt::with_image_storage(size(), STORAGE).unwrap();
    transmit(&mut vt, "a=T,q=2,f=32,s=1,v=1,i=9", &[255, 0, 0, 255]);
    transmit(&mut vt, "a=f,q=2,f=32,s=1,v=1,i=9,z=20", &[0, 255, 0, 255]);
    vt.write(b"\x1b_Ga=a,q=2,i=9,s=3,v=1\x1b\\");
    let first = vt.view(size(), 1, None);
    vt.take_images();
    assert!(vt.tick_animations().is_some(), "the animation plays");
    std::thread::sleep(Duration::from_millis(60));
    vt.tick_animations();
    let second = vt.view(size(), 2, Some(&first));
    assert_ne!(
        first.images[0].image, second.images[0].image,
        "the next frame is another image"
    );
    assert_eq!(vt.take_images().len(), 1);
}
