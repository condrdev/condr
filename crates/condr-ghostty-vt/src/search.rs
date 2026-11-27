//! The terminal's search (ADR 0077): libghostty-vt's, over the active area and the
//! scrollback of both screens, kept on the terminal so every call is serialized with it.

use super::{Error, Terminal, check, sys};
use std::ffi::c_void;
use std::mem::{MaybeUninit, size_of};
use std::ptr;
use std::time::{Duration, Instant};

/// Where a search stands after a step.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum SearchStatus {
    /// More to do on what it has read.
    Running,
    /// Waiting to read more of the terminal.
    FeedRequired,
    /// Caught up with the terminal as of its last read.
    Complete,
}

/// A match's first and last cell in the viewport as `(column, row)`, clipped to it.
pub type ViewportMatch = ((u16, u32), (u16, u32));

impl Terminal {
    /// Searches for `needle`, keeping what was found when it did not change and starting
    /// again when it did; an empty one ends the search.
    pub fn set_search(&mut self, needle: &str) -> Result<(), Error> {
        if needle.is_empty() {
            self.end_search();
            return Ok(());
        }
        if self.search.is_null() {
            let mut search = ptr::null_mut();
            // SAFETY: a live terminal, `&mut self` serializes the call, and `search` is read
            // only once it succeeded; `Drop` frees it before the terminal.
            check(unsafe { sys::ghostty_search_new(ptr::null(), &mut search, self.raw) })?;
            self.search = search;
        }
        let needle = sys::GhosttyString {
            ptr: needle.as_ptr(),
            len: needle.len(),
        };
        // SAFETY: a live search on this terminal; the library copies the bytes.
        check(unsafe {
            sys::ghostty_search_set(
                self.search,
                sys::GhosttySearchOption::GHOSTTY_SEARCH_OPT_NEEDLE,
                (&raw const needle).cast(),
            )
        })
    }

    pub(crate) fn end_search(&mut self) {
        if !self.search.is_null() {
            // SAFETY: created by `set_search` on this live terminal and not used again.
            unsafe { sys::ghostty_search_free(self.search) };
            self.search = ptr::null_mut();
        }
    }

    pub fn searching(&self) -> bool {
        !self.search.is_null()
    }

    /// Selects the next match toward older output or newer, wrapping at the ends, and
    /// scrolls the viewport to it when it is off screen. `false` when there is none.
    pub fn select_search_match(&mut self, older: bool) -> bool {
        if self.search.is_null() {
            return false;
        }
        let option = if older {
            sys::GhosttySearchOption::GHOSTTY_SEARCH_OPT_SELECT_NEXT
        } else {
            sys::GhosttySearchOption::GHOSTTY_SEARCH_OPT_SELECT_PREV
        };
        // SAFETY: a live search; `&mut self` serializes the terminal read it makes.
        check(unsafe { sys::ghostty_search_set(self.search, option, ptr::null()) }).is_ok()
    }

    /// Reads the terminal once, a bounded amount, then works on what it read until it
    /// needs to read more or `budget` is spent. `None` without a search.
    pub fn advance_search(&mut self, budget: Duration) -> Option<SearchStatus> {
        if self.search.is_null() {
            return None;
        }
        let start = Instant::now();
        // SAFETY: a live search; `&mut self` serializes the terminal read.
        check(unsafe { sys::ghostty_search_feed(self.search) }).ok()?;
        loop {
            let mut status = sys::GhosttySearchStatus::GHOSTTY_SEARCH_STATUS_COMPLETE;
            // SAFETY: a live search and a writable out pointer.
            check(unsafe { sys::ghostty_search_tick(self.search, &mut status) }).ok()?;
            let status = match status {
                sys::GhosttySearchStatus::GHOSTTY_SEARCH_STATUS_RUNNING => SearchStatus::Running,
                sys::GhosttySearchStatus::GHOSTTY_SEARCH_STATUS_FEED_REQUIRED => {
                    SearchStatus::FeedRequired
                }
                _ => SearchStatus::Complete,
            };
            if status != SearchStatus::Running || start.elapsed() >= budget {
                return Some(status);
            }
        }
    }

    /// Matches found so far on the screen shown.
    pub fn search_total(&self) -> usize {
        self.search_value(sys::GhosttySearchData::GHOSTTY_SEARCH_DATA_TOTAL_MATCHES)
            .unwrap_or(0)
    }

    /// The selected match's place, counted from the newest, which is 0.
    pub fn search_selected(&self) -> Option<usize> {
        self.search_value(sys::GhosttySearchData::GHOSTTY_SEARCH_DATA_SELECTED_INDEX)
    }

    /// The matches on the viewport as of the last read, each clipped to it.
    pub fn search_viewport_matches(&self) -> Vec<ViewportMatch> {
        if self.search.is_null() {
            return Vec::new();
        }
        let data = sys::GhosttySearchData::GHOSTTY_SEARCH_DATA_VIEWPORT_MATCHES;
        let mut buffer = sys::GhosttySelectionBuffer {
            ptr: ptr::null_mut(),
            cap: 0,
            len: 0,
        };
        // SAFETY: a live search; a null buffer only asks for the length.
        let result = unsafe {
            sys::ghostty_search_get(self.search, data, (&raw mut buffer).cast::<c_void>())
        };
        if result == sys::GhosttyResult::GHOSTTY_SUCCESS || buffer.len == 0 {
            return Vec::new();
        }
        let mut selections = vec![empty_selection(); buffer.len];
        buffer.ptr = selections.as_mut_ptr();
        buffer.cap = selections.len();
        // SAFETY: a live search and a buffer of `cap` sized selections it fills.
        check(unsafe {
            sys::ghostty_search_get(self.search, data, (&raw mut buffer).cast::<c_void>())
        })
        .map(|()| {
            selections[..buffer.len]
                .iter()
                .filter_map(|selection| self.clip_to_viewport(selection))
                .collect()
        })
        .unwrap_or_default()
    }

    /// The selected match, when it is on the viewport.
    pub fn search_selected_match(&self) -> Option<ViewportMatch> {
        if self.search.is_null() {
            return None;
        }
        let mut selection = empty_selection();
        // SAFETY: a live search and a sized selection it fills.
        check(unsafe {
            sys::ghostty_search_get(
                self.search,
                sys::GhosttySearchData::GHOSTTY_SEARCH_DATA_SELECTED_MATCH,
                (&raw mut selection).cast::<c_void>(),
            )
        })
        .ok()?;
        self.clip_to_viewport(&selection)
    }

    fn search_value(&self, data: sys::GhosttySearchData) -> Option<usize> {
        if self.search.is_null() {
            return None;
        }
        let mut value = 0usize;
        // SAFETY: a live search; both fields read are `size_t`.
        check(unsafe {
            sys::ghostty_search_get(self.search, data, (&raw mut value).cast::<c_void>())
        })
        .ok()?;
        Some(value)
    }

    /// A match in viewport cells, its ends in order; one end off screen is clipped to the
    /// viewport's edge, and a match wholly off it is `None`.
    fn clip_to_viewport(&self, selection: &sys::GhosttySelection) -> Option<ViewportMatch> {
        let point = |grid_ref: &sys::GhosttyGridRef| {
            let mut out = sys::GhosttyPointCoordinate { x: 0, y: 0 };
            // SAFETY: a live terminal, a grid reference it produced since its last change
            // (`&self` keeps it unchanged), and a writable out pointer.
            check(unsafe {
                sys::ghostty_terminal_point_from_grid_ref(
                    self.raw,
                    grid_ref,
                    sys::GhosttyPointTag::GHOSTTY_POINT_TAG_VIEWPORT,
                    &mut out,
                )
            })
            .ok()
            .map(|()| (out.x, out.y))
        };
        let last = (
            self.cols().saturating_sub(1),
            u32::from(self.rows().saturating_sub(1)),
        );
        let (start, end) = match (point(&selection.start), point(&selection.end)) {
            (Some(start), Some(end)) => (start, end),
            (Some(start), None) => (start, last),
            (None, Some(end)) => ((0, 0), end),
            (None, None) => return None,
        };
        let key = |(x, y): (u16, u32)| (y, x);
        Some(if key(start) <= key(end) {
            (start, end)
        } else {
            (end, start)
        })
    }
}

fn empty_selection() -> sys::GhosttySelection {
    // SAFETY: a plain C struct for the library to fill; zero is a valid bit pattern.
    let mut selection = unsafe { MaybeUninit::<sys::GhosttySelection>::zeroed().assume_init() };
    selection.size = size_of::<sys::GhosttySelection>();
    selection.start.size = size_of::<sys::GhosttyGridRef>();
    selection.end.size = size_of::<sys::GhosttyGridRef>();
    selection
}
