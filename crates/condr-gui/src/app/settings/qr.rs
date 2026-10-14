//! An Invite drawn as a QR code for the Companion to scan (ADR 0040).

use super::*;
use qrcode::{Color as Module, EcLevel, QrCode};
use std::cell::RefCell;

/// The light margin a scanner needs around the code, in modules (ISO/IEC 18004).
const QUIET_ZONE: usize = 4;

/// A code's modules, dark or light, row by row.
#[derive(Debug, PartialEq)]
pub(super) struct QrMatrix {
    width: usize,
    dark: Vec<bool>,
}

impl QrMatrix {
    pub(super) fn new(text: &str) -> Option<Self> {
        let code = QrCode::with_error_correction_level(text, EcLevel::M).ok()?;
        Some(Self {
            width: code.width(),
            dark: code
                .to_colors()
                .into_iter()
                .map(|module| module == Module::Dark)
                .collect(),
        })
    }

    /// Each row's runs of dark modules as (row, first column, length): one quad apiece.
    pub(super) fn runs(&self) -> impl Iterator<Item = (usize, usize, usize)> + '_ {
        self.dark
            .chunks(self.width)
            .enumerate()
            .flat_map(|(row, cells)| {
                let mut runs = Vec::new();
                let mut start = None;
                for (column, &dark) in cells.iter().chain([&false]).enumerate() {
                    match (start, dark) {
                        (None, true) => start = Some(column),
                        (Some(first), false) => {
                            runs.push((row, first, column - first));
                            start = None;
                        }
                        _ => {}
                    }
                }
                runs
            })
    }
}

/// The codes last drawn, so a Settings window redrawn for every change elsewhere does
/// not encode its invite again each time.
fn matrix(text: &str) -> Option<Rc<QrMatrix>> {
    thread_local! {
        static DRAWN: RefCell<Vec<(String, Rc<QrMatrix>)>> = const { RefCell::new(Vec::new()) };
    }
    DRAWN.with_borrow_mut(|drawn| {
        if let Some((_, matrix)) = drawn.iter().find(|(drawn, _)| drawn == text) {
            return Some(matrix.clone());
        }
        let matrix = Rc::new(QrMatrix::new(text)?);
        // A Peer-to-peer and a TCP link at a time, and the one being typed.
        if drawn.len() >= 4 {
            drawn.remove(0);
        }
        drawn.push((text.to_owned(), matrix.clone()));
        Some(matrix)
    })
}

/// `text` as a QR code on a white tile `side` across, dark on white in any theme. The
/// code fills the tile, quiet zone included, whatever its length, so the tile is one size
/// on every display; each module edge snaps to a device pixel, so a module is a pixel
/// wider or narrower rather than blurred, as a browser scales an SVG code.
pub(super) fn qr_code(id: &'static str, text: &str, side: Pixels) -> AnyElement {
    let Some(matrix) = matrix(text) else {
        return div().into_any_element();
    };
    let modules = (matrix.width + 2 * QUIET_ZONE) as f32;
    div()
        .debug_selector(move || id.into())
        .flex_none()
        .size(side)
        .rounded(px(8.))
        .bg(white())
        .child(
            canvas(
                |_, _, _| {},
                move |bounds, _, window, _| {
                    let scale = window.scale_factor();
                    let module = f32::from(side) * scale / modules;
                    let edge = |modules: usize| px((modules as f32 * module).round() / scale);
                    for (row, column, length) in matrix.runs() {
                        let left = edge(QUIET_ZONE + column);
                        let top = edge(QUIET_ZONE + row);
                        let right = edge(QUIET_ZONE + column + length);
                        let bottom = edge(QUIET_ZONE + row + 1);
                        window.paint_quad(fill(
                            Bounds::new(
                                bounds.origin + point(left, top),
                                size(right - left, bottom - top),
                            ),
                            black(),
                        ));
                    }
                },
            )
            .size_full(),
        )
        .into_any_element()
}

#[cfg(test)]
mod tests {
    // Not the parent's glob: it brings GPUI's `test` attribute, which shadows the standard one.
    use super::{QrMatrix, matrix};
    use std::rc::Rc;

    #[test]
    fn an_invite_encodes_with_its_finder_patterns_and_runs_cover_every_dark_module() {
        let link = "p2p://IIp97X9_1QRy3J8JfD6mfOZijNFqvidGI1z9n2Lsawc.9XInogWZdS-bBkNIWwk0TxogXk3jrojXhvwhyqll9dw";
        let matrix = QrMatrix::new(link).unwrap();
        assert_eq!((matrix.width - 21) % 4, 0, "a QR version's side is 21 + 4n");
        assert_eq!(matrix.dark.len(), matrix.width * matrix.width);
        // The top-left finder: a dark 7-module ring around a light one, a dark 3×3 core.
        let at = |row: usize, column: usize| matrix.dark[row * matrix.width + column];
        assert!((0..7).all(|i| at(0, i) && at(6, i) && at(i, 0) && at(i, 6)));
        assert!((1..6).all(|i| !at(1, i) && !at(5, i)));
        assert!((2..5).all(|row| (2..5).all(|column| at(row, column))));

        let mut painted = vec![false; matrix.dark.len()];
        for (row, column, length) in matrix.runs() {
            for column in column..column + length {
                assert!(!painted[row * matrix.width + column], "runs do not overlap");
                painted[row * matrix.width + column] = true;
            }
        }
        assert_eq!(painted, matrix.dark);
    }

    #[test]
    fn the_same_link_is_encoded_once() {
        let first = matrix("tcp://key.secret@192.168.1.20:7418").unwrap();
        let again = matrix("tcp://key.secret@192.168.1.20:7418").unwrap();
        assert!(Rc::ptr_eq(&first, &again));
    }
}
