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
fn a_pane_says_it_runs_ghosttys_core_and_answers_the_graphics_query() {
    let mut vt = Vt::new(size()).unwrap();
    let replies: Vec<u8> = vt
        .write(b"\x1b_Gi=31,s=1,v=1,a=q,t=d,f=24;AAAA\x1b\\\x1b[>0q\x1b[c")
        .into_iter()
        .filter_map(|event| match event {
            condr_ghostty_vt::Event::WritePty(bytes) => Some(bytes),
            _ => None,
        })
        .flatten()
        .collect();
    let replies = String::from_utf8_lossy(&replies);
    let query = replies
        .find("\x1b_Gi=31;OK\x1b\\")
        .expect("the query is answered");
    let name = replies
        .find(&format!(
            "\x1bP>|ghostty (Condr {})",
            env!("CARGO_PKG_VERSION")
        ))
        .expect("XTVERSION names Ghostty's core");
    let da1 = replies.find("\x1b[?62;22c").expect("DA1 last");
    assert!(query < name && name < da1, "{replies:?}");
}

#[test]
fn images_stay_off_without_storage() {
    let mut vt = Vt::with_image_storage(size(), 0).unwrap();
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

/// With no output from the program, an animation's next frame still makes a frame of its
/// own, and a looping animation names the same images again.
#[test]
fn an_animation_makes_frames_without_output_and_loops_through_the_same_keys() {
    let size = size();
    let mut vt = Vt::with_image_storage(size, STORAGE).unwrap();
    transmit(
        &mut vt,
        "a=T,f=32,s=8,v=8,q=2,i=20",
        &[255, 40, 40, 255].repeat(64),
    );
    transmit(
        &mut vt,
        "a=f,f=32,s=8,v=8,q=2,i=20,z=60",
        &[40, 200, 80, 255].repeat(64),
    );
    // The root frame's gap, then play, looping.
    vt.write(b"\x1b_Ga=a,q=2,i=20,r=1,z=60\x1b\\\x1b_Ga=a,q=2,i=20,s=3,v=1\x1b\\");
    let source = TerminalViewSource {
        terminal: Arc::new(Mutex::new(vt)),
        size: Arc::new(Mutex::new(size)),
        revision: Arc::new(AtomicU64::new(1)),
        damage_baseline: Arc::new(Mutex::new(None)),
        cursor_settle: Arc::new(Mutex::new(super::cursor_settle::CursorSettle::default())),
    };
    let mut shown = Vec::new();
    let deadline = Instant::now() + Duration::from_secs(5);
    while shown.len() < 4 && Instant::now() < deadline {
        source.tick_animations();
        if let Some(frame) = source.take_frame() {
            let images = match frame {
                TerminalViewFrame::Full(view) => Some(view.images),
                TerminalViewFrame::Delta(delta) => delta.images,
            };
            shown.extend(images.and_then(|images| images.first().map(|image| image.image)));
        }
        std::thread::sleep(
            source
                .animation_wakeup()
                .unwrap_or(Duration::from_millis(10)),
        );
    }
    assert_eq!(shown.len(), 4, "a frame per animation frame: {shown:?}");
    assert_ne!(shown[0], shown[1]);
    assert_eq!(
        (shown[0], shown[1]),
        (shown[2], shown[3]),
        "the loop names the same images"
    );
}
