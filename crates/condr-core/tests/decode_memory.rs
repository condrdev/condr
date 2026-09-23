//! What a peer can make this side allocate by decoding one frame or record (ADR 0028).
//! prost materializes a `repeated` field before the domain conversion counts it, so the
//! size limits, not the domain counts, bound memory: decoding peaks at no more than
//! [`MAX_DECODE_MULTIPLE`] times the limit the input was held to.

#![allow(
    unsafe_code,
    reason = "a counting global allocator measures decode peaks"
)]

use std::alloc::{GlobalAlloc, Layout, System};
use std::sync::atomic::{AtomicUsize, Ordering};

use condr_core::protocol::{
    MAX_CHUNKED_RECORD_SIZE, MAX_FRAME_SIZE, ServerMessage, decode_bootstrap_record, read_message,
};

/// The stated bound: peak decode memory over the input's own limit.
/// Measured: 31x for a frame of empty metadata, 91x for a Snapshot of empty Workspaces,
/// 24x for a record of empty hyperlink strings.
const MAX_DECODE_MULTIPLE: usize = 96;

struct Counting;

static CURRENT: AtomicUsize = AtomicUsize::new(0);
static PEAK: AtomicUsize = AtomicUsize::new(0);

unsafe impl GlobalAlloc for Counting {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        let pointer = unsafe { System.alloc(layout) };
        if !pointer.is_null() {
            let now = CURRENT.fetch_add(layout.size(), Ordering::Relaxed) + layout.size();
            PEAK.fetch_max(now, Ordering::Relaxed);
        }
        pointer
    }

    unsafe fn dealloc(&self, pointer: *mut u8, layout: Layout) {
        unsafe { System.dealloc(pointer, layout) };
        CURRENT.fetch_sub(layout.size(), Ordering::Relaxed);
    }
}

#[global_allocator]
static ALLOCATOR: Counting = Counting;

/// Bytes allocated at the peak of `run` above what was live before it.
fn peak_of(run: impl FnOnce()) -> usize {
    let before = CURRENT.load(Ordering::Relaxed);
    PEAK.store(before, Ordering::Relaxed);
    run();
    PEAK.load(Ordering::Relaxed) - before
}

fn varint(mut value: usize) -> Vec<u8> {
    let mut bytes = Vec::new();
    while value >= 0x80 {
        bytes.push(value as u8 | 0x80);
        value >>= 7;
    }
    bytes.push(value as u8);
    bytes
}

/// `tag` as a length-delimited field around `body`.
fn field(tag: u8, body: &[u8]) -> Vec<u8> {
    let mut bytes = vec![tag];
    bytes.extend(varint(body.len()));
    bytes.extend_from_slice(body);
    bytes
}

/// A body of `limit` bytes, less `room`, of empty two-byte `tag` elements: the most
/// elements a `repeated` message field can hold per byte.
fn empty_elements(tag: u8, limit: usize, room: usize) -> Vec<u8> {
    [tag, 0].repeat((limit - room) / 2)
}

// One test, so no other test's allocations land between the counters.
#[test]
fn adversarial_input_decodes_within_the_stated_multiple_of_its_limit() {
    // ServerMessage.overview_terminals full of empty PaneTerminalMetadata.
    let inner = empty_elements(0x1a, MAX_FRAME_SIZE, 16);
    let body = field(0x1a, &inner);
    let mut frame = varint(body.len());
    frame.extend(&body);
    let peak = peak_of(|| {
        let message: ServerMessage = read_message(&mut frame.as_slice()).unwrap();
        drop(message);
    });
    assert!(
        peak <= MAX_DECODE_MULTIPLE * MAX_FRAME_SIZE,
        "a frame of empty repeated metadata peaks at {peak} bytes"
    );
    println!("metadata: {:.1}x", peak as f64 / MAX_FRAME_SIZE as f64);

    // ServerMessage.event carrying a LayoutChanged Snapshot of empty Workspaces: refused
    // for their count, but only after prost built them.
    let workspaces = empty_elements(0x0a, MAX_FRAME_SIZE, 32);
    let session_event = field(0x0a, &field(0x0a, &workspaces));
    let event = field(0x22, &session_event);
    let body = field(0x42, &event);
    let mut frame = varint(body.len());
    frame.extend(&body);
    let peak = peak_of(|| {
        let _ = read_message::<_, ServerMessage>(&mut frame.as_slice());
    });
    assert!(
        peak <= MAX_DECODE_MULTIPLE * MAX_FRAME_SIZE,
        "a Snapshot of empty Workspaces peaks at {peak} bytes"
    );
    println!("workspaces: {:.1}x", peak as f64 / MAX_FRAME_SIZE as f64);

    // A Terminal record whose hyperlink table is empty strings, the cheapest element that
    // still costs a String each.
    let hyperlinks = empty_elements(0x2a, MAX_CHUNKED_RECORD_SIZE, 64);
    let record = field(0x0a, &field(0x12, &hyperlinks));
    let peak = peak_of(|| {
        let _ = decode_bootstrap_record(&record);
    });
    assert!(
        peak <= MAX_DECODE_MULTIPLE * MAX_CHUNKED_RECORD_SIZE,
        "a record of empty hyperlinks peaks at {peak} bytes"
    );
    println!(
        "hyperlinks: {:.1}x",
        peak as f64 / MAX_CHUNKED_RECORD_SIZE as f64
    );
}
